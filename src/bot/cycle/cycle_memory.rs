/// Fetch approved rooms in adapter-native position order. Pages remain normalized;
/// denied/unsupported history is not a successful empty scan.
async fn read_history(bot: &Bot, connection: &ConnectionId, space: &SpaceRef) -> bool {
    let adapter = {
        bot.adapters.lock().unwrap_or_else(|e| e.into_inner()).get(connection)
    };
    let Some(adapter) = adapter else {
        return false;
    };
    let cutoff = history_cutoff(now_unix());
    let rooms: Vec<_> = {
        let registry = bot.adapters.lock().unwrap_or_else(|e| e.into_inner());
        space.rooms.iter()
            .filter(|room| registry.allows(connection, room) && registry.is_bound(connection, room))
            .cloned()
            .collect()
    };
    let mut collected = Vec::new();
    let mut succeeded = !rooms.is_empty();
    for room in rooms {
        let mut cursor = None;
        loop {
            let page = match adapter.fetch_history(&room, platform::outgoing::HistoryRequest {
                cursor: cursor.clone(),
                cutoff_unix_ms: Some(cutoff),
                limit: 100,
            }).await {
                Ok(page) => page,
                Err(error) => {
                    log::warn!("history [{}]: {error}", room.conversation());
                    succeeded = false;
                    break;
                }
            };
            if !page.messages.iter().all(|message|
                valid_message_source(message, connection, room.id.platform())
                    && message.room.id == room.id && message.room.thread == room.thread
            ) {
                log::warn!("history [{}]: invalid page source identity", room.conversation());
                succeeded = false;
                break;
            }
            if page.limitations.iter().any(|limitation| matches!(
                limitation,
                platform::outgoing::HistoryLimitation::Restricted
                    | platform::outgoing::HistoryLimitation::Undecryptable
            )) {
                log::warn!("history [{}]: access limitations {:?}", room.conversation(), page.limitations);
                succeeded = false;
            }
            let next = page.next_cursor.clone();
            let hit_cutoff = collect_history_page(&mut collected, page, cutoff);
            if hit_cutoff || next.is_none() {
                break;
            }
            if next == cursor {
                log::warn!("history [{}]: repeated history cursor", room.conversation());
                succeeded = false;
                break;
            }
            cursor = next;
        }
    }
    let count = collected.len();
    apply_history(&mut bot.state(), collected);
    log::debug!("{}: read {count} message(s)", space.id);
    succeeded
}

fn history_cutoff(now: i64) -> i64 {
    (now - HISTORY_DAYS * 24 * 60 * 60) * 1000
}

fn collect_history_page(
    collected: &mut Vec<IncomingMessage>,
    page: platform::outgoing::HistoryPage,
    cutoff: i64,
) -> bool {
    let newest_first = page.order == platform::outgoing::HistoryOrder::NewestFirst;
    let mut hit_cutoff = false;
    for message in page.messages {
        if message.sent_at_unix_ms < cutoff {
            hit_cutoff = true;
            if newest_first {
                break;
            }
        } else {
            collected.push(message);
        }
    }
    // Oldest-first pages may still have newer pages after crossing the cutoff.
    hit_cutoff && newest_first
}

fn apply_history(state: &mut State, mut collected: Vec<IncomingMessage>) {
    collected.sort_by_key(|message| message.sent_at_unix_ms);
    let skip = collected.len().saturating_sub(MEMORY_SIZE);
    let mut lines = Vec::with_capacity(collected.len().saturating_sub(skip));
    for message in collected.into_iter().skip(skip) {
        if is_owner(&message.author.id) {
            state.favorite_name = Some(message.author.display_name.clone());
        }
        let live = state.recent_users.iter()
            .find(|(_, person)| person.id == message.author.id)
            .map(|(label, person)| (label.clone(), person.clone()));
        let username = state.usernames.get(&message.author.id).cloned();
        let name = observe_user(state, &message.author);
        // Live identity metadata takes priority over stale historical names.
        if let Some((label, person)) = live {
            state.recent_users.insert(label, person);
        }
        if let Some(username) = username {
            state.usernames.insert(message.author.id.clone(), username);
        }
        lines.push(format!("{name}: {}", message.text));
    }
    // Live messages received during the scan stay at the chronological end.
    for line in lines.into_iter().rev() {
        state.recent_messages.push_front(line);
    }
    while state.recent_messages.len() > MEMORY_SIZE {
        state.recent_messages.pop_front();
    }
}

/// Explicit news targets precede discovered preferred rooms; otherwise use the
/// native system-room then text-position order within each discovered space.
fn default_channel(bot: &Bot) -> Option<ConversationKey> {
    let registry = bot.adapters.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(room) = &bot.news_channel {
        let route = ConversationKey { room: room.clone(), thread: None };
        return registry.for_conversation(&route).map(|_| route);
    }
    for (connection, space) in registry.discovered_spaces() {
        if registry.get(&connection).is_none() {
            continue;
        }
        if let Some(route) = preferred_route(&space, |room|
            registry.allows(&connection, room) && registry.is_bound(&connection, room)
        ) {
            return Some(route);
        }
    }
    registry.first_approved_room()
}

fn preferred_route(space: &SpaceRef, approved: impl Fn(&RoomRef) -> bool)
    -> Option<ConversationKey>
{
    if let Some(preferred) = &space.preferred_room {
        let room = RoomRef {
            space: Some(space.id.clone()),
            ..RoomRef::from_key(preferred.clone())
        };
        if approved(&room) {
            return Some(room.conversation());
        }
    }
    space.rooms.iter().find(|room| approved(room)).map(RoomRef::conversation)
}

