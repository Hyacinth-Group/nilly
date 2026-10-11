// the visible part of the reply while streaming is in progress: completed lines (ones
// followed by \n) plus the trailing half-line, but only once it clears HALF_LINE_THRESHOLD
// characters. That way "tep" doesn't sit half-formed as a message that gets deleted on the
// next edit, and a short line doesn't trigger a wasted edit.
fn stream_slice(reply: &str, done: bool) -> &str {
    if done {
        return reply;
    }
    let (complete, partial) = match reply.rfind('\n') {
        Some(i) => (&reply[..i], &reply[i + 1..]),
        None => ("", reply),
    };
    if partial.trim().chars().count() >= HALF_LINE_THRESHOLD {
        reply
    } else {
        complete
    }
}


// thinking shouldn't get a newline per thought; collapsed into a single flowing line
fn single_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

// reconciles the layout with the messages already open: changed ones are edited, missing
// ones are opened, and if the text got shorter (e.g. the name prefix got stripped) extra
// messages are deleted
/// The room is resolved once for the whole reply; only the first message gets a
/// visual reply target. Network errors are logged, not propagated.
async fn write_stream(
    transport: &platform::Adapter,
    room: &platform::types::RoomRef,
    sent: &mut Vec<platform::outgoing::MessageReceipt>,
    layout: &[String],
    reply_to: Option<&MessageKey>,
) {
    let target = reply_to.map(|id| platform::types::MessageRef {
        id: id.clone(), room: room.id.clone(), author: None,
    });
    // Reconcile in order: edits, appends, then reverse-order excess deletion.
    for (i, content) in layout.iter().enumerate() {
        match sent.get_mut(i) {
            Some(receipt) if receipt.last_accepted_text != *content => {
                let body = platform::outgoing::OutgoingMessage::text(content);
                match transport.edit(receipt, &body).await {
                    Ok(()) => {
                        receipt.last_accepted_text.clone_from(content);
                        receipt.version += 1;
                    }
                    Err(e) => log::warn!("edit failed ({}): {e}", room.id),
                }
            }
            Some(_) => {}
            None => {
                let mut body = platform::outgoing::OutgoingMessage::text(content);
                body.reply_to = target.as_ref().filter(|_| i == 0);
                body.mentions.reply_user = body.reply_to.is_some();
                match transport.send(room, &body).await {
                    Ok(receipt) => sent.push(receipt),
                    Err(e) => {
                        log::error!("send failed ({}): {e}", room.id);
                        break;
                    }
                }
            }
        }
    }
    while sent.len() > layout.len() {
        if let Some(receipt) = sent.pop() {
            let _ = transport.delete(&receipt).await;
        }
    }
}

/// Best-effort cleanup of provisional messages; individual deletion failures are ignored.
async fn delete_messages(
    transport: &platform::Adapter,
    messages: Vec<platform::outgoing::MessageReceipt>,
) {
    for receipt in messages {
        let _ = transport.delete(&receipt).await;
    }
}

