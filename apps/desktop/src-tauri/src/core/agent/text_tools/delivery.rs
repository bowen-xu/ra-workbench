use super::*;
use crate::agent::{writing_delivery_tool, writing_workspace_commands::Scope};
use std::sync::atomic::Ordering;

impl TextTools {
    pub(super) fn delivery_result_committed(&self, call: ToolCall) -> RunFuture<'_, ()> {
        Box::pin(async move {
            let store = self.store.clone();
            let record = self.record.clone();
            let diagnostic_record = record.clone();
            let result = tokio::task::spawn_blocking(move || {
                crate::agent::writing_delivery_commands::proof_transaction(&store, |tx| {
                    crate::agent::writing_delivery_commands::confirm_result_in(
                        tx,
                        &record,
                        &call.call_id,
                    )
                })
            })
            .await
            .map_err(|_| RunError::EventSink)?;
            result.map_err(|error| {
                let detail = json!({ "code": "writing_delivery_result_unconfirmed", "phase": "confirm_result",
                    "safeCause": format!("{error:?}"), "conversationId": diagnostic_record.conversation_id,
                    "requestId": diagnostic_record.request_id, "turnId": diagnostic_record.turn_id,
                    "contextSnapshotId": diagnostic_record.context_snapshot_id });
                research_agent_core::runner::ToolOutcomeDiagnostic::try_from_value(detail)
                    .map(|value| RunError::EventSinkDiagnostic(Box::new(value)))
                    .unwrap_or(RunError::EventSink)
            })
        })
    }
    pub(super) fn delivery_call(&self) -> RunFuture<'_, Option<ToolCall>> {
        Box::pin(async move {
            if self.version() != Some(2) {
                return Ok(None);
            }
            if self.delivery_attempted.load(Ordering::Acquire) {
                #[cfg(target_os = "macos")]
                let changed = if let Some(source) = &self.file_workspace {
                    let current = source.current_lease(&self.record)?;
                    let attempted = *self
                        .delivery_lease
                        .lock()
                        .map_err(|_| RunError::EventSink)?;
                    current.is_some() && attempted.is_some() && current != attempted
                } else {
                    false
                };
                #[cfg(not(target_os = "macos"))]
                let changed = false;
                if !changed {
                    return Ok(None);
                }
            }
            let backend = self.file_backend();
            if backend.receipt().is_err() {
                return Ok(None);
            }
            if let Some(call) = self.previous.finalization_call().await? {
                return Ok(Some(call));
            }
            if backend.changes_empty().await == Ok(true) {
                return Ok(None);
            }
            Ok(Some(ToolCall {
                namespace: None,
                raw_arguments_json: None,
                call_id: Uuid::new_v4().to_string(),
                name: writing_delivery_tool::NAME.into(),
                arguments: json!({}),
            }))
        })
    }
    pub(super) async fn deliver_files(&self, call_id: &str) -> Result<ToolOutput, RunError> {
        let attempted = self.delivery_attempted.swap(true, Ordering::AcqRel);
        if attempted {
            #[cfg(target_os = "macos")]
            let advanced = if let Some(source) = &self.file_workspace {
                let current = source.current_lease(&self.record)?;
                let previous = *self
                    .delivery_lease
                    .lock()
                    .map_err(|_| RunError::EventSink)?;
                current.is_some() && previous.is_some() && current != previous
            } else {
                false
            };
            #[cfg(not(target_os = "macos"))]
            let advanced = false;
            if !advanced {
                return Err(RunError::ToolOutcomeUnknown);
            }
        }
        let backend = self.file_backend();
        // Scope validation precedes any writer handoff or delivery dispatch.
        let receipt = backend.receipt().map_err(|_| RunError::ToolRejected)?;
        let binding = backend
            .delivery_binding()
            .await
            .map_err(|_| RunError::ToolRejected)?;
        {
            let mut previous = self
                .delivery_lease
                .lock()
                .map_err(|_| RunError::EventSink)?;
            if previous.is_some_and(|lease| lease != binding.lease_id) {
                self.delivery_sealed.store(false, Ordering::Release);
            }
            *previous = Some(binding.lease_id);
        }
        if binding.conversation_id != receipt.conversation_id
            || binding.request_id != receipt.request_id
            || binding.manuscript_id != receipt.manuscript_id
            || binding.turn_id != self.record.turn_id
            || binding.context_snapshot_id != self.record.context_snapshot_id
        {
            return Err(RunError::ToolRejected);
        }
        if !self.delivery_sealed.load(Ordering::Acquire) {
            if !self
                .previous
                .release_for_finalization()
                .await
                .map_err(|_| RunError::ToolOutcomeUnknown)?
            {
                backend
                    .seal_for_delivery()
                    .await
                    .map_err(|_| RunError::ToolUnavailable)?;
            }
            self.delivery_sealed.store(true, Ordering::Release);
        }
        let response = self
            .store
            .writing_delivery_tools()
            .deliver(
                Scope {
                    conversation_id: receipt.conversation_id,
                    request_id: receipt.request_id,
                    manuscript_id: receipt.manuscript_id,
                    context_snapshot_id: self.record.context_snapshot_id,
                },
                self.record.turn_id,
                crate::agent::writing_workspace_commands::Confirmation {
                    scope: Scope {
                        conversation_id: binding.conversation_id,
                        request_id: binding.request_id,
                        manuscript_id: binding.manuscript_id,
                        context_snapshot_id: binding.context_snapshot_id,
                    },
                    turn_id: binding.turn_id,
                    lease_id: binding.lease_id,
                    manifest_hash: binding.manifest_hash.clone(),
                },
            )
            .await
            .map_err(|_| RunError::ToolOutcomeUnknown)?;
        if !response.complete {
            return Err(RunError::ToolOutcomeUnknown);
        }
        let request = crate::agent::writing_delivery_commands::Acknowledge {
            scope: response.request.scope.clone(),
            turn_id: response.request.turn_id,
        };
        let proof = crate::agent::writing_delivery_commands::verify_running_with_call(
            &self.store,
            request.clone(),
            response.request.request_id,
            call_id,
            binding,
        )
        .await
        .map_err(|error| delivery_proof_error(&request, error))?;
        confirmed_delivery(response, proof)
    }
}

