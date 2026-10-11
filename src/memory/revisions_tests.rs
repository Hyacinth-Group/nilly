use super::*;
use std::cell::RefCell;

fn store() -> (tempfile::TempDir, Database) {
    let directory = tempfile::tempdir().unwrap();
    let database = open_private_database(&directory.path().join("memory.redb")).unwrap();
    let txn = database.begin_write().unwrap();
    txn.open_table(CONTENT).unwrap();
    txn.open_table(MODIFIED).unwrap();
    initialize(&txn).unwrap();
    txn.commit().unwrap();
    (directory, database)
}
fn seed(database: &Database, key: &str, content: &str, when: u64) {
    let txn = database.begin_write().unwrap();
    txn.open_table(CONTENT)
        .unwrap()
        .insert(key, content)
        .unwrap();
    txn.open_table(MODIFIED).unwrap().insert(key, when).unwrap();
    txn.commit().unwrap();
}
fn replace(
    database: &Database,
    key: &str,
    revision: u64,
    content: &str,
) -> Result<RecordSnapshot, MutationError> {
    mutate_database(
        database,
        key,
        Some(revision),
        false,
        123,
        |_| Ok(content.to_owned()),
        |_, _| {},
    )
}
fn historical(database: &Database, key: &str, revision: u64) -> RecordSnapshot {
    let txn = database.begin_read().unwrap();
    let table = txn.open_table(HISTORY).unwrap();
    let key = serde_json::to_string(&(key, revision)).unwrap();
    serde_json::from_str(table.get(key.as_str()).unwrap().unwrap().value()).unwrap()
}

#[test]
fn legacy_same_second_cas_and_exact_unicode_history() {
    let (_directory, database) = store();
    let key = "konular/Unicode.md";
    let original = "# Türkçe 🧠\n- İstanbul, 中文, café\n";
    seed(&database, key, original, 123);
    let snapshot = read_database_versioned(&database, key).unwrap().unwrap();
    assert_eq!(snapshot.revision, 0);
    let saved = replace(&database, key, snapshot.revision, "# düzeltilmiş 🧠\n").unwrap();
    assert_eq!(saved.revision, 1);
    assert_eq!(saved.modified, snapshot.modified);
    assert_eq!(historical(&database, key, 0).content, original);
    let error = replace(&database, key, snapshot.revision, "stale").unwrap_err();
    assert!(
        matches!(error, MutationError::Conflict(current) if current.revision == 1 && current.content == saved.content)
    );
    let second = replace(&database, key, 1, "second").unwrap();
    assert_eq!(second.revision, 2);
    assert_eq!(historical(&database, key, 1).content, saved.content);
}

#[test]
fn absent_temperament_creation_competes_by_revision() {
    let (_directory, database) = store();
    assert!(read_database_versioned(&database, "huy.md")
        .unwrap()
        .is_none());
    let created = mutate_database(
        &database,
        "huy.md",
        Some(0),
        true,
        1,
        |_| Ok("new".into()),
        |_, _| {},
    )
    .unwrap();
    assert_eq!(created.revision, 1);
    assert!(matches!(
        mutate_database(
            &database,
            "huy.md",
            Some(0),
            true,
            1,
            |_| Ok("loser".into()),
            |_, _| {}
        ),
        Err(MutationError::Conflict(_))
    ));
    assert!(matches!(
        replace(&database, "konular/missing.md", 0, "absent"),
        Err(MutationError::Missing)
    ));
}

