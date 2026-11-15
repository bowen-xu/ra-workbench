//! The original bootstrap reads, resumed through the existing preparation prefix.
use super::super::{analysis_history::ObservedHistory, reading_history::DecidedHistory};
use super::*;
use crate::runtime_plugins::{
    literature_analysis::capture,
    literature_reading_history::{resources::ReadingHistoryResources, snapshot_attachments},
};
use research_workbench_native_sdk::{
    analysis_history::{HistoryInputV1, HistoryRequestV1},
    paper_context::PreparedPaperContextV1,
    reading_history::{ReadingHistoryDecisionV1 as Decision, ReadingHistoryInputV1 as Input},
};
use rusqlite::Connection;

pub(super) struct BootstrapCache {
    pub resources: Arc<ReadingHistoryResources>,
    sources: Option<DecidedHistory>,
    pub history: Option<ObservedHistory>,
    qualification: Option<DecidedHistory>,
    snapshot: Option<DecidedHistory>,
}
impl BootstrapCache {
    pub fn new(resources: ReadingHistoryResources) -> Self {
        Self {
            resources: Arc::new(resources),
            sources: None,
            history: None,
            qualification: None,
            snapshot: None,
        }
    }
    pub fn push(&mut self, value: DecidedHistory) -> Result<(), Error> {
        match value.input {
            Input::BootstrapSources { .. } => self.sources = Some(value),
            Input::BootstrapHistory { .. } => self.qualification = Some(value),
            Input::BootstrapSnapshot { .. } => self.snapshot = Some(value),
            _ => return Err(Error::mismatch()),
        }
        Ok(())
    }
}
fn qualified<'a>(
    input: &Input,
    decided: &'a Option<DecidedHistory>,
) -> Result<Option<&'a Decision>, Error> {
    let Some(decided) = decided else {
        return Ok(None);
    };
    if decided.input != *input {
        return Err(Error::conflict(Some(decided.origin.clone())));
    }
    match decided.decision.as_ref() {
        Decision::Rejected { message } => {
            Err(Error::rejected(message.clone(), decided.origin.clone()))
        }
        decision => Ok(Some(decision)),
    }
}
fn complete(
    options: research_workbench_native_sdk::reading_history::contracts::PaperReadingBootstrapOptions,
) -> Step {
    Step::Complete(Output::Bootstrap(options))
}
pub(super) fn attempt(
    db: &Connection,
    paper: &str,
    prepared: &PreparedPaperContextV1,
    cache: &worker::Cache,
) -> Result<Step, Error> {
    let Some(cache) = cache.bootstrap.as_ref() else {
        return Err(Error::mismatch());
    };
    let source = Input::BootstrapSources {
        statuses: prepared.attachment_statuses.clone(),
    };
    match qualified(&source, &cache.sources)? {
        None => return Ok(Step::Need(Box::new(Need::ReadingHistory(source)))),
        Some(Decision::Bootstrap { options }) => return Ok(complete(options.clone())),
        Some(Decision::HistoryRequired {}) => {}
        _ => return Err(Error::mismatch()),
    }
    let request = HistoryRequestV1::Latest {
        paper_id: paper.into(),
    };
    let workflow = capture::capture(db, &request, &cache.resources.history.statements())
        .map_err(|error| Error::original(error, "source-read"))?;
    let input = HistoryInputV1 { request, workflow };
    let Some(history) = cache.history.as_ref() else {
        return Ok(Step::Need(Box::new(Need::AnalysisHistory(Box::new(input)))));
    };
    if history.input != input {
        return Err(Error::conflict(Some(history.origin.clone())));
    }
    let policy = Input::BootstrapHistory {
        workflow: history.workflow.clone().map(Box::new),
    };
    let snapshot = match qualified(&policy, &cache.qualification)? {
        None => return Ok(Step::Need(Box::new(Need::ReadingHistory(policy)))),
        Some(Decision::Bootstrap { options }) => return Ok(complete(options.clone())),
        Some(Decision::SnapshotRequired {
            context_snapshot_id,
        }) => context_snapshot_id,
        _ => return Err(Error::mismatch()),
    };
    let Some(workflow) = history.workflow.as_ref() else {
        return Err(Error::mismatch());
    };
    if snapshot != &workflow.context_snapshot_id {
        return Err(Error::mismatch());
    }
    let attachments =
        snapshot_attachments(db, &cache.resources, snapshot).map_err(Error::boundary)?;
    let input = Input::BootstrapSnapshot {
        statuses: prepared.attachment_statuses.clone(),
        workflow: Box::new(workflow.clone()),
        attachments,
    };
    match qualified(&input, &cache.snapshot)? {
        None => Ok(Step::Need(Box::new(Need::ReadingHistory(input)))),
        Some(Decision::Bootstrap { options }) => Ok(complete(options.clone())),
        _ => Err(Error::mismatch()),
    }
}
