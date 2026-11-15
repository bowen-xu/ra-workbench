//! The explicit pause used by transaction-owner regression tests.
use crate::{
    agent::writing_context::{ContextRead, WritingContextReader, WritingContextRequest},
    ai::structured_resource_validation::Flow,
};
use research_agent_core::submission::ContextText;
use research_workbench_native_sdk::{writing_context::ContextInput, writing_project::Input};
use rusqlite::Connection;
use uuid::Uuid;
pub(in crate::data_interface) struct Waiting;
impl WritingContextReader for Waiting {
    fn selected(
        &mut self,
        _: &Connection,
        _: &std::path::Path,
        request: WritingContextRequest,
    ) -> ContextRead<Vec<ContextText>> {
        Err(Flow::Need(Box::new(Input::Context {
            input: Box::new(ContextInput::Validate { request }),
        })))
    }
    fn current_file(
        &mut self,
        _: &Connection,
        _: &std::path::Path,
        manuscript_id: Uuid,
        path: &str,
    ) -> ContextRead<Vec<ContextText>> {
        Err(Flow::Need(Box::new(Input::Context {
            input: Box::new(ContextInput::CurrentFile {
                manuscript_id,
                relative_path: path.into(),
                rows: research_workbench_native_sdk::data::DataRows { rows: vec![] },
            }),
        })))
    }
}
