pub mod ir;
pub mod proto;
pub mod diagnostics;
pub mod select;
pub mod validate;
pub mod analysis;
pub mod profile;
pub mod cost;
pub mod lowering;
pub mod planner;
pub mod optimize;
pub mod backends;
pub mod telemetry;
pub mod testing;
pub mod docs;

pub use ir::Workflow;

use diagnostics::Diagnostic;

pub struct Pipeline {
    pub workflow: ir::Workflow,
}

impl Pipeline {
    pub fn new(workflow: ir::Workflow) -> Self {
        Self { workflow }
    }

    pub fn validate(&self) -> Vec<Diagnostic> {
        validate::validate(&self.workflow)
    }

    pub fn plan(&self) -> Result<planner::Plan, Vec<Diagnostic>> {
        planner::plan(&self.workflow)
    }

    pub fn emit_github(
        &self,
        plan: &planner::Plan,
        opts: &backends::github::EmitOptions,
    ) -> String {
        backends::github::emit(plan, opts)
    }
}
