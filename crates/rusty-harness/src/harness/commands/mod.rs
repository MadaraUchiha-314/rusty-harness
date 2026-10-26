//! Commands: the `Command` entity, the parser and the built-ins (R10).
//!
//! A message from a **person** that starts with `/<name>` is a command, never a message
//! to the model; commands are not parsed from tool results or sub-agent replies (design.md
//! § Security design, boundary 4). `command.invoke` runs through the dispatcher, with
//! the role check as a guard (abuse case 10), and the built-ins `/exit`, `/model` and
//! `/effort` change the next call without restarting the task (R9.5).

use std::collections::BTreeSet;
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};

use crate::HarnessError;
use crate::assets::{FmtArgs, StringKey, keys};
use crate::harness::core_loop::task::{Caller, Role};
use crate::harness::hooks::remote::{RemoteTransport, call_remote};
use crate::harness::hooks::{
    GuardHook, GuardOutcome, Hook, HookCx, HookDispatcher, HookError, HookOrder, HookPoint,
    Operation, Veto,
};
use crate::harness::models::llm::{Effort, Llm, MessageRole};
use crate::harness::plugins::DeclaredCommand;
use crate::harness_async_trait;
use crate::registry::{Entity, EntityId, EntityKind, Registry};

/// How a command describes itself in the palette.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandDescription {
    /// A catalog key (built-ins, R33.6).
    Key(StringKey),
    /// Text a plugin supplied (plugin content, not an application string).
    Plugin(String),
}

impl CommandDescription {
    /// The text to show in `locale`.
    #[must_use]
    pub fn render(&self, catalog: &crate::assets::Catalog, locale: &str) -> String {
        match self {
            Self::Key(key) => catalog.get(*key, locale, &FmtArgs::new()),
            Self::Plugin(text) => text.clone(),
        }
    }
}

/// What the palette and the role guard know about a command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandDescriptor {
    /// The name after the slash.
    pub name: String,
    /// Its description.
    pub description: CommandDescription,
    /// Roles that may invoke it; empty means every participant.
    pub roles: BTreeSet<Role>,
}

/// A parsed invocation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandCall {
    /// The name after the slash.
    pub name: String,
    /// The rest of the line, trimmed.
    pub args: String,
    /// Who invoked it.
    pub caller: Caller,
}

/// A rendered line for the person, as a catalog key plus arguments (R25.6).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Line {
    /// The key.
    pub key: String,
    /// Arguments, as text.
    pub args: Vec<(String, String)>,
}

impl Line {
    fn new(key: StringKey, args: &[(&str, String)]) -> Self {
        Self {
            key: key.as_str().to_owned(),
            args: args
                .iter()
                .map(|(k, v)| ((*k).to_owned(), v.clone()))
                .collect(),
        }
    }

    /// Renders the line with the catalog.
    #[must_use]
    pub fn render(&self, catalog: &crate::assets::Catalog, locale: &str) -> String {
        let key = keys_by_name(&self.key).unwrap_or(keys::command::UNKNOWN_NO_SUGGESTION);
        let mut fmt = FmtArgs::new();
        for (k, v) in &self.args {
            fmt = fmt.text(k, v.clone());
        }
        catalog.get(key, locale, &fmt)
    }
}

fn keys_by_name(name: &str) -> Option<StringKey> {
    [
        keys::command::UNKNOWN,
        keys::command::UNKNOWN_NO_SUGGESTION,
        keys::command::DENIED,
        keys::command::model::LIST,
        keys::command::model::UNKNOWN,
        keys::command::model::SWITCHED,
        keys::command::effort::SWITCHED,
        keys::command::effort::UNKNOWN,
        keys::command::exit::BYE,
    ]
    .into_iter()
    .find(|k| k.as_str() == name)
}

/// The `commands/v1` extension URI: the metadata key of command tokens and outcomes.
pub const COMMANDS_EXT: &str =
    "https://madarauchiha-314.github.io/rusty-harness/a2a/ext/commands/v1";

/// One command as the `commands/v1` card payload lists it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CommandEntry {
    /// The name after the slash.
    pub name: String,
    /// The catalog key of its description, or the plugin's text.
    pub description_key: String,
    /// Roles that may invoke it; empty means every participant.
    pub roles: Vec<Role>,
}

/// A command token in a message (`commands/v1` message payload), for highlighting.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CommandToken {
    /// Byte offset of the slash.
    pub start: usize,
    /// Byte offset past the token.
    pub end: usize,
    /// The command.
    pub name: String,
}

