//! Default system prompt assembly from registered fragments (R7).
//!
//! The system prompt is a [`PromptPlan`]: an ordered list of fragment ids with their
//! arguments. The default plan is `system/base` (who the agent is) and
//! `system/participants` (the roles on the task); a configured prompt replaces the base
//! (`[agent] system_prompt = "file:…"` or a registered fragment id); extensions are
//! appended at the `SystemExtensions` position. `prompt.assemble` hooks see the plan
//! before it is rendered, so a hook can rewrite it (R7.3). Nothing here is a string
//! literal: every text is a fragment (R7.4).

use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::HarnessError;
use crate::assets::{Prompt, PromptArgs, PromptError, PromptFragment, PromptRegistry};
use crate::config::AgentConfig;
use crate::harness::core_loop::task::Participant;
use crate::harness::hooks::{HookCx, HookDispatcher, HookPoint, Operation};
use crate::host::FileSystem;
use crate::registry::Entity;

/// One fragment to render, with its arguments.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedFragment {
    /// The fragment id (`system/base`).
    pub id: String,
    /// The arguments, by placeholder name.
    pub args: Vec<(String, String)>,
}

/// The ordered fragments of a system prompt, before rendering.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptPlan {
    /// The base prompt and its companions, in order.
    pub base: Vec<PlannedFragment>,
    /// Extensions appended after the base (R7.2).
    pub extensions: Vec<PlannedFragment>,
    /// The locale to render in.
    pub locale: String,
}

/// The `prompt.assemble` operation (R7.3): the plan goes in, the rendered prompt comes out.
#[derive(Debug)]
pub struct PromptAssemble;

impl Operation for PromptAssemble {
    const POINT: HookPoint = HookPoint::PromptAssemble;
    type Input = PromptPlan;
    type Output = Prompt;
}

/// Builds the plan for an agent and renders it through the hooks.
pub struct SystemPrompt {
    prompts: Arc<PromptRegistry>,
    agent: AgentConfig,
    locale: String,
}

impl core::fmt::Debug for SystemPrompt {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SystemPrompt")
            .field("agent", &self.agent.name)
            .field("locale", &self.locale)
            .finish_non_exhaustive()
    }
}

/// Where a configured replacement prompt comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Replacement {
    Fragment(String),
    File(String),
}

fn parse_replacement(value: &str) -> Replacement {
    value.strip_prefix("file:").map_or_else(
        || Replacement::Fragment(value.to_owned()),
        |p| Replacement::File(p.to_owned()),
    )
}

impl SystemPrompt {
    /// The assembler for `[agent]`, rendering in `locale`.
    #[must_use]
    pub fn new(
        prompts: Arc<PromptRegistry>,
        agent: AgentConfig,
        locale: impl Into<String>,
    ) -> Self {
        Self {
            prompts,
            agent,
            locale: locale.into(),
        }
    }

    /// Registers a replacement prompt given as `file:<path>` by reading it through the
    /// host, so the plan can name it as a fragment (R7.2). A registered id needs nothing.
    ///
    /// # Errors
    ///
    /// [`HarnessError::Internal`] when the file cannot be read or does not parse as a
    /// fragment (front matter with `id`, `version`, `placeholders`).
    pub async fn load_replacement(&mut self, fs: &dyn FileSystem) -> Result<(), HarnessError> {
        let Some(value) = &self.agent.system_prompt else {
            return Ok(());
        };
        if let Replacement::File(path) = parse_replacement(value) {
            let text = fs
                .read(Path::new(&path))
                .await
                .map_err(|e| HarnessError::Internal(format!("system prompt {path}: {e}")))?;
            let text = String::from_utf8(text.to_vec())
                .map_err(|_| HarnessError::Internal(format!("system prompt {path}: not UTF-8")))?;
            let fragment = PromptFragment::parse(&path, &text)
                .map_err(|e| HarnessError::Internal(e.to_string()))?;
            let id = fragment.entity_id().id.to_string();
            Arc::get_mut(&mut self.prompts)
                .ok_or_else(|| {
                    HarnessError::Internal(
                        "prompt registry is shared; load the replacement before sharing it".into(),
                    )
                })?
                .add(fragment);
            self.agent.system_prompt = Some(id);
        }
        Ok(())
    }

