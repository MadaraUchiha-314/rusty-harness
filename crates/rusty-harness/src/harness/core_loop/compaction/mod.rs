//! Compaction: policy, never-compact positions, the summariser (R16).
//!
//! When the estimate exceeds the configured fraction of the model's window (0.75), the
//! [`Compactor`] summarises the oldest turns of the history, keeping the most recent
//! `keep_recent_turns`, every pinned turn and every position of the template's
//! never-compact set verbatim (R16.2). The summary replaces the span as one
//! [`HistoryEntry::Summary`] and is returned for the loop to persist with the task's
//! state (R16.4). `context.compact` hooks see the plan and may change it or veto (R16.3).

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::HarnessError;
use crate::assets::{Prompt, PromptArgs, PromptRegistry};
use crate::config::CompactionConfig;
use crate::harness::core_loop::context_window::{
    ContextTemplate, HistoryEntry, TokenEstimate, TokenRatio, Turn,
};
use crate::harness::hooks::{HookCx, HookDispatcher, HookPoint, Operation};
use crate::harness::models::llm::{Llm, ModelRequest, collect};

/// What compaction intends to do; hooks may edit it (R16.3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompactionPlan {
    /// The task.
    pub task: String,
    /// Indexes into the history that will be summarised, ascending.
    pub summarise: Vec<usize>,
    /// Indexes kept verbatim because they are recent or pinned.
    pub keep: Vec<usize>,
    /// The estimate that triggered it.
    pub before: u32,
    /// The window the estimate was measured against.
    pub window: u32,
}

/// What compaction did (R16.4): for the trace and the persisted summary.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Compacted {
    /// The history after compaction.
    pub history: Vec<Turn>,
    /// The summary text, to persist with the state.
    pub summary: Option<String>,
    /// How many turns were replaced.
    pub replaced: usize,
    /// Tokens before.
    pub tokens_before: u32,
    /// Tokens after (estimated).
    pub tokens_after: u32,
}

/// The `context.compact` operation (R16.3): the plan goes in, the compacted history
/// comes out; `pre` hooks may edit the plan or veto.
#[derive(Debug)]
pub struct ContextCompact;

impl Operation for ContextCompact {
    const POINT: HookPoint = HookPoint::ContextCompact;
    type Input = CompactionPlan;
    type Output = Compacted;
    fn identity(input: &CompactionPlan) -> String {
        input.task.clone()
    }
    fn attributes(_input: &CompactionPlan, output: &Compacted) -> Vec<(&'static str, String)> {
        vec![
            ("compaction.replaced", output.replaced.to_string()),
            ("tokens.before", output.tokens_before.to_string()),
            ("tokens.after", output.tokens_after.to_string()),
        ]
    }
}

/// The characters the summarised turns would have cost.
fn removed_chars(plan: &CompactionPlan, history: &[Turn]) -> usize {
    plan.summarise
        .iter()
        .filter_map(|&i| history.get(i))
        .flat_map(|t| t.message().parts)
        .map(|p| match p {
            crate::harness::models::llm::ContentPart::Text { text } => text.len(),
            _ => 0,
        })
        .sum()
}