/// What a command did, on the message that invoked it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CommandOutcome {
    /// The command.
    pub name: String,
    /// `done`, `exit`, `unknown` or `failed`.
    pub outcome: String,
}

/// The `commands/v1` extension payload: the card form lists the commands, the message
/// form marks tokens and the outcome (design.md § A2A extensions).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CommandsMeta {
    /// Card: the registered commands.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<CommandEntry>,
    /// Message: the command tokens.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tokens: Vec<CommandToken>,
    /// Message: what the command did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<CommandOutcome>,
}

impl CommandsMeta {
    /// The card payload for every registered command.
    #[must_use]
    pub fn card(registry: &Registry) -> Self {
        Self {
            commands: registry
                .list::<dyn Command>()
                .iter()
                .map(|c| {
                    let d = c.descriptor();
                    CommandEntry {
                        name: d.name.clone(),
                        description_key: match &d.description {
                            CommandDescription::Key(k) => k.as_str().to_owned(),
                            CommandDescription::Plugin(t) => t.clone(),
                        },
                        roles: d.roles.iter().copied().collect(),
                    }
                })
                .collect(),
            tokens: Vec::new(),
            result: None,
        }
    }
}

/// What a command did (R10.4).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "outcome")]
pub enum CommandResult {
    /// It ran; show this line.
    Done {
        /// The line.
        line: Line,
    },
    /// The surface should leave the session.
    Exit {
        /// The line.
        line: Line,
    },
    /// No command has that name; nothing was sent to the model (R10.6).
    Unknown {
        /// The line, with a suggestion when one is close.
        line: Line,
    },
    /// The command failed; show this line.
    Failed {
        /// The line.
        line: Line,
    },
}

harness_async_trait! {
    /// A slash command (R10.3).
    pub trait Command: Entity {
        /// Its descriptor.
        fn descriptor(&self) -> &CommandDescriptor;
        /// Runs it.
        async fn run(&self, call: &CommandCall) -> Result<CommandResult, HarnessError>;
    }
}

/// The `command.invoke` operation; `pre` hooks may veto, the role guard denies (R10.4).
#[derive(Debug)]
pub struct CommandInvoke;

impl Operation for CommandInvoke {
    const POINT: HookPoint = HookPoint::CommandInvoke;
    type Input = CommandCall;
    type Output = CommandResult;
    fn identity(input: &CommandCall) -> String {
        input.name.clone()
    }
}

/// Splits `/name rest` on a line a **person** wrote; `None` for anything else.
#[derive(Debug, Default, Clone, Copy)]
pub struct CommandParser;

impl CommandParser {
    /// The command in `text`, when `role` is a person's message and the text starts
    /// with `/<name>`.
    #[must_use]
    pub fn parse(role: MessageRole, human: bool, text: &str) -> Option<(String, String)> {
        if role != MessageRole::User || !human {
            return None;
        }
        let rest = text.trim_start().strip_prefix('/')?;
        let (name, args) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return None;
        }
        Some((name.to_owned(), args.trim().to_owned()))
    }
}

/// The model and effort the next call uses (R9.5): `/model` and `/effort` change it, the
/// context builder reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelSelection {
    /// The alias of the model in `[models]`.
    pub model: String,
    /// The effort, when set.
    pub effort: Option<Effort>,
}

/// Resolves the current selection to a model (R9.5).
pub struct ModelRouter {
    registry: Arc<Registry>,
    selection: Arc<RwLock<ModelSelection>>,
}

impl core::fmt::Debug for ModelRouter {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ModelRouter")
            .field("selection", &self.selection())
            .finish()
    }
}

impl ModelRouter {
    /// A router starting with `selection`.
    #[must_use]
    pub fn new(registry: Arc<Registry>, selection: ModelSelection) -> Self {
        Self {
            registry,
            selection: Arc::new(RwLock::new(selection)),
        }
    }