    /// The plan for a task with `participants` and the configured `extensions`
    /// (fragment ids with no arguments).
    #[must_use]
    pub fn plan(&self, participants: &[Participant], extensions: &[String]) -> PromptPlan {
        let base_id = self
            .agent
            .system_prompt
            .clone()
            .unwrap_or_else(|| "system/base".to_owned());
        let base = PlannedFragment {
            id: base_id,
            args: vec![
                ("agent_name".into(), self.agent.name.clone()),
                ("agent_description".into(), self.agent.description.clone()),
            ],
        };
        let roles = participants
            .iter()
            .map(|p| {
                format!(
                    "- {} ({}): {}",
                    p.display,
                    p.id,
                    p.roles
                        .iter()
                        .map(|r| r.name())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let participants = PlannedFragment {
            id: "system/participants".into(),
            args: vec![("participants".into(), roles)],
        };
        PromptPlan {
            base: vec![base, participants],
            extensions: extensions
                .iter()
                .map(|id| PlannedFragment {
                    id: id.clone(),
                    args: Vec::new(),
                })
                .collect(),
            locale: self.locale.clone(),
        }
    }

    /// Renders a plan: every fragment resolved in the plan's locale, base then extensions.
    ///
    /// # Errors
    ///
    /// [`PromptError`] for a missing fragment or a placeholder mismatch.
    pub fn render(&self, plan: &PromptPlan) -> Result<Prompt, PromptError> {
        let mut prompt = Prompt::default();
        for planned in plan.base.iter().chain(&plan.extensions) {
            let fragment = self.prompts.get(&planned.id, &plan.locale).ok_or_else(|| {
                PromptError::MissingPlaceholder {
                    fragment: crate::registry::EntityId::new(planned.id.clone()),
                    placeholder: "<fragment not registered>".into(),
                }
            })?;
            let mut args = PromptArgs::new();
            for (k, v) in &planned.args {
                args = args.with(k, v.clone());
            }
            prompt = prompt.then(Prompt::fragment(&fragment, &args)?);
        }
        Ok(prompt)
    }

    /// Plans, runs the `prompt.assemble` hooks on the plan, and renders (R7.3).
    ///
    /// # Errors
    ///
    /// A veto, a hook failure, or a rendering error.
    pub async fn assemble(
        &self,
        dispatcher: &HookDispatcher,
        cx: &HookCx,
        participants: &[Participant],
        extensions: &[String],
    ) -> Result<Prompt, HarnessError> {
        let plan = self.plan(participants, extensions);
        let this = self.snapshot();
        dispatcher
            .run::<PromptAssemble, _, _>(cx, plan, move |plan| async move {
                this.render(&plan)
                    .map_err(|e| HarnessError::Internal(e.to_string()))
            })
            .await
    }

    fn snapshot(&self) -> SystemPrompt {
        SystemPrompt {
            prompts: Arc::clone(&self.prompts),
            agent: self.agent.clone(),
            locale: self.locale.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PlannedFragment, PromptAssemble, SystemPrompt};
    use crate::assets::PromptRegistry;
    use crate::config::AgentConfig;
    use crate::harness::core_loop::task::{Participant, Role};
    use crate::harness::hooks::{
        Hook, HookCx, HookDispatcher, HookError, HookOrder, PreHook, PreOutcome,
    };
    use crate::harness_async_trait;
    use crate::host::Host;
    use crate::host::testing::MemFs;
    use crate::registry::{EntityId, Registry};
    use std::sync::Arc;

    fn agent() -> AgentConfig {
        AgentConfig {
            name: "demo-agent".into(),
            description: "a test agent".into(),
            system_prompt: None,
        }
    }

    fn people() -> Vec<Participant> {
        vec![
            Participant::human("u1", "sam", [Role::Reporter, Role::Admin]),
            Participant::human("u2", "ana", [Role::Watcher]),
        ]
    }

    #[test]
    fn the_default_plan_is_base_plus_participants_rendered_from_fragments() {
        let sp = SystemPrompt::new(Arc::new(PromptRegistry::builtin()), agent(), "en");
        let plan = sp.plan(&people(), &["system/commands".into()]);
        assert_eq!(
            plan.base.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(),
            ["system/base", "system/participants"]
        );
        let prompt = sp
            .render(&plan)
            .expect_err("the extension lacks its placeholder");
        assert!(prompt.to_string().contains("command_index"));
        let plan = sp.plan(&people(), &[]);
        let prompt = sp.render(&plan).expect("renders");
        let text = prompt.render();
        assert!(text.starts_with("You are demo-agent: a test agent"));
        assert!(
            text.contains("- sam (u1): reporter, admin") && text.contains("- ana (u2): watcher")
        );
        assert_eq!(
            prompt
                .provenance()
                .iter()
                .map(|id| id.id.as_ref())
                .collect::<Vec<_>>(),
            ["system/base", "system/participants"]
        );
    }

    #[tokio::test]
    async fn a_configured_file_replaces_the_base_and_hooks_can_rewrite_the_plan() {
        struct DropParticipants;
        harness_async_trait! {
            impl PreHook<PromptAssemble> for DropParticipants {
                async fn pre(&self, _cx: &HookCx, mut plan: super::PromptPlan) -> Result<PreOutcome<super::PromptPlan>, HookError> {
                    plan.base.retain(|f: &PlannedFragment| f.id != "system/participants");
                    Ok(PreOutcome::Continue(plan))
                }
            }
        }
        let fs = MemFs::new([(
            "/etc/mine.md".into(),
            bytes::Bytes::from_static(b"---\nid: /etc/mine\nversion: 0.1.0\nplaceholders: [agent_name, agent_description]\ndescription: mine\n---\nI am {{agent_name}}, {{agent_description}}, and I speak like a pirate.\n"),
        )]);
        let mut cfg = agent();
        cfg.system_prompt = Some("file:/etc/mine.md".into());
        let mut sp = SystemPrompt::new(Arc::new(PromptRegistry::builtin()), cfg, "en");
        sp.load_replacement(&fs).await.expect("loads");
        let plan = sp.plan(&people(), &[]);
        assert_eq!(plan.base[0].id, "/etc/mine");
        assert!(sp.render(&plan).unwrap().render().contains("pirate"));

        let d = HookDispatcher::new();
        d.register::<PromptAssemble>(
            EntityId::new("drop"),
            Hook::Pre(Arc::new(DropParticipants)),
            HookOrder::default(),
        );
        let cx = HookCx::new(Arc::new(Registry::new()), Arc::new(Host::testing()));
        let prompt = sp
            .assemble(&d, &cx, &people(), &[])
            .await
            .expect("assembles");
        assert!(
            !prompt.render().contains("sam"),
            "the hook rewrote the plan before rendering"
        );
    }
}