fn delivery_proof_error(
    request: &crate::agent::writing_delivery_commands::Acknowledge,
    error: crate::agent::writing_workspace_commands::Error,
) -> RunError {
    let detail = json!({
        "code": "writing_delivery_proof_unavailable",
        "phase": "verify_delivery",
        "safeCause": format!("{error:?}"),
        "conversationId": request.scope.conversation_id,
        "requestId": request.scope.request_id,
        "turnId": request.turn_id,
        "manuscriptId": request.scope.manuscript_id,
        "contextSnapshotId": request.scope.context_snapshot_id,
    });
    research_agent_core::runner::ToolOutcomeDiagnostic::try_from_value(detail)
        .map(|value| RunError::ToolOutcomeUnknownDiagnostic(Box::new(value)))
        .unwrap_or(RunError::ToolOutcomeUnknown)
}

fn confirmed_delivery(
    response: writing_delivery_tool::ResultInput,
    proof: crate::agent::writing_delivery_commands::RunningProof,
) -> Result<ToolOutput, RunError> {
    // The existing typed receipt distinguishes confirmation from an unknown or
    // partial application. Its message is never interpreted as execution state.
    // The original scope/application journal remains the reconciliation owner.
    if !response.complete {
        return Err(RunError::ToolOutcomeUnknown);
    }
    let (status, message) = match proof {
        crate::agent::writing_delivery_commands::RunningProof::Applied => {
            ("complete", response.message)
        }
        crate::agent::writing_delivery_commands::RunningProof::Rejected => (
            "rejected",
            "File changes were rejected; no application was confirmed.".into(),
        ),
        crate::agent::writing_delivery_commands::RunningProof::Empty => (
            "unchanged",
            "The released workspace has no file changes.".into(),
        ),
    };
    Ok(ToolOutput {
        is_error: false,
        content: json!({"status":status,"message":message,
            "requestId":response.request.scope.request_id})
        .to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn response(complete: bool) -> writing_delivery_tool::ResultInput {
        let scope = Scope {
            conversation_id: Uuid::new_v4(),
            request_id: Uuid::new_v4(),
            manuscript_id: Uuid::new_v4(),
            context_snapshot_id: Uuid::new_v4(),
        };
        let turn_id = Uuid::new_v4();
        writing_delivery_tool::ResultInput {
            request: writing_delivery_tool::Request {
                request_id: Uuid::new_v4(),
                turn_id,
                scope: scope.clone(),
                binding: crate::agent::writing_workspace_commands::Confirmation {
                    scope,
                    turn_id,
                    lease_id: Uuid::new_v4(),
                    manifest_hash: "a".repeat(64),
                },
            },
            complete,
            message: "synthetic delivery receipt".into(),
        }
    }
    #[test]
    fn unconfirmed_delivery_requires_reconciliation_instead_of_model_retry() {
        assert!(matches!(
            confirmed_delivery(
                response(false),
                crate::agent::writing_delivery_commands::RunningProof::Applied
            ),
            Err(RunError::ToolOutcomeUnknown)
        ));
    }
    #[test]
    fn confirmed_delivery_preserves_the_original_application_request_identity() {
        let response = response(true);
        let request_id = response.request.scope.request_id;
        let output = confirmed_delivery(
            response,
            crate::agent::writing_delivery_commands::RunningProof::Applied,
        )
        .unwrap();
        assert!(!output.is_error);
        assert_eq!(
            serde_json::from_str::<Value>(&output.content).unwrap()["requestId"],
            request_id.to_string()
        );
    }

    #[test]
    fn rejected_or_empty_delivery_does_not_repeat_a_success_claim_from_the_host() {
        use crate::agent::writing_delivery_commands::RunningProof;
        for (proof, status) in [
            (RunningProof::Rejected, "rejected"),
            (RunningProof::Empty, "unchanged"),
        ] {
            let output = confirmed_delivery(response(true), proof).unwrap();
            let value: Value = serde_json::from_str(&output.content).unwrap();
            assert_eq!(value["status"], status);
            assert_ne!(value["message"], "synthetic delivery receipt");
        }
    }
}
