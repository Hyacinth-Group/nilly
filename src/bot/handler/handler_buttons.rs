impl Handler {
    // settings-panel buttons: apply the change (same paths the commands use), refresh the
    // panel in place. Same permissions as the commands: open to everyone (model switching
    // isn't on the panel)
    async fn setting_button(&self, ctx: &Context, component: ComponentInteraction) {
        let id = component.data.custom_id.clone();
        if let Some(mode) = id.strip_prefix(modal::SETTING_THINKING) {
            if let Some(new_mode) = ThinkingMode::from_arg(mode) {
                self.bot.state().thinking_mode = new_mode;
                memory::write("dusunme.md", new_mode.file_value());
            }
        } else if id == modal::SETTING_DEBUG {
            self.bot.set_debug("");
        } else if id == modal::SETTING_WAKE {
            self.bot.wake();
            self.bot.sleep_transition(ctx).await;
        } else if id == modal::SETTING_SLEEP {
            self.bot.put_to_sleep(8);
            self.bot.sleep_transition(ctx).await;
        } else {
            return;
        }
        log::info!("setting [{}]: {id}", component.user.id);
        let response = modal::settings_message(&self.bot.state(), false);
        if let Err(e) = component
            .create_response(&ctx.http, CreateInteractionResponse::UpdateMessage(response))
            .await
        {
            log::warn!("couldn't refresh settings panel: {e}");
        }
    }

    // the "Show Thought Process" button in hide mode: looks it up in the thought store,
    // opens it as an ephemeral code block visible only to the clicker
    async fn thought_button(&self, ctx: &Context, component: ComponentInteraction) {
        if component.data.custom_id != THOUGHT_BUTTON {
            return;
        }
        let message = MessageKey::discord(component.message.id.get());
        let thought = self.bot.state().thought_store.get(&message).cloned();
        let Some(thought) = thought else {
            let _ = component
                .create_response(
                    &ctx.http,
                    CreateInteractionResponse::Message(
                        CreateInteractionResponseMessage::new()
                            .ephemeral(true)
                            .content("düşünce bulunamadı (bot yeniden başlamış olabilir)"),
                    ),
                )
                .await;
            return;
        };
        let content = thought_display(&thought);
        if let Err(e) = component
            .create_response(
                &ctx.http,
                CreateInteractionResponse::Message(
                    CreateInteractionResponseMessage::new()
                        .ephemeral(true)
                        .content(content),
                ),
            )
            .await
        {
            log::warn!("couldn't send thought ephemeral reply: {e}");
        }
    }
}
