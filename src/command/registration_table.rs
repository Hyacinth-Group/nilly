// Slash command manager: a registration table (`definitions`) holding each command's
// name+description+options+runner in one place. `modal::register_commands` builds the
// Discord registration list from this table, and `interaction_create` (main.rs) looks up
// an incoming `Interaction::Command` in it by name and runs it. There are no `!`/text
// commands — the bot is only managed via slash (plain messages only ever feed the
// chat/memory pipeline).
//
// Discord expects a first response within 3s: local/fast commands return an embed
// directly, commands that make a network/model call first acknowledge with `defer` and
// then edit the result in with `report_result` (news/problem/prank/agents/wander/wake/sleep
// already post their own messages to the channel via `Bot::send`; the embed here is just a short "ok" note).

use super::*;
use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;

// /yardim's embed description used to live here as a Rust `HELP` const; it's
// `langs/tr.json`'s `help.text` now (see AGENTS.md rule 7), read directly by
// `modal::help_message`.

// ---------- command registration table ----------

/// A boxed-future function pointer type for a command's runner: takes the shared `&Bot`,
/// the event `&Context`, and the `&CommandInteraction`, returns a pinned `()`-future.
type CommandFn = for<'a> fn(
    &'a Bot,
    &'a Context,
    &'a CommandInteraction,
) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

/// One slash command's full registration: `name`/`description` (Discord-facing, Turkish),
/// `options` (a fn building its `CreateCommandOption`s), `run` (its handler, see
/// `CommandFn`). Built by `define_command!` below; the full table lives in `definitions`
/// below.
pub struct CommandDefinition {
    pub name: &'static str,
    pub description: &'static str,
    pub options: fn() -> Vec<CreateCommandOption>,
    pub run: CommandFn,
}

/// Builds one `CommandDefinition`, wrapping `$f` (an `async fn(&Bot, &Context,
/// &CommandInteraction)`) into the boxed-future shape `CommandFn` requires.
macro_rules! define_command {
    ($name:expr, $description:expr, $options:expr, $f:expr) => {
        CommandDefinition {
            name: $name,
            description: $description,
            options: $options,
            run: |bot, ctx, interaction| Box::pin($f(bot, ctx, interaction)),
        }
    };
}

/// The single source of truth for every slash command.
/// Built once and reused for registration and dispatch.
pub fn definitions() -> &'static [CommandDefinition] {
    static TABLE: OnceLock<Vec<CommandDefinition>> = OnceLock::new();
    TABLE.get_or_init(|| {
        vec![
            define_command!(
                strings::t("cmd.durum.name"),
                strings::t("cmd.durum.description"),
                Vec::new,
                cmd_status
            ),
            define_command!(
                strings::t("cmd.yardim.name"),
                strings::t("cmd.yardim.description"),
                Vec::new,
                cmd_help
            ),
            define_command!(
                strings::t("cmd.zihin.name"),
                strings::t("cmd.zihin.description"),
                options_mind,
                cmd_mind
            ),
            define_command!(
                strings::t("cmd.ayarlar.name"),
                strings::t("cmd.ayarlar.description"),
                Vec::new,
                cmd_settings
            ),
            define_command!(
                strings::t("cmd.sifirla.name"),
                strings::t("cmd.sifirla.description"),
                options_reset,
                cmd_reset
            ),
            define_command!(
                strings::t("cmd.haber.name"),
                strings::t("cmd.haber.description"),
                Vec::new,
                cmd_news
            ),
            define_command!(
                strings::t("cmd.sorun.name"),
                strings::t("cmd.sorun.description"),
                Vec::new,
                cmd_problem
            ),
            define_command!(
                strings::t("cmd.gez.name"),
                strings::t("cmd.gez.description"),
                Vec::new,
                cmd_wander
            ),
            define_command!(
                strings::t("cmd.saka.name"),
                strings::t("cmd.saka.description"),
                Vec::new,
                cmd_prank
            ),
            define_command!(
                strings::t("cmd.hack.name"),
                strings::t("cmd.hack.description"),
                Vec::new,
                cmd_hack
            ),
            define_command!(
                strings::t("cmd.ajanlar.name"),
                strings::t("cmd.ajanlar.description"),
                Vec::new,
                cmd_agents
            ),
            define_command!(
                strings::t("cmd.uyan.name"),
                strings::t("cmd.uyan.description"),
                Vec::new,
                cmd_wake
            ),
            define_command!(
                strings::t("cmd.uyu.name"),
                strings::t("cmd.uyu.description"),
                options_sleep,
                cmd_sleep
            ),
            define_command!(
                strings::t("cmd.dusunme.name"),
                strings::t("cmd.dusunme.description"),
                options_thinking,
                cmd_thinking
            ),
            define_command!(
                strings::t("cmd.model.name"),
                strings::t("cmd.model.description"),
                options_model,
                cmd_model
            ),
            define_command!(
                strings::t("cmd.debug.name"),
                strings::t("cmd.debug.description"),
                options_debug,
                cmd_debug
            ),
        ]
    })
}

// ---------- option definitions ----------

