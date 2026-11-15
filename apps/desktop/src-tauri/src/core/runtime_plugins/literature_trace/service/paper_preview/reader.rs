//! Re-read the original prefix in each settled transaction before using a decision.
use super::*;
use crate::{
    ai::structured_resource_validation::{DecisionOrigin, Failure, FailureCause, Flow},
    data_interface::guarded_transaction,
    hbs_storage::boundary::{self, BoundaryFailure},
    runtime_plugins::literature_context::preview::PreviewCapture,
};
use research_workbench_native_sdk::paper_context_preview::PaperContextPreviewDecisionV1 as Decision;
use rusqlite::Connection;

type AttemptResult = Result<Step, Error>;

struct Decided {
    capture: PreviewCapture,
    decision: Decision,
    origin: DecisionOrigin,
}
pub(super) struct ResolvedTrace {
    pub(super) prepared: Box<crate::runtime_plugins::literature_trace::data::PreparedTrace>,
    pub(super) decision: research_workbench_native_sdk::literature_trace_protocol::TraceDecision,
    pub(super) origin: DecisionOrigin,
}
pub(super) struct Reader {
    pub(super) resources: Option<PreviewResources>,
    decisions: Vec<Decided>,
    pub(super) source_capture: Option<reading_sources::SourceCapture>,
    replay_observation: Option<crate::agent::reading_context::ReplayObservation>,
    pub(super) trace_resources:
        Option<crate::runtime_plugins::literature_trace::data::TraceResources>,
    pub(super) trace: Option<ResolvedTrace>,
}
impl Reader {
    pub fn new(resources: PreviewResources) -> Self {
        Self {
            resources: Some(resources),
            ..Self::pending()
        }
    }
    pub fn pending() -> Self {
        Self {
            resources: None,
            decisions: Vec::new(),
            source_capture: None,
            replay_observation: None,
            trace_resources: None,
            trace: None,
        }
    }
    pub fn push(&mut self, capture: PreviewCapture, decision: Decision, origin: DecisionOrigin) {
        self.decisions.push(Decided {
            capture,
            decision,
            origin,
        });
    }
    fn decide(
        &self,
        index: usize,
        capture: PreviewCapture,
    ) -> Result<Decision, Flow<PaperContextError, Box<PreviewCapture>>> {
        let Some(decided) = self.decisions.get(index) else {
            return Err(Flow::Need(Box::new(capture)));
        };
        if decided.capture != capture {
            return Err(Flow::original(PaperContextError::Conflict));
        }
        match &decided.decision {
            Decision::Rejected { error } => Err(Flow::Fail(Failure {
                error: error.clone(),
                cause: FailureCause::NativeRejection(Box::new(decided.origin.clone())),
            })),
            decision => Ok(decision.clone()),
        }
    }
    fn read(
        &self,
        db: &Connection,
        request: &PaperContextRequest,
    ) -> Result<PaperContextPreview, Flow<PaperContextError, Box<PreviewCapture>>> {
        let resources = self
            .resources
            .as_ref()
            .ok_or_else(|| Flow::original(PaperContextError::Unavailable))?;
        let header = resources.header(db, request).map_err(Flow::Fail)?;
        self.decide(0, header)?;
        let budget = resources.preflight(db, request).map_err(Flow::Fail)?;
        self.decide(1, budget)?;
        let snapshot = resources.snapshot(db, request).map_err(Flow::Fail)?;
        match self.decide(2, snapshot)? {
            Decision::Previewed { preview } => Ok(preview),
            _ => Err(Flow::original(PaperContextError::Corrupt)),
        }
    }
}
impl Reader {
    fn turn_sources_in(
        &mut self,
        db: &Connection,
        request: reading_sources::Request,
    ) -> AttemptResult {
        if !db.is_autocommit() {
            return Err(Error::unsettled());
        }
        let captured =
            reading_sources::capture_sources(db, &request).map_err(Error::source_failure)?;
        let expected = self.source_capture.get_or_insert_with(|| captured.clone());
        if expected != &captured {
            return Err(Error::sources(reading_sources::Error::Conflict));
        }
        reading_sources::validate_capture_ids(&captured).map_err(Error::sources)?;
        let snapshot = captured.snapshot_request(&request);
        match self.read(db, &snapshot) {
            Ok(preview) => reading_sources::complete_sources(request, captured, preview)
                .map(|response| Step::Complete(Output::TurnSources(response)))
                .map_err(Error::sources),
            Err(Flow::Need(capture)) => Ok(Step::Need(ReadingNeed::Preview(capture))),
            Err(Flow::Fail(failure)) => Err(Error::source_failure(failure)),
        }
    }
}
pub(super) async fn attempt(
    store: AppStore,
    operation: Operation,
    mut reader: Reader,
) -> Result<(Reader, AttemptResult), Error> {
    let paper = operation.is_paper();
    tokio::task::spawn_blocking(move || {
        let database = store.shared_agent_connection();
        let paper_result = match &operation {
            Operation::Status(intent) => Some(sql_step(
                database.paper_prepare_status_with_reader(intent, &mut reader),
                Output::Status,
                "source-read",
            )),
            Operation::Replay(intent) => Some(sql_step(
                database.replay_paper_prepare_with_reader(intent, &mut reader),
                |record| Output::Replay(record.map(Box::new)),
                "source-read",
            )),
            Operation::Prepare { intent, model } => Some(sql_step(
                database.prepare_paper_with_reader(
                    intent,
                    crate::data_interface::PaperPrepareSource {
                        store: store.clone(),
                        model: (**model).clone(),
                    },
                    &mut reader,
                ),
                |record| Output::Prepared(Box::new(record)),
                "write",
            )),
            _ => None,
        };
        if let Some(result) = paper_result {
            return Ok((reader, result));
        }
        let mut db = store
            .connection()
            .map_err(|_| Error::domain(PaperContextError::Unavailable))?;
        let result = guarded_transaction::run_mut(&mut db, |db| {
            let result = match operation {
                Operation::Preview(request) => match attempt_in(db, &request, &reader) {
                    Ok(preview) => Ok(Step::Complete(Output::Preview(preview))),
                    Err(BoundaryError::Failure(failure)) => Err(Error::Boundary(Box::new(failure))),
                    Err(BoundaryError::Pause(pause)) => {
                        Ok(Step::Need(ReadingNeed::Preview(pause.need())))
                    }
                },
                Operation::TurnSources(request) => reader.turn_sources_in(db, request),
                _ => unreachable!(
                    "paper SQL operations return before borrowing the general connection"
                ),
            };
            let unknown = !db.is_autocommit() || result.as_ref().is_err_and(Error::needs_recovery);
            (result, unknown)
        });
        Ok((reader, result))
    })
    .await
    .map_err(|_| {
        if paper {
            Error::paper_unsettled()
        } else {
            Error::unsettled()
        }
    })?
}
fn sql_step<T>(
    result: Result<T, Flow<crate::data_interface::SqlImmediateFailure, ReadingNeed>>,
    output: impl FnOnce(T) -> Output,
    phase: &'static str,
) -> AttemptResult {
    match result {
        Ok(value) => Ok(Step::Complete(output(value))),
        Err(Flow::Need(need)) => Ok(Step::Need(need)),
        Err(Flow::Fail(failure)) => Err(Error::sql_failure(failure, phase)),
    }
}
fn attempt_in(
    db: &mut Connection,
    request: &PaperContextRequest,
    reader: &Reader,
) -> Result<PaperContextPreview, BoundaryError<Box<PreviewCapture>, PaperContextError>> {
    let tx = db.transaction().map_err(|error| {
        BoundaryError::Failure(BoundaryFailure {
            failure: Failure::sql(PaperContextError::Unavailable, "begin", &error),
            phase: "begin",
            outcome: HbsOutcome::NotStarted,
            rollback_error: None,
        })
    })?;
    let result = reader.read(&tx, request);
    let settled = boundary::settle_with(
        tx,
        result,
        "read",
        true,
        false,
        |_| PaperContextError::Unavailable,
        |_| PaperContextError::Unavailable,
    );
    boundary::confirm_with(db, settled, true, |_| PaperContextError::Unavailable)
}