#[test]
fn person_delta_reads_manual_edit_not_its_old_snapshot() {
    let (_directory, database) = store();
    let id = UserKey::discord(42);
    let key = platform::keys::person_record(&id);
    let original = "# Çağrı\nid: discord:user:42\npuan: +2\nnot: old\ncustom: untouched 🧠\n- header bullet\n\n## Bildiklerin\n- fact\n\n## Son olaylar\n\n## Özel bölüm\npuan: 999\n- untouched\n";
    seed(&database, &key, original, 1);
    let stale = read_database_versioned(&database, &key).unwrap().unwrap();
    let mut edited = Person::parse(&id, original);
    edited.score = 6;
    edited.facts.push("manual fact".into());
    let edited = edited.text_preserving(original);
    replace(&database, &key, stale.revision, &edited).unwrap();
    let delta = mutate_database(
        &database,
        &key,
        None,
        true,
        123,
        |latest| {
            Ok(person_updated_content(&id, latest, false, |person| {
                person.score += 3;
                person.facts.push("background fact".into());
            }))
        },
        |_, _| {},
    )
    .unwrap();
    let person = Person::parse(&id, &delta.content);
    assert_eq!(person.score, 9);
    assert_eq!(person.id, id);
    assert!(
        person.events.is_empty(),
        "unknown header bullets must not become events"
    );
    assert!(!person.facts.iter().any(|fact| fact == "header bullet"));
    assert!(person.facts.contains(&"manual fact".to_owned()));
    assert!(person.facts.contains(&"background fact".to_owned()));
    assert!(delta.content.contains("custom: untouched 🧠\n"));
    assert!(delta.content.contains("- header bullet\n"));
    assert!(delta
        .content
        .contains("## Özel bölüm\npuan: 999\n- untouched\n"));
    assert!(matches!(
        replace(&database, &key, stale.revision, "old model prose"),
        Err(MutationError::Conflict(_))
    ));
}

#[test]
fn coach_and_summarizer_discard_stale_outputs() {
    let (_directory, database) = store();
    for key in [
        "huy.md",
        "konular/summary.md",
        "profil.md",
        "kendim.md",
        "duzeltmeler.md",
    ] {
        seed(&database, key, "model input", 1);
        let used = read_database_versioned(&database, key).unwrap().unwrap();
        replace(&database, key, used.revision, "manual correction").unwrap();
        let published = RefCell::new(false);
        let result = mutate_database(
            &database,
            key,
            Some(used.revision),
            true,
            123,
            |_| Ok("stale model output".into()),
            |_, _| *published.borrow_mut() = true,
        );
        assert!(matches!(result, Err(MutationError::Conflict(_))));
        assert!(!*published.borrow());
        assert_eq!(
            read_database_versioned(&database, key)
                .unwrap()
                .unwrap()
                .content,
            "manual correction"
        );
    }
}

#[test]
fn archive_failure_retains_full_displaced_body_transactionally() {
    let (directory, database) = store();
    let key = "olaylar/2026-10.md";
    let original = "earlier summary prose\n- older 🧠\n- newer İstanbul\n";
    seed(&database, key, original, 1);
    let record = replace(&database, key, 0, "short summary\n- newer İstanbul\n").unwrap();
    let impossible_root = directory.path().join("not-a-directory");
    fs::write(&impossible_root, "file").unwrap();
    assert!(archive_write_root(&impossible_root, "arsiv/olaylar/2026-10.md", original).is_err());
    assert_eq!(historical(&database, key, 0).content, original);
    assert_eq!(
        read_database_versioned(&database, key)
            .unwrap()
            .unwrap()
            .content,
        record.content
    );
}

#[test]
fn derived_index_and_publication_happen_only_after_durable_commit() {
    let (_directory, database) = store();
    let key = "konular/commit.md";
    seed(&database, key, "# Old\n- old note\n", 1);
    refresh_index_database(&database).unwrap();
    let old_index = read_database(&database, "INDEX.md").unwrap().unwrap();
    let published = RefCell::new(Vec::new());
    let result = mutate_database_commit(
        &database,
        MutationSpec {
            key,
            expected: Some(0),
            create: false,
            when: 123,
        },
        |_| Ok("# New\n- new note\n".into()),
        |record, index| {
            published.borrow_mut().push((record.content.clone(), index));
        },
        |txn| {
            drop(txn);
            Err("injected durable commit failure".into())
        },
    );
    assert!(matches!(result, Err(MutationError::Storage(_))));
    assert!(published.borrow().is_empty());
    assert_eq!(
        read_database(&database, key).unwrap().unwrap(),
        "# Old\n- old note\n"
    );
    assert_eq!(
        read_database(&database, "INDEX.md").unwrap().unwrap(),
        old_index
    );
    let txn = database.begin_read().unwrap();
    assert_eq!(txn.open_table(HISTORY).unwrap().len().unwrap(), 0);
    drop(txn);
    mutate_database(
        &database,
        key,
        Some(0),
        false,
        123,
        |_| Ok("# New\n- new note\n".into()),
        |record, index| {
            // A read after commit observes the same content/index being published.
            assert_eq!(
                read_database(&database, key).unwrap().unwrap(),
                record.content
            );
            assert_eq!(
                read_database(&database, "INDEX.md").unwrap().unwrap(),
                index
            );
            assert!(index.contains("New"));
            published.borrow_mut().push((record.content.clone(), index));
        },
    )
    .unwrap();
    assert_eq!(published.borrow().len(), 1);
}

