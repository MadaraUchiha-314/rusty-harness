//! The sandbox-provider seam: a typed extension point, nothing implemented in issue-3 (S2).

use std::collections::BTreeMap;
use std::time::Duration;

use crate::harness_async_trait;
use crate::registry::Entity;

/// What to run in a sandbox.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SandboxSpec {
    /// The program.
    pub program: String,
    /// Its arguments.
    pub args: Vec<String>,
    /// Its environment; the sandbox adds nothing.
    pub env: BTreeMap<String, String>,
    /// Standard input.
    pub stdin: Option<String>,
    /// A wall-clock limit.
    pub timeout: Option<Duration>,
}

/// What it produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SandboxOutput {
    /// The exit code, when the process exited.
    pub exit_code: Option<i32>,
    /// Standard output.
    pub stdout: String,
    /// Standard error.
    pub stderr: String,
}

harness_async_trait! {
    /// A sandbox provider (S2): the seam a plugin fills; the harness ships none.
    pub trait SandboxProvider: Entity {
        /// Runs `spec` in isolation.
        async fn run(&self, spec: SandboxSpec) -> Result<SandboxOutput, super::ToolError>;
    }
}
