//! Admit a saved submission once before any model/tool work. A lost IPC receiver
//! does not own or cancel the supervisor; durable history gates subsequent retries.
use super::*;
use crate::agent::{cancellation::AgentCancellation, tasks::TaskRegistryError};
use research_agent_core::{
    protocol::{EventPayload, ItemKind, TaskKind, TurnOutcome},
    runner::RunError,
    submission::{SubmissionError, SubmissionRecord, SubmissionStore},
};
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::oneshot;

pub enum SubmissionAdmission {
    Started(OwnedTurn),
    AlreadyStarted {
        turn_id: Uuid,
        outcome: Option<TurnOutcome>,
    },
}
#[derive(Debug, thiserror::Error)]
pub enum SubmissionRunError {
    #[error(transparent)]
    Input(#[from] SubmissionError),
    #[error(transparent)]
    Session(#[from] AgentServiceError),
    #[error(transparent)]
    Task(#[from] TaskRegistryError),
    #[error("submission admission requires recovery")]
    RecoveryRequired,
    #[error("compaction history changed from {expected} to {actual}")]
    HistoryChanged { expected: u64, actual: u64 },
}
enum Begin {
    Started(ConversationProjection),
    Existing(Option<TurnOutcome>),
    Changed { expected: u64, actual: u64 },
}
#[derive(Clone, Copy)]
struct TaskAdmission {
    kind: TaskKind,
    expected_revision: Option<u64>,
}
enum AdmissionNotice {
    Started,
    Existing(Option<TurnOutcome>),
}

impl AgentSessionService {
    /// Inspect durable admission before preparing provider or file tools. This
    /// never starts work; first admission still belongs to `start_submission`.
    pub async fn existing_submission(
        &self,
        submissions: &dyn SubmissionStore,
        record: &SubmissionRecord,
    ) -> Result<Option<SubmissionAdmission>, SubmissionRunError> {
        let id = ConversationId::from_uuid(record.conversation_id);
        let saved = read_submission(submissions, id, record.request_id).await?;
        if saved != *record {
            return Err(SubmissionError::Conflict.into());
        }
        self.existing_turn(id, &saved, TaskKind::Regular).await
    }

    /// Compact retries inspect their own durable task kind before refreshing auth.
    pub(crate) async fn existing_compaction(
        &self,
        submissions: &dyn SubmissionStore,
        record: &SubmissionRecord,
    ) -> Result<Option<SubmissionAdmission>, SubmissionRunError> {
        let id = ConversationId::from_uuid(record.conversation_id);
        let saved = read_submission(submissions, id, record.request_id).await?;
        if saved != *record {
            return Err(SubmissionError::Conflict.into());
        }
        self.existing_turn(id, &saved, TaskKind::Compact).await
    }

    async fn existing_turn(
        &self,
        id: ConversationId,
        record: &SubmissionRecord,
        task_kind: TaskKind,
    ) -> Result<Option<SubmissionAdmission>, SubmissionRunError> {
        let history = self.open(id).await?;
        if history
            .turn(record.turn_id)
            .is_some_and(|turn| turn.task_kind != task_kind)
        {
            return Err(SubmissionError::Conflict.into());
        }
        Ok(history
            .turn(record.turn_id)
            .map(|turn| SubmissionAdmission::AlreadyStarted {
                turn_id: record.turn_id,
                outcome: turn.outcome,
            }))
    }

    /// The caller has authorized this specific saved input. The operation gets the
    /// pre-turn history and runs only after input/TurnStarted are durably committed.
    /// Preparation and external work must honor cancellation; event writes must finish.
    pub async fn start_submission<F, Fut>(
        self: &Arc<Self>,
        submissions: &dyn SubmissionStore,
        id: ConversationId,
        request_id: Uuid,
        operation: F,
    ) -> Result<SubmissionAdmission, SubmissionRunError>
    where
        F: FnOnce(SubmissionRecord, ConversationProjection, AgentCancellation) -> Fut
            + Send
            + 'static,
        Fut: Future<Output = Result<(), RunError>> + Send + 'static,
    {
        self.start_submission_task(
            submissions,
            id,
            request_id,
            TaskAdmission {
                kind: TaskKind::Regular,
                expected_revision: None,
            },
            operation,
        )
        .await
    }

    /// Compact admission commits only a task start, never the command as user text.
    /// The caller authorizes the saved, frozen input and injects its model strategy.
    pub(crate) async fn start_compact_submission<F, Fut>(
        self: &Arc<Self>,
        submissions: &dyn SubmissionStore,
        id: ConversationId,
        request_id: Uuid,
        expected_revision: u64,
        operation: F,
    ) -> Result<SubmissionAdmission, SubmissionRunError>
    where
        F: FnOnce(SubmissionRecord, ConversationProjection, AgentCancellation) -> Fut
            + Send
            + 'static,
        Fut: Future<Output = Result<(), RunError>> + Send + 'static,
    {
        self.start_submission_task(
            submissions,
            id,
            request_id,
            TaskAdmission {
                kind: TaskKind::Compact,
                expected_revision: Some(expected_revision),
            },
            operation,
        )
        .await
    }

    async fn start_submission_task<F, Fut>(
        self: &Arc<Self>,
        submissions: &dyn SubmissionStore,
        id: ConversationId,
        request_id: Uuid,
        admission: TaskAdmission,
        operation: F,
    ) -> Result<SubmissionAdmission, SubmissionRunError>
    where
        F: FnOnce(SubmissionRecord, ConversationProjection, AgentCancellation) -> Fut
            + Send
            + 'static,
        Fut: Future<Output = Result<(), RunError>> + Send + 'static,
    {
        let record = read_submission(submissions, id, request_id).await?;
        if let Some(admission) = self.existing_turn(id, &record, admission.kind).await? {
            return Ok(admission);
        }
        let turn_id = record.turn_id;
        let settlement = (admission.kind == TaskKind::Regular).then_some(
            crate::data_interface::TaskSettlement {
                conversation_id: record.conversation_id,
                request_id: record.request_id,
                turn_id,
            },
        );
        let service = self.clone();
        let turn_owned = Arc::new(AtomicBool::new(false));
        let claim = turn_owned.clone();
        let (send, receive) = oneshot::channel();
        let owned = self.start_turn_with_id(
            id,
            turn_id,
            turn_owned,
            settlement,
            move |_, signal| async move {
                match service.begin_submission(id, &record, admission).await {
                    Ok(Begin::Started(history)) => {
                        claim.store(true, Ordering::Release);
                        // The UI may have left; the task remains application-owned.
                        let _ = send.send(Ok(AdmissionNotice::Started));
                        operation(record, history, signal).await
                    }
                    Ok(Begin::Existing(outcome)) => {
                        let _ = send.send(Ok(AdmissionNotice::Existing(outcome)));
                        Ok(())
                    }
                    Ok(Begin::Changed { expected, actual }) => {
                        let _ =
                            send.send(Err(SubmissionRunError::HistoryChanged { expected, actual }));
                        Err(RunError::EventSink)
                    }
                    Err(error) => {
                        let _ = send.send(Err(SubmissionRunError::Session(error)));
                        Err(RunError::EventSink)
                    }
                }
            },
        )?;
        match receive
            .await
            .map_err(|_| SubmissionRunError::RecoveryRequired)??
        {
            AdmissionNotice::Started => Ok(SubmissionAdmission::Started(owned)),
            AdmissionNotice::Existing(outcome) => {
                Ok(SubmissionAdmission::AlreadyStarted { turn_id, outcome })
            }
        }
    }

    async fn begin_submission(
        &self,
        id: ConversationId,
        record: &SubmissionRecord,
        admission: TaskAdmission,
    ) -> Result<Begin, AgentServiceError> {
        let slot = self.slot(id, false)?;
        let mut owner = slot.lock().await;
        // A live or empty session may still be owned by a pending legacy import.
        // Check before admitting a new turn or invoking any model/tool work.
        self.ensure_no_legacy_import(id).await?;
        let session = owner.as_mut().ok_or(AgentServiceError::NotOpen)?;
        if session.needs_recovery() || session.projection().has_unknown_tool_outcome() {
            return Err(SessionError::RecoveryRequired.into());
        }
        if let Some(turn) = session.projection().turn(record.turn_id) {
            if turn.task_kind != admission.kind {
                return Err(SessionError::InvalidBatch.into());
            }
            return Ok(Begin::Existing(turn.outcome));
        }
        if session.projection().active_turn().is_some() {
            return Err(AgentServiceError::Active);
        }
        let actual = session.projection().seq();
        if let Some(expected) = admission.expected_revision
            && expected != actual
        {
            return Ok(Begin::Changed { expected, actual });
        }
        let history = session.projection().clone();
        let turn_id = record.turn_id;
        let item_id = Uuid::new_v4();
        let mut payloads = vec![EventPayload::TurnStarted {
            task_kind: admission.kind,
            turn_id,
            context_snapshot_id: record.context_snapshot_id,
            model: record.input.model.clone(),
        }];
        if admission.kind.is_regular() {
            payloads.push(EventPayload::ItemStarted {
                turn_id,
                item_id,
                kind: ItemKind::UserMessage,
            });
            // The event wire caps each delta at 65,536 Unicode scalar values.
            let text = &record.input.text;
            let mut start = 0;
            for (count, (index, _)) in text.char_indices().enumerate() {
                if count > 0 && count % 65_536 == 0 {
                    payloads.push(EventPayload::TextDelta {
                        turn_id,
                        item_id,
                        text: text[start..index].into(),
                    });
                    start = index;
                }
            }
            if !text.is_empty() {
                payloads.push(EventPayload::TextDelta {
                    turn_id,
                    item_id,
                    text: text[start..].into(),
                });
            }
            payloads.push(EventPayload::ItemCompleted { turn_id, item_id });
        }
        let timestamp = now_ms()?;
        let events = payloads
            .into_iter()
            .enumerate()
            .map(|(index, payload)| {
                AgentEvent::new(
                    id,
                    history.seq() + index as u64 + 1,
                    timestamp,
                    record.request_id,
                    payload,
                )
                .map_err(|_| SessionError::InvalidBatch)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let receipt = session.commit(events).await?;
        self.notify(id, &receipt);
        Ok(Begin::Started(history))
    }
}

async fn read_submission(
    submissions: &dyn SubmissionStore,
    id: ConversationId,
    request_id: Uuid,
) -> Result<SubmissionRecord, SubmissionRunError> {
    let record = submissions.get(*id.as_uuid(), request_id).await?;
    if record.conversation_id != *id.as_uuid() || record.request_id != request_id {
        return Err(SubmissionError::Corrupt.into());
    }
    let images = if record.input.text.trim().is_empty() {
        submissions
            .get_image_references(*id.as_uuid(), request_id)
            .await?
    } else {
        vec![]
    };
    record.input.validate_with_image_references(&images)?;
    Ok(record)
}

#[cfg(test)]
#[path = "submission/tests.rs"]
mod tests;
