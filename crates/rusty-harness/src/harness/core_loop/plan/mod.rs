//! The plan as a DAG of steps (R13).
//!
//! Steps are nodes, `edges` are `from → to` dependencies. [`Plan::link`] rejects an edge
//! that would close a cycle with [`PlanError::Cycle`] (R13.2); [`Plan::topo`] and
//! [`Plan::ready`] are Kahn's algorithm inline (design.md § Plan and Step, no graph
//! crate). The plan is persisted whole: the store's `put_plan` runs the `plan.persist`
//! hooks so a plugin can mirror it (R14.3).

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use crate::harness::core_loop::task::{TaskId, TaskRef};
use crate::harness::hooks::{HookPoint, Operation};

/// A plan id.
pub type PlanId = String;
/// A step id, unique within its plan.
pub type StepId = String;

/// Where a step is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StepStatus {
    /// Not started.
    #[default]
    Pending,
    /// Being executed.
    Running,
    /// Done, with its output recorded.
    Done,
    /// Skipped by a participant or the model.
    Skipped,
    /// Failed; the plan may be edited around it.
    Failed,
}

/// What a step produced (R13.3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepOutput {
    /// A short summary for the plan view and the context window.
    pub summary: String,
    /// Blob keys of the full output, when it was stored.
    #[serde(default)]
    pub blobs: Vec<String>,
}

/// One step (R13.1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    /// Unique within the plan.
    pub id: StepId,
    /// Short name.
    pub name: String,
    /// What to do.
    pub description: String,
    /// Where it is.
    #[serde(default)]
    pub status: StepStatus,
    /// What it produced.
    #[serde(default)]
    pub output: Option<StepOutput>,
    /// Tasks spawned while executing it (R12.3).
    #[serde(default)]
    pub linked_tasks: Vec<TaskRef>,
}

impl Step {
    /// A pending step.
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: description.into(),
            status: StepStatus::Pending,
            output: None,
            linked_tasks: Vec::new(),
        }
    }
}

/// A typed plan error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlanError {
    /// The edit would close a cycle (R13.2).
    #[error("plan {plan}: linking {from} -> {to} would close a cycle")]
    Cycle {
        /// The plan.
        plan: PlanId,
        /// The edge's tail.
        from: StepId,
        /// The edge's head.
        to: StepId,
    },
    /// No step has that id.
    #[error("plan {plan}: no step {step}")]
    UnknownStep {
        /// The plan.
        plan: PlanId,
        /// The id.
        step: StepId,
    },
    /// A step with that id already exists.
    #[error("plan {plan}: step {step} already exists")]
    DuplicateStep {
        /// The plan.
        plan: PlanId,
        /// The id.
        step: StepId,
    },
}

/// The plan (R13.1): a DAG of steps for one task.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    /// The plan's id.
    pub id: PlanId,
    /// The task it delivers.
    pub task: TaskId,
    steps: Vec<Step>,
    edges: Vec<(StepId, StepId)>,
    /// Incremented by every edit.
    pub version: u64,
}