    /// The current selection.
    #[must_use]
    pub fn selection(&self) -> ModelSelection {
        self.selection
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// The shared selection, for the commands that change it.
    #[must_use]
    pub fn shared(&self) -> Arc<RwLock<ModelSelection>> {
        Arc::clone(&self.selection)
    }

    /// The model the next call goes to.
    ///
    /// # Errors
    ///
    /// [`HarnessError::Internal`] naming the alias when it is not registered.
    pub fn current(&self) -> Result<Arc<dyn Llm>, HarnessError> {
        let alias = self.selection().model;
        self.registry
            .get::<dyn Llm>(&EntityId::new(alias))
            .map_err(|e| HarnessError::Internal(e.to_string()))
    }

    /// The configured aliases.
    #[must_use]
    pub fn aliases(&self) -> Vec<String> {
        self.registry
            .list::<dyn Llm>()
            .iter()
            .map(|m| m.entity_id().id.to_string())
            .collect()
    }
}

/// `/exit`.
#[derive(Debug)]
pub struct ExitCommand {
    id: EntityId,
    descriptor: CommandDescriptor,
}

impl Default for ExitCommand {
    fn default() -> Self {
        Self {
            id: EntityId::new("exit"),
            descriptor: CommandDescriptor {
                name: "exit".into(),
                description: CommandDescription::Key(keys::command::exit::DESCRIPTION),
                roles: BTreeSet::new(),
            },
        }
    }
}

impl Entity for ExitCommand {
    fn kind(&self) -> EntityKind {
        EntityKind::Command
    }
    fn entity_id(&self) -> &EntityId {
        &self.id
    }
}

harness_async_trait! {
    impl Command for ExitCommand {
        fn descriptor(&self) -> &CommandDescriptor {
            &self.descriptor
        }
        async fn run(&self, _call: &CommandCall) -> Result<CommandResult, HarnessError> {
            Ok(CommandResult::Exit { line: Line::new(keys::command::exit::BYE, &[]) })
        }
    }
}

/// `/model [alias]`.
pub struct ModelCommand {
    id: EntityId,
    descriptor: CommandDescriptor,
    router: Arc<ModelRouter>,
}

impl core::fmt::Debug for ModelCommand {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ModelCommand").finish_non_exhaustive()
    }
}

impl ModelCommand {
    /// The command over `router`; admins and assignees may use it.
    #[must_use]
    pub fn new(router: Arc<ModelRouter>) -> Self {
        Self {
            id: EntityId::new("model"),
            descriptor: CommandDescriptor {
                name: "model".into(),
                description: CommandDescription::Key(keys::command::model::DESCRIPTION),
                roles: BTreeSet::from([Role::Admin, Role::Assignee, Role::Reporter]),
            },
            router,
        }
    }
}

impl Entity for ModelCommand {
    fn kind(&self) -> EntityKind {
        EntityKind::Command
    }
    fn entity_id(&self) -> &EntityId {
        &self.id
    }
}

harness_async_trait! {
    impl Command for ModelCommand {
        fn descriptor(&self) -> &CommandDescriptor {
            &self.descriptor
        }
        async fn run(&self, call: &CommandCall) -> Result<CommandResult, HarnessError> {
            let aliases = self.router.aliases();
            let list = aliases.join(", ");
            if call.args.is_empty() {
                return Ok(CommandResult::Done { line: Line::new(keys::command::model::LIST, &[("models", list), ("current", self.router.selection().model)]) });
            }
            let alias = call.args.split_whitespace().next().unwrap_or("").to_owned();
            if !aliases.contains(&alias) {
                return Ok(CommandResult::Failed { line: Line::new(keys::command::model::UNKNOWN, &[("alias", alias), ("models", list)]) });
            }
            self.router.shared().write().unwrap_or_else(std::sync::PoisonError::into_inner).model.clone_from(&alias);
            Ok(CommandResult::Done { line: Line::new(keys::command::model::SWITCHED, &[("alias", alias)]) })
        }
    }
}

/// `/effort low|medium|high`.
pub struct EffortCommand {
    id: EntityId,
    descriptor: CommandDescriptor,
    selection: Arc<RwLock<ModelSelection>>,
}

impl core::fmt::Debug for EffortCommand {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("EffortCommand").finish_non_exhaustive()
    }
}

impl EffortCommand {
    /// The command over the shared selection.
    #[must_use]
    pub fn new(selection: Arc<RwLock<ModelSelection>>) -> Self {
        Self {
            id: EntityId::new("effort"),
            descriptor: CommandDescriptor {
                name: "effort".into(),
                description: CommandDescription::Key(keys::command::effort::DESCRIPTION),
                roles: BTreeSet::from([Role::Admin, Role::Assignee, Role::Reporter]),
            },
            selection,
        }
    }
}