/// The span to summarise, one line per turn.
fn render_span(plan: &CompactionPlan, history: &[Turn]) -> String {
    plan.summarise
        .iter()
        .filter_map(|&i| history.get(i))
        .map(|t| match &t.entry {
            HistoryEntry::User { text } => format!("user: {text}"),
            HistoryEntry::Assistant { text, tool_calls } => {
                let called = if tool_calls.is_empty() {
                    String::new()
                } else {
                    format!(
                        " [called {}]",
                        tool_calls
                            .iter()
                            .map(|c| c.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                };
                format!("assistant: {text}{called}")
            }
            HistoryEntry::ToolResult { call_id, text } => format!("tool {call_id}: {text}"),
            HistoryEntry::SubAgentReply { task, text } => format!("sub-agent {task}: {text}"),
            HistoryEntry::ChannelReply { ask, text } => format!("reply to {ask}: {text}"),
            HistoryEntry::Summary { text, .. } => format!("earlier summary: {text}"),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Decides when to compact and does it.
pub struct Compactor {
    config: CompactionConfig,
    template: ContextTemplate,
    prompts: Arc<PromptRegistry>,
    summariser: Arc<dyn Llm>,
}

impl core::fmt::Debug for Compactor {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Compactor")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl Compactor {
    /// A compactor summarising with `summariser` (the active model, or the configured one).
    #[must_use]
    pub fn new(
        config: CompactionConfig,
        template: ContextTemplate,
        prompts: Arc<PromptRegistry>,
        summariser: Arc<dyn Llm>,
    ) -> Self {
        Self {
            config,
            template,
            prompts,
            summariser,
        }
    }

    /// Whether `estimate` is above the threshold for `window` (R16.1).
    #[must_use]
    pub fn needed(&self, estimate: &TokenEstimate, window: u32) -> bool {
        f64::from(estimate.total) > self.config.threshold * f64::from(window)
    }

    /// The plan for `history`: everything but the recent and pinned turns is summarised.
    /// The never-compact positions are not part of the history, so they are untouched by
    /// construction; the template is consulted so a hook can read what is protected.
    #[must_use]
    pub fn plan(
        &self,
        task: &str,
        history: &[Turn],
        estimate: &TokenEstimate,
        window: u32,
    ) -> CompactionPlan {
        let recent_from = history
            .len()
            .saturating_sub(self.config.keep_recent_turns as usize);
        let (keep, summarise): (Vec<usize>, Vec<usize>) =
            (0..history.len()).partition(|&i| i >= recent_from || history[i].pinned);
        debug_assert!(
            self.template
                .never_compact
                .contains(&crate::harness::core_loop::context_window::Position::SystemPrompt)
        );
        CompactionPlan {
            task: task.to_owned(),
            summarise,
            keep,
            before: estimate.total,
            window,
        }
    }

    /// Summarises the planned span with `prompts/compaction/summarise.md` and splices
    /// the result in as one `Summary` turn at the span's start (R16.2, R16.4).
    ///
    /// # Errors
    ///
    /// A model failure.
    pub async fn apply(
        &self,
        plan: &CompactionPlan,
        history: &[Turn],
        goal: &str,
        ratio: &TokenRatio,
    ) -> Result<Compacted, HarnessError> {
        if plan.summarise.is_empty() {
            return Ok(Compacted {
                history: history.to_vec(),
                summary: None,
                replaced: 0,
                tokens_before: plan.before,
                tokens_after: plan.before,
            });
        }
        let span = render_span(plan, history);
        let fragment = self
            .prompts
            .get("compaction/summarise", "en")
            .ok_or_else(|| {
                HarnessError::Internal("compaction/summarise fragment missing".into())
            })?;
        let args = PromptArgs::new()
            .with("task_goal", goal)
            .with("span", span.as_str());
        let instructions = Prompt::fragment(&fragment, &args)
            .map_err(|e| HarnessError::Internal(e.to_string()))?;
        let request = ModelRequest::new(instructions, Vec::new());
        let events = self
            .summariser
            .stream(request)
            .await
            .map_err(|e| HarnessError::Internal(e.to_string()))?;
        let turn = collect(events, None)
            .await
            .map_err(|e| HarnessError::Internal(e.to_string()))?;
        let summary = turn.text.trim().to_owned();
        let mut out: Vec<Turn> = Vec::with_capacity(plan.keep.len() + 1);
        let mut inserted = false;
        for (i, t) in history.iter().enumerate() {
            if plan.summarise.contains(&i) {
                if !inserted {
                    out.push(Turn::new(HistoryEntry::Summary {
                        text: summary.clone(),
                        replaced: plan.summarise.len(),
                    }));
                    inserted = true;
                }
                continue;
            }
            out.push(t.clone());
        }
        let tokens_after = plan
            .before
            .saturating_sub(ratio.tokens(removed_chars(plan, history)))
            .saturating_add(ratio.tokens(summary.len()));
        Ok(Compacted {
            history: out,
            summary: Some(summary),
            replaced: plan.summarise.len(),
            tokens_before: plan.before,
            tokens_after,
        })
    }

    /// Plans, runs the `context.compact` hooks, and applies (R16.3).
    ///
    /// # Errors
    ///
    /// A veto, a hook failure or a model failure.
    pub async fn compact(
        &self,
        dispatcher: &HookDispatcher,
        cx: &HookCx,
        request: CompactionRequest<'_>,
    ) -> Result<Compacted, HarnessError> {
        let plan = self.plan(
            request.task,
            request.history,
            request.estimate,
            request.window,
        );
        let history = request.history.to_vec();
        let goal = request.goal.to_owned();
        let ratio = request.ratio.clone();
        dispatcher
            .run::<ContextCompact, _, _>(cx, plan, |plan| async move {
                self.apply(&plan, &history, &goal, &ratio).await
            })
            .await
    }
}

/// What one compaction needs from the loop.
#[derive(Clone, Debug)]
pub struct CompactionRequest<'a> {
    /// The task.
    pub task: &'a str,
    /// Its goal, for the summariser.
    pub goal: &'a str,
    /// The conversation.
    pub history: &'a [Turn],
    /// The estimate that triggered compaction.
    pub estimate: &'a TokenEstimate,
    /// The model's window.
    pub window: u32,
    /// The learned ratio, for the after-count.
    pub ratio: &'a TokenRatio,
}

#[cfg(test)]
mod tests {
    use super::{CompactionPlan, CompactionRequest, Compactor, ContextCompact};
    use crate::assets::PromptRegistry;
    use crate::config::CompactionConfig;
    use crate::harness::core_loop::context_window::{
        ContextTemplate, EstimateSource, HistoryEntry, TokenEstimate, TokenRatio, Turn,
    };
    use crate::harness::hooks::{
        Hook, HookCx, HookDispatcher, HookError, HookOrder, PreHook, PreOutcome, Veto,
    };
    use crate::harness::models::llm::testing::RecordedLlm;
    use crate::harness::models::llm::{ModelEvent, StopReason};
    use crate::harness_async_trait;
    use crate::host::Host;
    use crate::registry::{EntityId, Registry};
    use std::sync::Arc;

    fn history(n: usize) -> Vec<Turn> {
        (0..n)
            .map(|i| {
                if i % 2 == 0 {
                    Turn::new(HistoryEntry::User {
                        text: format!("question {i} about the invoice INV-{i}"),
                    })
                } else {
                    Turn::new(HistoryEntry::Assistant {
                        text: format!("answer {i}"),
                        tool_calls: vec![],
                    })
                }
            })
            .collect()
    }

    const SUMMARY: &str = "The user asked about invoices INV-0 to INV-5; all were answered.";

    fn summariser() -> Arc<RecordedLlm> {
        Arc::new(RecordedLlm::new(
            "sum",
            vec![vec![
                ModelEvent::TextDelta {
                    text: SUMMARY.into(),
                },
                ModelEvent::Stop {
                    reason: StopReason::EndTurn,
                },
            ]],
        ))
    }

    fn compactor(llm: Arc<RecordedLlm>, keep: u32) -> Compactor {
        let config = CompactionConfig {
            threshold: 0.75,
            keep_recent_turns: keep,
            model: None,
        };
        Compactor::new(
            config,
            ContextTemplate::default(),
            Arc::new(PromptRegistry::builtin()),
            llm,
        )
    }

    fn estimate(total: u32) -> TokenEstimate {
        TokenEstimate {
            total,
            by_position: std::collections::BTreeMap::new(),
            source: EstimateSource::Heuristic,
        }
    }

    fn cx() -> HookCx {
        HookCx::new(Arc::new(Registry::new()), Arc::new(Host::testing()))
    }

    #[test]
    fn compaction_triggers_above_the_threshold_and_plans_around_recent_and_pinned_turns() {
        let c = compactor(summariser(), 4);
        assert!(!c.needed(&estimate(7_500), 10_000));
        assert!(c.needed(&estimate(7_501), 10_000));
        let mut h = history(12);
        h[1].pinned = true;
        let plan = c.plan("t1", &h, &estimate(8_000), 10_000);
        assert_eq!(plan.keep, [1, 8, 9, 10, 11]);
        assert_eq!(plan.summarise, [0, 2, 3, 4, 5, 6, 7]);
        let short = c.plan("t1", &history(3), &estimate(8_000), 10_000);
        assert_eq!(short.summarise.len(), 0);
    }

    /// Scenario: Compaction keeps the pinned positions verbatim
    ///     Given eight turns, the fourth pinned by a hook, and a keep-recent count of two
    ///     When compaction runs
    ///     Then the pinned turn and the last two turns are unchanged, the rest is one summary
    ///     And the summariser was asked with the compaction fragment and the task goal
    #[tokio::test]
    async fn compaction_keeps_the_pinned_positions_verbatim() {
        let llm = summariser();
        let c = compactor(Arc::clone(&llm), 2);
        let mut h = history(8);
        h[3].pinned = true;
        let d = HookDispatcher::new();
        let out = c
            .compact(
                &d,
                &cx(),
                CompactionRequest {
                    task: "t1",
                    goal: "answer invoice questions",
                    history: &h,
                    estimate: &estimate(9_000),
                    window: 10_000,
                    ratio: &TokenRatio::default(),
                },
            )
            .await
            .unwrap();
        assert_eq!(out.replaced, 5);
        assert_eq!(out.history.len(), 4);
        assert!(
            matches!(&out.history[0].entry, HistoryEntry::Summary { text, replaced: 5 } if text == SUMMARY)
        );
        assert_eq!(out.history[1], h[3], "pinned, verbatim");
        assert_eq!(&out.history[2..], &h[6..], "the recent turns, verbatim");
        assert_eq!(out.summary.as_deref(), Some(SUMMARY));
        assert!(
            out.tokens_after < out.tokens_before,
            "{} < {}",
            out.tokens_after,
            out.tokens_before
        );
        let request = &llm.requests()[0];
        let text = request.instructions.render();
        assert!(
            text.contains("answer invoice questions")
                && text.contains("INV-4")
                && !text.contains("INV-6")
        );
        assert_eq!(
            request.instructions.provenance()[0].id.as_ref(),
            "compaction/summarise"
        );
        // Nothing to do when everything is recent or pinned.
        let none = c
            .compact(
                &d,
                &cx(),
                CompactionRequest {
                    task: "t1",
                    goal: "g",
                    history: &history(2),
                    estimate: &estimate(9_000),
                    window: 10_000,
                    ratio: &TokenRatio::default(),
                },
            )
            .await
            .unwrap();
        assert_eq!((none.replaced, none.summary), (0, None));
    }

    #[tokio::test]
    async fn context_compact_hooks_can_edit_the_plan_or_veto() {
        struct KeepFirst;
        harness_async_trait! {
            impl PreHook<ContextCompact> for KeepFirst {
                async fn pre(&self, _cx: &HookCx, mut plan: CompactionPlan) -> Result<PreOutcome<CompactionPlan>, HookError> {
                    if plan.before > 9_500 {
                        return Ok(PreOutcome::Veto(Veto { reason: "compaction.never".into(), detail: None, by: EntityId::new("keep-first") }));
                    }
                    plan.summarise.retain(|&i| i != 0);
                    plan.keep.insert(0, 0);
                    Ok(PreOutcome::Continue(plan))
                }
            }
        }
        let c = compactor(summariser(), 2);
        let d = HookDispatcher::new();
        d.register::<ContextCompact>(
            EntityId::new("keep-first"),
            Hook::Pre(Arc::new(KeepFirst)),
            HookOrder::default(),
        );
        let h = history(6);
        let out = c
            .compact(
                &d,
                &cx(),
                CompactionRequest {
                    task: "t1",
                    goal: "g",
                    history: &h,
                    estimate: &estimate(8_000),
                    window: 10_000,
                    ratio: &TokenRatio::default(),
                },
            )
            .await
            .unwrap();
        assert_eq!(out.history[0], h[0], "the hook kept the first turn");
        assert!(matches!(
            out.history[1].entry,
            HistoryEntry::Summary { replaced: 3, .. }
        ));
        let err = c
            .compact(
                &d,
                &cx(),
                CompactionRequest {
                    task: "t1",
                    goal: "g",
                    history: &h,
                    estimate: &estimate(9_900),
                    window: 10_000,
                    ratio: &TokenRatio::default(),
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, crate::HarnessError::Vetoed(v) if v.reason == "compaction.never"));
    }
}