impl Plan {
    /// An empty plan for `task`.
    #[must_use]
    pub fn new(id: impl Into<String>, task: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            task: task.into(),
            steps: Vec::new(),
            edges: Vec::new(),
            version: 0,
        }
    }

    /// The steps, in insertion order.
    #[must_use]
    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    /// The edges, `from → to`.
    #[must_use]
    pub fn edges(&self) -> &[(StepId, StepId)] {
        &self.edges
    }

    /// The step with `id`.
    #[must_use]
    pub fn step(&self, id: &str) -> Option<&Step> {
        self.steps.iter().find(|s| s.id == id)
    }

    /// Adds a step.
    ///
    /// # Errors
    ///
    /// [`PlanError::DuplicateStep`] when the id is taken.
    pub fn add_step(&mut self, step: Step) -> Result<StepId, PlanError> {
        if self.step(&step.id).is_some() {
            return Err(PlanError::DuplicateStep {
                plan: self.id.clone(),
                step: step.id,
            });
        }
        let id = step.id.clone();
        self.steps.push(step);
        self.version += 1;
        Ok(id)
    }

    /// Adds the dependency `from → to` (`to` waits for `from`).
    ///
    /// # Errors
    ///
    /// [`PlanError::UnknownStep`] for an id the plan lacks; [`PlanError::Cycle`] when the
    /// edge would close a cycle, in which case the plan is unchanged (R13.2).
    pub fn link(&mut self, from: &str, to: &str) -> Result<(), PlanError> {
        for id in [from, to] {
            if self.step(id).is_none() {
                return Err(PlanError::UnknownStep {
                    plan: self.id.clone(),
                    step: id.to_owned(),
                });
            }
        }
        if self.edges.iter().any(|(f, t)| f == from && t == to) {
            return Ok(());
        }
        let edge = (from.to_owned(), to.to_owned());
        self.edges.push(edge);
        if self.topo().len() != self.steps.len() {
            self.edges.pop();
            return Err(PlanError::Cycle {
                plan: self.id.clone(),
                from: from.to_owned(),
                to: to.to_owned(),
            });
        }
        self.version += 1;
        Ok(())
    }

    /// Records a step's status and output (R13.3).
    ///
    /// # Errors
    ///
    /// [`PlanError::UnknownStep`].
    pub fn record(
        &mut self,
        id: &str,
        status: StepStatus,
        output: Option<StepOutput>,
    ) -> Result<(), PlanError> {
        let plan = self.id.clone();
        let step = self
            .steps
            .iter_mut()
            .find(|s| s.id == id)
            .ok_or(PlanError::UnknownStep {
                plan,
                step: id.to_owned(),
            })?;
        step.status = status;
        if output.is_some() {
            step.output = output;
        }
        self.version += 1;
        Ok(())
    }

    /// Links a spawned task to a step (R12.3).
    ///
    /// # Errors
    ///
    /// [`PlanError::UnknownStep`].
    pub fn link_task(&mut self, id: &str, task: TaskRef) -> Result<(), PlanError> {
        let plan = self.id.clone();
        let step = self
            .steps
            .iter_mut()
            .find(|s| s.id == id)
            .ok_or(PlanError::UnknownStep {
                plan,
                step: id.to_owned(),
            })?;
        step.linked_tasks.push(task);
        self.version += 1;
        Ok(())
    }

    /// A topological order of the steps (Kahn); shorter than `steps()` when a cycle
    /// exists, which [`Plan::link`] prevents.
    #[must_use]
    pub fn topo(&self) -> Vec<StepId> {
        let mut indegree: Vec<usize> = self
            .steps
            .iter()
            .map(|s| self.edges.iter().filter(|(_, t)| *t == s.id).count())
            .collect();
        let mut queue: std::collections::VecDeque<usize> = (0..self.steps.len())
            .filter(|&i| indegree[i] == 0)
            .collect();
        let mut order = Vec::with_capacity(self.steps.len());
        while let Some(i) = queue.pop_front() {
            let id = &self.steps[i].id;
            order.push(id.clone());
            for (_, to) in self.edges.iter().filter(|(f, _)| f == id) {
                if let Some(j) = self.steps.iter().position(|s| &s.id == to) {
                    indegree[j] -= 1;
                    if indegree[j] == 0 {
                        queue.push_back(j);
                    }
                }
            }
        }
        order
    }

    /// The pending steps whose every predecessor is done or skipped: what can run now.
    #[must_use]
    pub fn ready(&self) -> Vec<StepId> {
        self.topo()
            .into_iter()
            .filter(|id| {
                let Some(step) = self.step(id) else {
                    return false;
                };
                step.status == StepStatus::Pending
                    && self.edges.iter().filter(|(_, t)| t == id).all(|(f, _)| {
                        matches!(
                            self.step(f).map(|s| s.status),
                            Some(StepStatus::Done | StepStatus::Skipped)
                        )
                    })
            })
            .collect()
    }

    /// Whether every step is done or skipped.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.steps
            .iter()
            .all(|s| matches!(s.status, StepStatus::Done | StepStatus::Skipped))
    }

    /// The plan as text for the context window's `Plan` position (R13.4): one line per
    /// step in topological order, with its status and output summary.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        for step in self.topo().iter().filter_map(|id| self.step(id)) {
            let id = &step.id;
            let mark = match step.status {
                StepStatus::Pending => " ",
                StepStatus::Running => ">",
                StepStatus::Done => "x",
                StepStatus::Skipped => "-",
                StepStatus::Failed => "!",
            };
            let deps: Vec<&str> = self
                .edges
                .iter()
                .filter(|(_, t)| t == id)
                .map(|(f, _)| f.as_str())
                .collect();
            let _ = write!(out, "- [{mark}] {}: {}", step.id, step.name);
            if !deps.is_empty() {
                let _ = write!(out, " (after {})", deps.join(", "));
            }
            if let Some(output) = &step.output {
                let _ = write!(out, " -> {}", output.summary);
            }
            out.push('\n');
        }
        out
    }
}

