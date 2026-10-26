//! The model's hands on its plan (R13.3–4, design.md § Plan and Step): `plan.update` sets
//! the steps and their order, `plan.step_done` records one step's outcome behind the
//! `step.execute` hooks. Both persist through `plan.persist` and hand the plan to the
//! surfaces as an `application/vnd.rusty-harness.plan+json` artifact under `plan/v1`.

use std::sync::Arc;

use serde_json::{Value, json};

use super::{Plan, PlanError, Step, StepExecute, StepOutput, StepRecord, StepStatus};
use crate::harness::core_loop::persistence::{PersistenceStore, persist_plan};
use crate::harness::core_loop::task::{PLAN_EXT, TaskId};
use crate::harness::hooks::{HookCx, HookDispatcher};
use crate::harness::tools::{Idempotency, Tool, ToolCx, ToolDescriptor, ToolError, ToolOutput};
use crate::harness_async_trait;
use crate::registry::{Entity, EntityId, EntityKind, Registry};

/// The tool that sets the plan.
pub const UPDATE_TOOL: &str = "plan.update";
/// The tool that records a step's outcome.
pub const STEP_DONE_TOOL: &str = "plan.step_done";
/// The media type of the plan artifact.
pub const PLAN_MEDIA_TYPE: &str = "application/vnd.rusty-harness.plan+json";

/// The plan of `task`: one per task, so the id is derived from it.
#[must_use]
pub fn plan_id(task: &str) -> String {
    format!("plan-{task}")
}

/// The plan as the surfaces receive it: an artifact replaced on every edit (its id is the
/// plan's), the plan in a data part and under `plan/v1` in the metadata.
#[must_use]
pub fn plan_artifact(plan: &Plan) -> a2a::Artifact {
    let value = serde_json::to_value(plan).unwrap_or(Value::Null);
    a2a::Artifact {
        artifact_id: plan.id.clone(),
        name: Some("plan".into()),
        description: Some(format!("The task's plan, version {}", plan.version)),
        parts: vec![a2a::Part::data(value.clone()).with_media_type(PLAN_MEDIA_TYPE)],
        metadata: Some([(PLAN_EXT.to_owned(), value)].into()),
        extensions: Some(vec![PLAN_EXT.to_owned()]),
    }
}

fn hook_cx(cx: &ToolCx) -> HookCx {
    let mut hook_cx = HookCx::new(Arc::new(Registry::new()), Arc::clone(&cx.host));
    hook_cx.task = Some(cx.task.clone());
    hook_cx
}

fn failed(tool: &str, reason: impl std::fmt::Display) -> ToolError {
    ToolError::Failed {
        tool: tool.into(),
        reason: reason.to_string(),
    }
}

fn invalid(tool: &str, path: &str, reason: impl std::fmt::Display) -> ToolError {
    ToolError::InvalidInput {
        tool: tool.into(),
        path: path.into(),
        reason: reason.to_string(),
    }
}

async fn current(
    store: &dyn PersistenceStore,
    task: &TaskId,
    tool: &str,
) -> Result<Option<Plan>, ToolError> {
    store
        .get_plan(&plan_id(task))
        .await
        .map_err(|e| failed(tool, e))
}

fn output(plan: &Plan) -> ToolOutput {
    ToolOutput::Artifacts {
        value: json!({ "plan": plan.render(), "version": plan.version }),
        artifacts: vec![plan_artifact(plan)],
    }
}

/// `plan.update`: replaces the steps and their dependencies (R13.1–2), keeping the status
/// and output of every step that keeps its id.
pub struct PlanUpdateTool {
    id: EntityId,
    descriptor: ToolDescriptor,
    store: Arc<dyn PersistenceStore>,
    dispatcher: Arc<HookDispatcher>,
}

impl core::fmt::Debug for PlanUpdateTool {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PlanUpdateTool").finish_non_exhaustive()
    }
}

impl PlanUpdateTool {
    /// The tool over `store`, persisting through `plan.persist` hooks on `dispatcher`.
    #[must_use]
    pub fn new(store: Arc<dyn PersistenceStore>, dispatcher: Arc<HookDispatcher>) -> Self {
        Self {
            id: EntityId::new(UPDATE_TOOL),
            descriptor: ToolDescriptor {
                name: UPDATE_TOOL.into(),
                description: "Set this task's plan: the steps, in a dependency order with no cycles. Call again to revise it; steps that keep their id keep their progress. The people on the task see the plan.".into(),
                parameters: json!({
                    "type": "object",
                    "required": ["steps"],
                    "additionalProperties": false,
                    "properties": {
                        "steps": {
                            "type": "array",
                            "minItems": 1,
                            "items": {
                                "type": "object",
                                "required": ["id", "name"],
                                "additionalProperties": false,
                                "properties": {
                                    "id": { "type": "string", "description": "A short stable id, e.g. \"draft\"." },
                                    "name": { "type": "string" },
                                    "description": { "type": "string" },
                                    "after": { "type": "array", "items": { "type": "string" }, "description": "Ids of the steps this one waits for." }
                                }
                            }
                        }
                    }
                }),
                idempotency: Idempotency::Idempotent,
            },
            store,
            dispatcher,
        }
    }
}

impl Entity for PlanUpdateTool {
    fn kind(&self) -> EntityKind {
        EntityKind::Tool
    }
    fn entity_id(&self) -> &EntityId {
        &self.id
    }
}

