use super::*;
use fixture::{Fixture, WAIT};
use research_agent_core::runner::ToolExecutor;
use serde_json::json;
mod actual_package;
mod analysis_history_waits;
mod analysis_workflow_waits;
mod fixture;
mod flow;
mod lifecycle;
mod retrieval_waits;
mod revalidation;

mod matrix_fixture;
mod matrix_waits;

mod derived_waits;

mod evidence_write_waits;

mod concept_capture_waits;
mod concept_evolution_waits;
mod concept_identity_waits;
mod evolution_fixture;
mod export_fixture;
mod manual_definition_waits;
mod manual_relation_waits;

mod shared_foundations_waits;
