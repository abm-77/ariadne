pub mod analysis;
pub mod backends;
pub mod cost;
pub mod dependency;
pub mod diagnostics;
pub mod docs;
pub mod ir;
pub mod lowering;
pub mod optimize;
pub mod planner;
pub mod profile;
pub mod proto;
pub mod select;
pub mod telemetry;
pub mod testing;
pub mod toolchain;
pub mod validate;

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