/// The plan `input` describes for `task`, carrying over `previous`'s progress.
fn build(task: &TaskId, input: &Value, previous: Option<&Plan>) -> Result<Plan, ToolError> {
    let mut plan = Plan::new(plan_id(task), task.clone());
    plan.version = previous.map_or(0, |p| p.version);
    let steps = input["steps"]
        .as_array()
        .ok_or_else(|| invalid(UPDATE_TOOL, "/steps", "an array of steps"))?;
    for (i, raw) in steps.iter().enumerate() {
        let field = |name: &str| raw[name].as_str().map(str::to_owned);
        let id = field("id")
            .ok_or_else(|| invalid(UPDATE_TOOL, &format!("/steps/{i}/id"), "a string"))?;
        let name = field("name")
            .ok_or_else(|| invalid(UPDATE_TOOL, &format!("/steps/{i}/name"), "a string"))?;
        let mut step = Step::new(id.clone(), name, field("description").unwrap_or_default());
        if let Some(before) = previous.and_then(|p| p.step(&id)) {
            step.status = before.status;
            step.output.clone_from(&before.output);
            step.linked_tasks.clone_from(&before.linked_tasks);
        }
        plan.add_step(step)
            .map_err(|e| invalid(UPDATE_TOOL, &format!("/steps/{i}/id"), e))?;
    }
    for (i, raw) in steps.iter().enumerate() {
        let id = raw["id"].as_str().unwrap_or_default();
        for from in raw["after"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            plan.link(from, id)
                .map_err(|e: PlanError| invalid(UPDATE_TOOL, &format!("/steps/{i}/after"), e))?;
        }
    }
    plan.version += 1;
    Ok(plan)
}

harness_async_trait! {
    impl Tool for PlanUpdateTool {
        fn descriptor(&self) -> &ToolDescriptor {
            &self.descriptor
        }

        async fn call(&self, cx: &ToolCx, input: Value) -> Result<ToolOutput, ToolError> {
            let previous = current(self.store.as_ref(), &cx.task, UPDATE_TOOL).await?;
            let plan = build(&cx.task, &input, previous.as_ref())?;
            persist_plan(self.store.as_ref(), &self.dispatcher, &hook_cx(cx), plan.clone())
                .await
                .map_err(|e| match e {
                    crate::HarnessError::Vetoed(v) => ToolError::Denied { tool: UPDATE_TOOL.into(), reason: v.reason },
                    other => failed(UPDATE_TOOL, other),
                })?;
            Ok(output(&plan))
        }
    }
}

/// `plan.step_done`: records a step's outcome (R13.3) behind the `step.execute` hooks.
pub struct PlanStepDoneTool {
    id: EntityId,
    descriptor: ToolDescriptor,
    store: Arc<dyn PersistenceStore>,
    dispatcher: Arc<HookDispatcher>,
}

impl core::fmt::Debug for PlanStepDoneTool {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PlanStepDoneTool").finish_non_exhaustive()
    }
}

impl PlanStepDoneTool {
    /// The tool over `store`, running `step.execute` and `plan.persist` hooks.
    #[must_use]
    pub fn new(store: Arc<dyn PersistenceStore>, dispatcher: Arc<HookDispatcher>) -> Self {
        Self {
            id: EntityId::new(STEP_DONE_TOOL),
            descriptor: ToolDescriptor {
                name: STEP_DONE_TOOL.into(),
                description: "Record the outcome of one step of this task's plan: done (the default), skipped or failed, with a one-line summary of what it produced.".into(),
                parameters: json!({
                    "type": "object",
                    "required": ["step"],
                    "additionalProperties": false,
                    "properties": {
                        "step": { "type": "string", "description": "The step's id." },
                        "status": { "type": "string", "enum": ["done", "skipped", "failed"] },
                        "summary": { "type": "string" }
                    }
                }),
                idempotency: Idempotency::Idempotent,
            },
            store,
            dispatcher,
        }
    }
}

impl Entity for PlanStepDoneTool {
    fn kind(&self) -> EntityKind {
        EntityKind::Tool
    }
    fn entity_id(&self) -> &EntityId {
        &self.id
    }
}

harness_async_trait! {
    impl Tool for PlanStepDoneTool {
        fn descriptor(&self) -> &ToolDescriptor {
            &self.descriptor
        }

        async fn call(&self, cx: &ToolCx, input: Value) -> Result<ToolOutput, ToolError> {
            let mut plan = current(self.store.as_ref(), &cx.task, STEP_DONE_TOOL)
                .await?
                .ok_or_else(|| invalid(STEP_DONE_TOOL, "/step", "the task has no plan yet: call plan.update first"))?;
            let step = input["step"].as_str().ok_or_else(|| invalid(STEP_DONE_TOOL, "/step", "a step id"))?.to_owned();
            let status = match input["status"].as_str().unwrap_or("done") {
                "done" => StepStatus::Done,
                "skipped" => StepStatus::Skipped,
                "failed" => StepStatus::Failed,
                other => return Err(invalid(STEP_DONE_TOOL, "/status", format!("{other} is not done, skipped or failed"))),
            };
            let summary = input["summary"].as_str().map(|s| StepOutput { summary: s.to_owned(), blobs: Vec::new() });
            let record = StepRecord { plan: plan.id.clone(), step, status, output: summary };
            let hook_cx = hook_cx(cx);
            let recorded = self
                .dispatcher
                .run::<StepExecute, _, _>(&hook_cx, record, |r| async move { Ok(r) })
                .await
                .map_err(|e| match e {
                    crate::HarnessError::Vetoed(v) => ToolError::Denied { tool: STEP_DONE_TOOL.into(), reason: v.reason },
                    other => failed(STEP_DONE_TOOL, other),
                })?;
            plan.record(&recorded.step, recorded.status, recorded.output)
                .map_err(|e| invalid(STEP_DONE_TOOL, "/step", e))?;
            persist_plan(self.store.as_ref(), &self.dispatcher, &hook_cx, plan.clone())
                .await
                .map_err(|e| failed(STEP_DONE_TOOL, e))?;
            Ok(output(&plan))
        }
    }
}