// ---------- background cycles ----------

// keeps a cycle alive: on a panic or an unexpected return, logs it and restarts after 5s
// (the panic hook writes the backtrace; this watchdog prevents a silent death; the 5s
// wait is on both branches so a cycle that returns immediately after starting can't burn CPU).
/// `build` must produce a fresh cycle future on each restart.
// Never restarts once the shutdown signal is set.
fn run_cycle<F, Fut>(name: &'static str, build: F)
where
    F: Fn() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    tokio::spawn(async move {
        loop {
            if SHUTTING_DOWN.load(Ordering::SeqCst) {
                return;
            }
            match tokio::spawn(build()).await {
                Ok(()) => {
                    if SHUTTING_DOWN.load(Ordering::SeqCst) {
                        return;
                    }
                    // cycles are meant to run forever; if one returns on its own, restart it anyway
                    log::warn!("cycle [{name}]: returned unexpectedly, restarting");
                }
                Err(e) => {
                    log::error!("cycle [{name}]: panicked, restarting in 5s: {e}");
                }
            }
            sleep(Duration::from_secs(5)).await;
        }
    });
}

#[cfg(test)]
mod generic_history_tests {
    use super::*;

    fn person(id: u64, name: &str) -> UserRef {
        UserRef {
            id: UserKey::discord(id),
            display_name: name.into(),
            username: Some(name.into()),
            automation: platform::types::Automation::Human,
            stability: platform::types::IdentityStability::Persistent,
        }
    }

    fn message(id: u64, timestamp: i64, author: UserRef) -> IncomingMessage {
        IncomingMessage {
            id: MessageKey::discord(id),
            room: RoomRef::from_key(RoomKey::discord(7)),
            author,
            sent_at_unix_ms: timestamp,
            text: id.to_string(),
            mentions: Vec::new(),
            reply_to: None,
            attachments: Vec::new(),
        }
    }

    fn page(messages: Vec<IncomingMessage>, order: platform::outgoing::HistoryOrder)
        -> platform::outgoing::HistoryPage
    {
        platform::outgoing::HistoryPage {
            messages,
            next_cursor: None,
            limitations: Vec::new(),
            order,
        }
    }

    #[test]
    fn cutoff_is_fourteen_days_inclusive_and_respects_page_order() {
        let cutoff = history_cutoff(2_000_000_000);
        assert_eq!(cutoff, (2_000_000_000 - 14 * 24 * 60 * 60) * 1000);
        let messages = || vec![
            message(1, cutoff + 1, person(1, "Alice")),
            message(2, cutoff, person(1, "Alice")),
            message(3, cutoff - 1, person(1, "Alice")),
        ];
        let mut collected = Vec::new();
        assert!(collect_history_page(&mut collected,
            page(messages(), platform::outgoing::HistoryOrder::NewestFirst), cutoff));
        assert_eq!(collected.len(), 2);
        let mut reverse = messages();
        reverse.reverse();
        collected.clear();
        assert!(!collect_history_page(&mut collected,
            page(reverse, platform::outgoing::HistoryOrder::OldestFirst), cutoff));
        assert_eq!(collected.len(), 2);
    }

    #[test]
    fn historical_messages_prepend_live_and_preserve_live_identity() {
        let mut state = State::default();
        let live = person(1, "Alice");
        observe_user(&mut state, &live);
        state.recent_messages.push_back("Alice: live".into());
        apply_history(&mut state, vec![
            message(2, 20, person(1, "Alice")),
            message(1, 10, person(2, "Alice")),
        ]);
        assert_eq!(state.recent_messages.back().unwrap(), "Alice: live");
        assert_eq!(state.recent_messages[0], "Alice [discord:user:2]: 1");
        assert_eq!(state.recent_messages[1], "Alice: 2");
        assert_eq!(state.recent_users["alice"].id, live.id);
        assert_eq!(state.recent_users["alice [discord:user:2]"].id, UserKey::discord(2));
    }

    #[test]
    fn chronological_history_trims_before_prepending_live_messages() {
        let mut state = State::default();
        state.recent_messages.push_back("Alice: live".into());
        let messages = (0..MEMORY_SIZE + 10).rev()
            .map(|i| message(i as u64, i as i64, person(1, "Alice")))
            .collect();
        apply_history(&mut state, messages);
        assert_eq!(state.recent_messages.len(), MEMORY_SIZE);
        assert_eq!(state.recent_messages.front().unwrap(), "Alice: 11");
        assert_eq!(state.recent_messages.back().unwrap(), "Alice: live");
    }

    #[test]
    fn preferred_room_precedes_native_text_position_and_requires_approval() {
        let first = RoomRef::from_key(RoomKey::discord(30));
        let second = RoomRef::from_key(RoomKey::discord(20));
        let space = SpaceRef {
            id: SpaceKey::discord(9),
            name: None,
            rooms: vec![first.clone(), second],
            preferred_room: Some(RoomKey::discord(10)),
        };
        assert_eq!(preferred_route(&space, |_| true).unwrap().room, RoomKey::discord(10));
        assert_eq!(preferred_route(&space, |room| room.id != RoomKey::discord(10)).unwrap(),
            first.conversation());
        assert!(preferred_route(&space, |_| false).is_none());
    }
}