impl Entity for EffortCommand {
    fn kind(&self) -> EntityKind {
        EntityKind::Command
    }
    fn entity_id(&self) -> &EntityId {
        &self.id
    }
}

harness_async_trait! {
    impl Command for EffortCommand {
        fn descriptor(&self) -> &CommandDescriptor {
            &self.descriptor
        }
        async fn run(&self, call: &CommandCall) -> Result<CommandResult, HarnessError> {
            match call.args.parse::<Effort>() {
                Ok(effort) => {
                    self.selection.write().unwrap_or_else(std::sync::PoisonError::into_inner).effort = Some(effort);
                    Ok(CommandResult::Done { line: Line::new(keys::command::effort::SWITCHED, &[("effort", effort.to_string())]) })
                }
                Err(_) => Ok(CommandResult::Failed { line: Line::new(keys::command::effort::UNKNOWN, &[("effort", call.args.clone())]) }),
            }
        }
    }
}

/// The role check, as a guard on `command.invoke` (abuse case 10). Resolves the command
/// through the hook context's registry, so roles come from the task, never from text.
#[derive(Debug, Default)]
pub struct RoleGuard;

harness_async_trait! {
    impl GuardHook<CommandInvoke> for RoleGuard {
        async fn guard(&self, cx: &HookCx, call: &CommandCall) -> Result<GuardOutcome, HookError> {
            let Ok(command) = cx.registry.latest::<dyn Command>(&call.name) else {
                return Ok(GuardOutcome::Allow); // unknown commands are answered, not denied
            };
            let allowed = &command.descriptor().roles;
            if allowed.is_empty() || call.caller.roles.iter().any(|r| allowed.contains(r)) {
                Ok(GuardOutcome::Allow)
            } else {
                Ok(GuardOutcome::Deny(Veto { reason: keys::command::DENIED.as_str().to_owned(), detail: Some(call.name.clone()), by: EntityId::new("commands.role-guard") }))
            }
        }
    }
}

/// A command a plugin declared (R5.5, R10.3): its body runs on the plugin's JSON-RPC
/// endpoint (method `command.<name>`) or as a tool on one of its MCP servers
/// (`command_<name>`), through the same transport as remote hooks.
pub struct RemoteCommand {
    id: EntityId,
    descriptor: CommandDescriptor,
    transport: RemoteTransport,
}

impl core::fmt::Debug for RemoteCommand {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RemoteCommand")
            .field("id", &self.id)
            .field("descriptor", &self.descriptor)
            .field("transport", &self.transport)
            .finish()
    }
}

/// What a remote command receives.
#[derive(Serialize)]
struct RemoteCommandParams<'a> {
    name: &'a str,
    args: &'a str,
    caller: &'a Caller,
}