/// The `plan.persist` operation (R14.3): the plan goes in, nothing comes out.
#[derive(Debug)]
pub struct PlanPersist;

impl Operation for PlanPersist {
    const POINT: HookPoint = HookPoint::PlanPersist;
    type Input = Plan;
    type Output = ();
    fn identity(input: &Plan) -> String {
        input.id.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::{Plan, PlanError, Step, StepOutput, StepStatus};

    fn three() -> Plan {
        let mut plan = Plan::new("p1", "t1");
        for (id, name) in [("a", "read"), ("b", "write"), ("c", "ship")] {
            plan.add_step(Step::new(id, name, "")).unwrap();
        }
        plan.link("a", "b").unwrap();
        plan.link("b", "c").unwrap();
        plan
    }

    #[test]
    fn a_cycle_is_rejected_with_a_typed_error_and_the_plan_is_unchanged() {
        let mut plan = three();
        let before = plan.clone();
        let err = plan.link("c", "a").expect_err("cycle");
        assert_eq!(
            err,
            PlanError::Cycle {
                plan: "p1".into(),
                from: "c".into(),
                to: "a".into()
            }
        );
        assert_eq!(plan, before);
        assert_eq!(
            plan.link("a", "a").unwrap_err(),
            PlanError::Cycle {
                plan: "p1".into(),
                from: "a".into(),
                to: "a".into()
            }
        );
        assert!(matches!(
            plan.link("a", "zz"),
            Err(PlanError::UnknownStep { .. })
        ));
        assert!(matches!(
            plan.add_step(Step::new("a", "", "")),
            Err(PlanError::DuplicateStep { .. })
        ));
    }

    #[test]
    fn topo_and_ready_follow_the_edges_and_outputs_are_recorded() {
        let mut plan = three();
        plan.add_step(Step::new("d", "docs", "")).unwrap();
        plan.link("a", "d").unwrap();
        assert_eq!(plan.topo(), ["a", "b", "d", "c"]);
        assert_eq!(plan.ready(), ["a"]);
        plan.record(
            "a",
            StepStatus::Done,
            Some(StepOutput {
                summary: "read it".into(),
                blobs: vec![],
            }),
        )
        .unwrap();
        assert_eq!(plan.ready(), ["b", "d"]);
        plan.record("b", StepStatus::Skipped, None).unwrap();
        assert_eq!(plan.ready(), ["d", "c"]);
        assert!(!plan.is_complete());
        let text = plan.render();
        assert!(
            text.starts_with("- [x] a: read -> read it\n- [-] b: write (after a)\n"),
            "{text}"
        );
        assert!(matches!(
            plan.record("zz", StepStatus::Done, None),
            Err(PlanError::UnknownStep { .. })
        ));
        let json = serde_json::to_string(&plan).unwrap();
        assert_eq!(serde_json::from_str::<Plan>(&json).unwrap(), plan);
    }
}