#[test]
fn browser_pages_every_record_beyond_context_limits() {
    let (_directory, database) = store();
    let txn = database.begin_write().unwrap();
    {
        let mut bodies = txn.open_table(CONTENT).unwrap();
        let mut modified = txn.open_table(MODIFIED).unwrap();
        for index in 0..75 {
            for key in [
                format!("konular/topic-{index:03}.md"),
                platform::keys::person_record(&UserKey::discord(index + 1)),
            ] {
                bodies
                    .insert(
                        key.as_str(),
                        "# Search 🧠 Türkçe\nsummary prose\n- raw line\n",
                    )
                    .unwrap();
                modified.insert(key.as_str(), 123).unwrap();
            }
        }
        bodies
            .insert("kisiler/legacy-unknown.md", "not a canonical person")
            .unwrap();
    }
    txn.commit().unwrap();
    for prefix in ["kisiler/", "konular/"] {
        let mut cursor = None;
        let mut keys = Vec::new();
        loop {
            let page = browse_database(&database, prefix, "türkçe", cursor.as_deref(), 13).unwrap();
            assert!(page.records.len() <= 13);
            for record in page.records {
                assert!(record.content.contains("summary prose"));
                assert_eq!(record.revision, 0);
                keys.push(record.key);
            }
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(keys.len(), 75);
        let mut unique = keys.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(keys, unique); // Equal timestamps order lexicographically without duplicates.
    }
    seed(
        &database,
        "olaylar/2026-10.md",
        "summary prose\n- raw event\n",
        999,
    );
    assert!(
        browse_database(&database, "olaylar/", "summary prose", None, 1)
            .unwrap()
            .records[0]
            .content
            .contains("summary prose")
    );
}

#[test]
fn browser_validates_search_page_and_cursor_bounds() {
    let (_directory, database) = store();
    assert!(browse_database(&database, "../", "", None, 10).is_err());
    assert!(browse_database(&database, "konular/", &"ü".repeat(201), None, 10).is_err());
    for limit in [0, 101] {
        assert!(browse_database(&database, "konular/", "", None, limit).is_err());
    }
    for cursor in ["invalid", "[1,\"kisiler/wrong.md\"]", "[1,\"../secret\"]"] {
        assert!(browse_database(&database, "konular/", "", Some(cursor), 10).is_err());
    }
    assert!(browse_database(&database, "konular/", "", Some(&"x".repeat(2049)), 10).is_err());
}

fn bot_fixture() -> Bot {
    Bot {
        state: Mutex::new(State::default()),
        adapters: Mutex::new(platform::ConnectionRegistry::default()),
        cycles_started: std::sync::atomic::AtomicBool::new(false),
        announcement_sent: std::sync::atomic::AtomicBool::new(false),
        http: reqwest::Client::new(),
        api_url: "https://custom.invalid/chat/completions".into(),
        key: String::new(),
        news_channel: None,
        firecrawl: None,
        guild_id: None,
        allowed_channels: None,
        debug_channel: None,
        image_analysis: false,
        stealth: false,
        reasoning_mandatory_models: Mutex::new(std::collections::HashSet::new()),
    }
}

#[test]
fn real_bot_prose_caches_are_unchanged_on_failed_commit() {
    for key in [
        "huy.md",
        "profil.md",
        "duzeltmeler.md",
        "kendim.md",
        "gundem.md",
    ] {
        let (_directory, database) = store();
        let bot = bot_fixture();
        let old_agenda = "## 2026-10-01\ncached old";
        let fresh = if key == "gundem.md" {
            "## 2026-10-02\nfresh"
        } else {
            "fresh"
        };
        seed(
            &database,
            key,
            if key == "gundem.md" {
                old_agenda
            } else {
                "cached old"
            },
            1,
        );
        {
            let mut state = bot.state();
            state.temperament = "cached old".into();
            state.profile = "cached old".into();
            state.corrections = "cached old".into();
            state.myself = "cached old".into();
            state.agenda = old_agenda.into();
            state.index = "old index".into();
        }
        let result = mutate_database_commit(
            &database,
            MutationSpec {
                key,
                expected: Some(0),
                create: false,
                when: 123,
            },
            |_| Ok(fresh.into()),
            |record, index| publish_record(&bot, record, index),
            |txn| {
                drop(txn);
                Err("injected commit failure".into())
            },
        );
        assert!(result.is_err());
        {
            let state = bot.state();
            assert_eq!(state.temperament, "cached old");
            assert_eq!(state.profile, "cached old");
            assert_eq!(state.corrections, "cached old");
            assert_eq!(state.myself, "cached old");
            assert_eq!(state.agenda, old_agenda);
            assert_eq!(state.index, "old index");
        }
        coordinated(&database, Some(&bot), key, Some(0), false, 123, |_| {
            Ok(fresh.into())
        })
        .unwrap();
        let state = bot.state();
        let cached = match key {
            "huy.md" => &state.temperament,
            "profil.md" => &state.profile,
            "duzeltmeler.md" => &state.corrections,
            "kendim.md" => &state.myself,
            _ => &state.agenda,
        };
        let expected = if key == "gundem.md" {
            crate::agenda::latest_agenda(fresh)
        } else {
            fresh.to_owned()
        };
        assert_eq!(cached, &expected);
        assert_eq!(
            state.index,
            read_database(&database, "INDEX.md").unwrap().unwrap()
        );
    }
}

#[test]
fn model_override_removal_and_control_errors_publish_only_after_commit() {
    let (_directory, database) = store();
    let bot = bot_fixture();
    seed(&database, "model.md", "saved-model", 1);
    seed(&database, "dusunme.md", "goster", 1);
    seed(&database, "debug.md", "kapali", 1);
    bot.state().model = "saved-model".into();
    persist_control_commit(
        &database,
        &bot,
        "model.md",
        None,
        |state| state.model = "configured-model".into(),
        |txn| txn.commit().map_err(|e| e.into()),
    )
    .unwrap();
    assert!(read_database(&database, "model.md").unwrap().is_none());
    assert_eq!(bot.state().model, "configured-model");
    let txn = database.begin_read().unwrap();
    assert!(txn
        .open_table(MODIFIED)
        .unwrap()
        .get("model.md")
        .unwrap()
        .is_none());
    drop(txn);
    for (key, value) in [
        ("model.md", "new-model"),
        ("dusunme.md", "kapali"),
        ("debug.md", "acik"),
    ] {
        let result = persist_control_commit(
            &database,
            &bot,
            key,
            Some(value),
            |state| {
                state.model = "must not publish".into();
                state.thinking_mode = ThinkingMode::Off;
                state.debug = true;
            },
            |txn| {
                drop(txn);
                Err("injected commit failure".into())
            },
        );
        assert!(result.is_err());
        let state = bot.state();
        assert_eq!(state.model, "configured-model");
        assert_eq!(state.thinking_mode, ThinkingMode::Show);
        assert!(!state.debug);
    }
    assert!(read_database(&database, "model.md").unwrap().is_none());
    assert_eq!(
        read_database(&database, "dusunme.md").unwrap().unwrap(),
        "goster"
    );
    assert_eq!(
        read_database(&database, "debug.md").unwrap().unwrap(),
        "kapali"
    );
}

#[test]
fn concurrent_debug_toggles_read_latest_committed_cache() {
    let (_directory, database) = store();
    let database = Arc::new(database);
    let bot = Arc::new(bot_fixture());
    let mut threads = Vec::new();
    for _ in 0..2 {
        let database = Arc::clone(&database);
        let bot = Arc::clone(&bot);
        threads.push(std::thread::spawn(move || {
            set_debug_commit(&database, &bot, None, |txn| {
                txn.commit().map_err(|e| e.into())
            })
            .unwrap()
        }));
    }
    let mut outcomes: Vec<bool> = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect();
    outcomes.sort();
    assert_eq!(outcomes, vec![false, true]);
    assert!(!bot.state().debug);
    assert_eq!(
        read_database(&database, "debug.md").unwrap().unwrap(),
        "kapali"
    );
    assert!(set_debug_commit(&database, &bot, None, |txn| {
        drop(txn);
        Err("injected commit failure".into())
    })
    .is_err());
    assert!(!bot.state().debug);
    assert_eq!(
        read_database(&database, "debug.md").unwrap().unwrap(),
        "kapali"
    );
}

#[test]
fn person_updates_restore_identity_and_enforce_favorite_and_bounds() {
    let id = UserKey::discord(42);
    let original = "# Emin\nid: telegram:user:42\npuan: -5\nnot: original\n";
    let ordinary = person_updated_content(&id, original, false, |person| {
        person.id = UserKey::discord(7);
        person.score = i32::MAX;
        person.tags = (0..12).map(|index| format!("tag-{index}")).collect();
        person.previous_names = (0..12).map(|index| format!("name-{index}")).collect();
    });
    let parsed = Person::parse(&id, &ordinary);
    assert_eq!(parsed.id, id);
    assert!(ordinary.contains("id: discord:user:42\n"));
    assert_eq!(parsed.score, 10);
    assert_eq!(parsed.tags.len(), 6);
    assert_eq!(parsed.previous_names.len(), 5);
    let favorite = person_updated_content(&id, original, true, |person| {
        person.score = -10;
        person.note = "model changed opinion".into();
    });
    let favorite = Person::parse(&id, &favorite);
    assert_eq!(favorite.score, 10);
    assert_eq!(favorite.note, FAVORITE_NOTE);
}

#[test]
fn unknown_person_text_keeps_its_original_sections_across_repeated_updates() {
    let id = UserKey::discord(42);
    let original = "# Ada\nid: discord:user:42\npuan: +1\nnot: opinion\ncustom: header 🧠\n- header bullet\n\n## Bildiklerin\n- known fact\nnot: facts-only prose\n\n## Özel ara bölüm\n- private bullet\n\n## Son olaylar\n- actual event\ncustom: events-only prose\n\n## Özel son bölüm\nbody\n";
    let mut person = Person::parse(&id, original);
    person.facts.push("new fact".into());
    let first = person.text_preserving(original);
    let parsed = Person::parse(&id, &first);
    assert_eq!(parsed.facts, person.facts);
    assert_eq!(parsed.events, vec!["actual event"]);
    assert_eq!(parsed.note, "opinion");
    assert!(first.find("- header bullet").unwrap() < first.find("## Bildiklerin").unwrap());
    assert!(
        first.find("not: facts-only prose").unwrap() < first.find("## Özel ara bölüm").unwrap()
    );
    assert!(first.find("## Özel ara bölüm").unwrap() < first.find("## Son olaylar").unwrap());
    assert!(
        first.find("custom: events-only prose").unwrap() < first.find("## Özel son bölüm").unwrap()
    );
    assert!(first.contains("## Özel ara bölüm\n- private bullet\n\n"));
    assert!(first.contains("## Özel son bölüm\nbody\n"));
    assert_eq!(parsed.text_preserving(&first), first);
}