impl RemoteCommand {
    /// The command for a plugin's declaration, bound to the transport the plugin's
    /// handler resolved to.
    ///
    /// # Errors
    ///
    /// [`HarnessError::Internal`] when the declaration names a role that does not exist.
    pub fn from_declared(
        declared: &DeclaredCommand,
        transport: RemoteTransport,
    ) -> Result<Self, HarnessError> {
        let roles = declared
            .spec
            .roles
            .iter()
            .map(|r| {
                Role::from_name(r).ok_or_else(|| {
                    HarnessError::Internal(format!(
                        "plugin {}: command /{}: unknown role {r}",
                        declared.plugin, declared.spec.name
                    ))
                })
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        Ok(Self {
            id: declared.entity_id().clone(),
            descriptor: CommandDescriptor {
                name: declared.spec.name.clone(),
                description: CommandDescription::Plugin(declared.spec.description.clone()),
                roles,
            },
            transport,
        })
    }
}

impl Entity for RemoteCommand {
    fn kind(&self) -> EntityKind {
        EntityKind::Command
    }
    fn entity_id(&self) -> &EntityId {
        &self.id
    }
}

harness_async_trait! {
    impl Command for RemoteCommand {
        fn descriptor(&self) -> &CommandDescriptor {
            &self.descriptor
        }
        async fn run(&self, call: &CommandCall) -> Result<CommandResult, HarnessError> {
            let name = &self.descriptor.name;
            let params = RemoteCommandParams { name, args: &call.args, caller: &call.caller };
            call_remote(&self.transport, &format!("command.{name}"), &format!("command_{}", name.replace('-', "_")), params)
                .await
                .map_err(|source| HarnessError::Internal(format!("command /{name}: {source}")))
        }
    }
}

/// Registers the built-ins and the role guard over `registry` (R10.2).
///
/// # Errors
///
/// A registry conflict when a built-in's name is already taken.
pub fn install_builtins(
    registry: &Registry,
    dispatcher: &HookDispatcher,
    router: &Arc<ModelRouter>,
) -> Result<(), HarnessError> {
    let reg = |cmd: Arc<dyn Command>| {
        registry
            .register(cmd)
            .map_err(|e| HarnessError::Internal(e.to_string()))
    };
    reg(Arc::new(ExitCommand::default()))?;
    reg(Arc::new(ModelCommand::new(Arc::clone(router))))?;
    reg(Arc::new(EffortCommand::new(router.shared())))?;
    dispatcher.register::<CommandInvoke>(
        EntityId::new("commands.role-guard"),
        Hook::Guard(Arc::new(RoleGuard)),
        HookOrder::default(),
    );
    Ok(())
}

/// What a person's command message comes to (R10.1, R10.6): the `commands/v1` outcome
/// (`done`, `exit`, `unknown` or `failed`) and the line to show. A role veto (abuse 10) is
/// `failed` with the denied line, not an error.
///
/// # Errors
///
/// A hook failure other than a veto.
pub async fn answer(
    dispatcher: &HookDispatcher,
    cx: &HookCx,
    call: CommandCall,
) -> Result<(&'static str, Line), HarnessError> {
    let name = format!("/{}", call.name);
    let role = if call.caller.roles.iter().all(|r| *r == Role::Watcher) {
        "watcher"
    } else {
        "other"
    };
    match invoke(dispatcher, cx, call).await {
        Ok(CommandResult::Done { line }) => Ok(("done", line)),
        Ok(CommandResult::Exit { line }) => Ok(("exit", line)),
        Ok(CommandResult::Unknown { line }) => Ok(("unknown", line)),
        Ok(CommandResult::Failed { line }) => Ok(("failed", line)),
        Err(HarnessError::Vetoed(_)) => Ok((
            "failed",
            Line::new(
                keys::command::DENIED,
                &[("role", role.to_owned()), ("name", name)],
            ),
        )),
        Err(e) => Err(e),
    }
}

/// Runs a parsed command through the dispatcher (R10.1, R10.4, R10.6).
///
/// # Errors
///
/// A veto (the role guard, abuse 10) or a hook failure; an unknown name is not an error
/// but [`CommandResult::Unknown`].
pub async fn invoke(
    dispatcher: &HookDispatcher,
    cx: &HookCx,
    call: CommandCall,
) -> Result<CommandResult, HarnessError> {
    let registry = Arc::clone(&cx.registry);
    dispatcher
        .run::<CommandInvoke, _, _>(cx, call, move |call| async move {
            if let Ok(command) = registry.latest::<dyn Command>(&call.name) {
                return command.run(&call).await;
            }
            let names: Vec<String> = registry
                .list::<dyn Command>()
                .iter()
                .map(|c| c.descriptor().name.clone())
                .collect();
            let suggestion = closest(&call.name, &names);
            let name = format!("/{}", call.name);
            Ok(CommandResult::Unknown {
                line: match suggestion {
                    Some(s) => Line::new(
                        keys::command::UNKNOWN,
                        &[("name", name), ("suggestion", format!("/{s}"))],
                    ),
                    None => Line::new(keys::command::UNKNOWN_NO_SUGGESTION, &[("name", name)]),
                },
            })
        })
        .await
}

/// The registered name within edit distance 2 of `name`, if any.
fn closest(name: &str, names: &[String]) -> Option<String> {
    names
        .iter()
        .map(|n| (levenshtein(name, n), n))
        .filter(|(d, _)| *d <= 2)
        .min_by_key(|(d, _)| *d)
        .map(|(_, n)| n.clone())
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur.push((prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost));
        }
        prev = cur;
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::{
        Command, CommandCall, CommandParser, CommandResult, ModelRouter, ModelSelection,
        install_builtins, invoke,
    };
    use crate::assets::Catalog;
    use crate::harness::core_loop::task::{Caller, Role};
    use crate::harness::hooks::{HookCx, HookDispatcher};
    use crate::harness::models::llm::testing::RecordedLlm;
    use crate::harness::models::llm::{Effort, Llm, MessageRole};
    use crate::host::Host;
    use crate::registry::EntityId;
    use crate::registry::Registry;
    use std::sync::Arc;

    fn harness() -> (Arc<Registry>, HookDispatcher, Arc<ModelRouter>, HookCx) {
        let registry = Arc::new(Registry::new());
        let gpt: Arc<dyn Llm> = Arc::new(RecordedLlm::new("gpt", vec![]));
        let claude: Arc<dyn Llm> = Arc::new(RecordedLlm::new("claude", vec![]));
        registry.register(gpt).unwrap();
        registry.register(claude).unwrap();
        let router = Arc::new(ModelRouter::new(
            Arc::clone(&registry),
            ModelSelection {
                model: "gpt".into(),
                effort: None,
            },
        ));
        let dispatcher = HookDispatcher::new();
        install_builtins(&registry, &dispatcher, &router).unwrap();
        let cx = HookCx::new(Arc::clone(&registry), Arc::new(Host::testing()));
        (registry, dispatcher, router, cx)
    }

    #[test]
    fn commands_are_parsed_from_people_only() {
        assert_eq!(
            CommandParser::parse(MessageRole::User, true, "/model gpt-6.1"),
            Some(("model".into(), "gpt-6.1".into()))
        );
        assert_eq!(
            CommandParser::parse(MessageRole::User, true, "  /exit"),
            Some(("exit".into(), String::new()))
        );
        assert_eq!(
            CommandParser::parse(MessageRole::User, true, "not a command"),
            None
        );
        assert_eq!(CommandParser::parse(MessageRole::User, true, "/"), None);
        assert_eq!(
            CommandParser::parse(MessageRole::User, false, "/exit"),
            None,
            "an agent's message"
        );
    }

    #[tokio::test]
    async fn abuse_command_in_tool_result_is_not_parsed() {
        // Boundary 4: a tool result that looks like a command never becomes one.
        assert_eq!(
            CommandParser::parse(MessageRole::Tool, true, "/model attacker"),
            None
        );
        assert_eq!(
            CommandParser::parse(MessageRole::Assistant, true, "/exit"),
            None
        );
    }

    /// Feature: Commands
    /// Requirement: docs/specs/issue-3/requirements.md#R9
    ///
    /// Scenario: Switching /model applies to the next call without a restart
    ///     Given a router on "gpt" and an admin on the task
    ///     When the admin runs `/model claude`
    ///     Then the next model the router resolves is "claude", and the task was not restarted
    #[tokio::test]
    async fn switching_model_applies_to_the_next_call_without_a_restart() {
        let (_, dispatcher, router, cx) = harness();
        assert_eq!(router.current().unwrap().entity_id().id.as_ref(), "gpt");
        let admin = Caller::human("u1", [Role::Admin]);
        let result = invoke(
            &dispatcher,
            &cx,
            CommandCall {
                name: "model".into(),
                args: "claude".into(),
                caller: admin.clone(),
            },
        )
        .await
        .unwrap();
        assert!(
            matches!(result, CommandResult::Done { ref line } if line.key == "command.model.switched")
        );
        assert_eq!(
            router.current().unwrap().entity_id().id.as_ref(),
            "claude",
            "the next call goes to the new model"
        );
        let result = invoke(
            &dispatcher,
            &cx,
            CommandCall {
                name: "effort".into(),
                args: "high".into(),
                caller: admin.clone(),
            },
        )
        .await
        .unwrap();
        assert!(matches!(result, CommandResult::Done { .. }));
        assert_eq!(router.selection().effort, Some(Effort::High));
        let listing = invoke(
            &dispatcher,
            &cx,
            CommandCall {
                name: "model".into(),
                args: String::new(),
                caller: admin.clone(),
            },
        )
        .await
        .unwrap();
        let CommandResult::Done { line } = listing else {
            panic!("listing")
        };
        assert!(
            line.render(&Catalog::builtin(), "en")
                .contains("Current: claude")
        );
        let bad = invoke(
            &dispatcher,
            &cx,
            CommandCall {
                name: "model".into(),
                args: "nope".into(),
                caller: admin,
            },
        )
        .await
        .unwrap();
        assert!(matches!(bad, CommandResult::Failed { .. }));
    }

    #[tokio::test]
    async fn an_unknown_command_is_answered_with_a_suggestion_and_never_sent_on() {
        let (_, dispatcher, _, cx) = harness();
        let result = invoke(
            &dispatcher,
            &cx,
            CommandCall {
                name: "modle".into(),
                args: String::new(),
                caller: Caller::human("u1", [Role::Reporter]),
            },
        )
        .await
        .unwrap();
        let CommandResult::Unknown { line } = result else {
            panic!("unknown")
        };
        assert_eq!(
            line.render(&Catalog::builtin(), "en"),
            "Unknown command /modle. Did you mean /model?"
        );
    }

    #[tokio::test]
    async fn abuse_watcher_invoking_model_command_is_refused() {
        let (_, dispatcher, router, cx) = harness();
        let err = invoke(
            &dispatcher,
            &cx,
            CommandCall {
                name: "model".into(),
                args: "claude".into(),
                caller: Caller::human("w", [Role::Watcher]),
            },
        )
        .await
        .expect_err("denied");
        assert!(matches!(err, crate::HarnessError::Vetoed(v) if v.reason == "command.denied"));
        assert_eq!(router.selection().model, "gpt", "nothing changed");
        let exit = invoke(
            &dispatcher,
            &cx,
            CommandCall {
                name: "exit".into(),
                args: String::new(),
                caller: Caller::human("w", [Role::Watcher]),
            },
        )
        .await
        .unwrap();
        assert!(
            matches!(exit, CommandResult::Exit { .. }),
            "exit is open to every participant"
        );
    }

    #[tokio::test]
    async fn a_plugin_declared_command_runs_on_its_mcp_server_with_the_caller_attached() {
        use crate::harness::hooks::remote::{McpToolCaller, RemoteTransport};
        use crate::harness::plugins::{CommandSpec, DeclaredCommand, RemoteHookTransport};
        struct Echo(std::sync::Mutex<Vec<(String, serde_json::Value)>>);
        impl McpToolCaller for Echo {
            fn call_tool(
                &self,
                tool: &str,
                arguments: serde_json::Value,
            ) -> crate::runtime::HarnessFuture<
                '_,
                Result<serde_json::Value, crate::harness::hooks::HookError>,
            > {
                self.0.lock().unwrap().push((tool.to_owned(), arguments));
                Box::pin(async {
                    Ok(
                        serde_json::json!({"outcome": "done", "line": {"key": "command.model.switched", "args": [["alias", "plan shown"]]}}),
                    )
                })
            }
        }
        let echo = Arc::new(Echo(std::sync::Mutex::new(Vec::new())));
        let declared = DeclaredCommand::new(
            EntityId::versioned("show-plan", "1.0.0".parse().unwrap()),
            CommandSpec {
                name: "show-plan".into(),
                description: "Show the plan.".into(),
                roles: vec!["admin".into()],
                handler: RemoteHookTransport::Mcp {
                    server: "echo".into(),
                },
            },
            "demo",
        );
        let command = super::RemoteCommand::from_declared(
            &declared,
            RemoteTransport::McpTool {
                server: EntityId::new("demo/echo"),
                caller: echo.clone(),
            },
        )
        .unwrap();
        assert_eq!(
            command
                .descriptor()
                .description
                .render(&Catalog::builtin(), "en"),
            "Show the plan."
        );
        let (registry, dispatcher, _, cx) = harness();
        registry.register::<dyn Command>(Arc::new(command)).unwrap();
        let denied = invoke(
            &dispatcher,
            &cx,
            CommandCall {
                name: "show-plan".into(),
                args: String::new(),
                caller: Caller::human("w", [Role::Watcher]),
            },
        )
        .await;
        assert!(
            matches!(denied, Err(crate::HarnessError::Vetoed(_))),
            "plugin roles are enforced by the same guard"
        );
        let ok = invoke(
            &dispatcher,
            &cx,
            CommandCall {
                name: "show-plan".into(),
                args: "now".into(),
                caller: Caller::human("a", [Role::Admin]),
            },
        )
        .await
        .unwrap();
        assert!(matches!(ok, CommandResult::Done { ref line } if line.args[0].1 == "plan shown"));
        let calls = echo.0.lock().unwrap();
        assert_eq!(calls[0].0, "command_show_plan");
        assert_eq!(calls[0].1["caller"]["roles"][0], "admin");
        assert_eq!(calls[0].1["args"], "now");

        let mut bad = declared.clone();
        bad.spec.roles = vec!["root".into()];
        assert!(
            super::RemoteCommand::from_declared(
                &bad,
                RemoteTransport::McpTool {
                    server: EntityId::new("demo/echo"),
                    caller: echo.clone()
                }
            )
            .is_err()
        );
    }
}