impl crate::agent::reading_context::PaperContextReader for Reader {
    fn observe_replay(
        &mut self,
        observation: crate::agent::reading_context::ReplayObservation,
    ) -> crate::agent::reading_context::ReadingResult<()> {
        match &self.replay_observation {
            Some(previous) if previous != &observation => {
                Err(Flow::original(PaperContextError::Conflict))
            }
            Some(_) => Ok(()),
            None => {
                self.replay_observation = Some(observation);
                Ok(())
            }
        }
    }
    fn preview(
        &mut self,
        db: &Connection,
        request: &PaperContextRequest,
    ) -> crate::agent::reading_context::ReadingResult<PaperContextPreview> {
        use crate::agent::reading_context::ReadingNeed;
        if self.resources.is_none() {
            return Err(Flow::Need(ReadingNeed::Resources));
        }
        self.read(db, request).map_err(|flow| match flow {
            Flow::Fail(failure) => Flow::Fail(failure),
            Flow::Need(capture) => Flow::Need(ReadingNeed::Preview(capture)),
        })
    }
    fn retrieval(
        &mut self,
        tx: &rusqlite::Transaction<'_>,
        _source: Option<crate::agent::retrieval_context::RetrievalTraceSource>,
        selection: &crate::agent::retrieval_context::RetrievalSelection,
        paper: &str,
        snapshot: &str,
    ) -> crate::agent::reading_context::ReadingResult<research_agent_core::submission::ContextText>
    {
        use crate::{
            agent::reading_context::ReadingNeed,
            runtime_plugins::literature_trace::data::{
                capture_paper_trace_in, consume_prepared_trace_in,
            },
        };
        use research_workbench_native_sdk::literature_trace_protocol::TraceDecision;
        selection
            .validate()
            .map_err(|_| Flow::original(PaperContextError::Conflict))?;
        if let Some(resolved) = &self.trace {
            if !resolved.prepared.matches_paper(paper, snapshot, selection) {
                return Err(Flow::original(PaperContextError::Conflict));
            }
            let origin = matches!(resolved.decision, TraceDecision::Rejected { .. })
                .then_some(&resolved.origin);
            return consume_prepared_trace_in(tx, &resolved.prepared, &resolved.decision)
                .map_err(|error| trace_failure(error, origin));
        }
        let Some(resources) = self.trace_resources.take() else {
            return Err(Flow::Need(ReadingNeed::TraceResources));
        };
        let prepared = capture_paper_trace_in(tx, resources, selection, paper, snapshot)
            .map_err(|error| trace_failure(error, None))?;
        Err(Flow::Need(ReadingNeed::Trace(Box::new(prepared))))
    }
}

fn trace_failure(
    error: crate::runtime_plugins::literature_trace::data::TraceDataError,
    origin: Option<&DecisionOrigin>,
) -> Flow<PaperContextError, crate::agent::reading_context::ReadingNeed> {
    use crate::runtime_plugins::literature_trace::data::TraceDataError;
    let cause = match (error, origin) {
        (TraceDataError::Data(error), _) => FailureCause::SourceData(error),
        (TraceDataError::Rejected(_), Some(origin)) => {
            FailureCause::NativeRejection(Box::new(origin.clone()))
        }
        (TraceDataError::InvalidObservation | TraceDataError::InvalidDecision, _) => {
            FailureCause::SourceReadUnavailable
        }
        (TraceDataError::Rejected(_) | TraceDataError::InvalidSource, _) => FailureCause::Original,
    };
    Flow::Fail(Failure {
        error: PaperContextError::Conflict,
        cause,
    })
}
