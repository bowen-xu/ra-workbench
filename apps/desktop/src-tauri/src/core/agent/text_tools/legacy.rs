//! Version-one compatibility uses the same file backend and workspace lease.
use super::*;
use crate::agent::workspace::TextPatchOperation;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    operations: Vec<Operation>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Operation {
    path: String,
    before: Option<String>,
    after: Option<String>,
}
enum LegacyOutcome {
    Confirmed(ToolOutput),
    Unknown(RunError),
}

fn saved_outcome(saved: Value, record: &SubmissionRecord) -> LegacyOutcome {
    // This is the version-one owner's existing application journal, not an
    // arbitrary tool/provider response. Keep unknown receipts unresolved.
    if saved["status"] == "unknown" {
        LegacyOutcome::Unknown(patch_diagnostic::outcome_error(record, &saved, &[]))
    } else {
        LegacyOutcome::Confirmed(ToolOutput {
            is_error: saved["error"].is_string(),
            content: saved.to_string(),
        })
    }
}
impl file::FileBackend {
    pub(super) async fn legacy(&self, call: &ToolCall) -> Result<ToolOutput, RunError> {
        match self.legacy_outcome(call).await {
            Ok(LegacyOutcome::Confirmed(output)) => Ok(output),
            Ok(LegacyOutcome::Unknown(error)) => Err(error),
            Err(message) => Ok(ToolOutput {
                is_error: true,
                content: json!({"error":"patch_conflict","message":message}).to_string(),
            }),
        }
    }
    async fn legacy_outcome(&self, call: &ToolCall) -> Result<LegacyOutcome, String> {
        let args: Request = serde_json::from_value(call.arguments.clone())
            .map_err(|_| "invalid version-one patch")?;
        let input = call.arguments.to_string();
        let call_id = format!("{}:{}", self.record.request_id, call.call_id);
        {
            let db = match self.store.connection() {
                Ok(db) => db,
                Err(error) => {
                    let detail = patch_journal::owner_diagnostic(
                        &self.record,
                        &call_id,
                        "journal-begin-read",
                        &error,
                    );
                    patch_journal::report(&detail);
                    return Ok(LegacyOutcome::Unknown(patch_diagnostic::detail_error(
                        &self.record,
                        &detail,
                    )));
                }
            };
            match journal::lookup_sql(&db, self.record.conversation_id, &call_id, &input) {
                Ok(Some(saved)) => return Ok(saved_outcome(saved, &self.record)),
                Ok(None) => {}
                Err(error) if error.uncertain() => {
                    let mut detail = error.detail(
                        self.record.conversation_id,
                        Some(self.record.request_id),
                        &call_id,
                    );
                    patch_journal::call_identity(&mut detail, &self.record, &call_id);
                    patch_journal::report(&detail);
                    return Ok(LegacyOutcome::Unknown(patch_diagnostic::detail_error(
                        &self.record,
                        &detail,
                    )));
                }
                Err(error) => return Err(error.call_message(&self.record, &call_id)),
            }
        }
        self.with_lease(move |backend, db, lease, _, _| {
        let receipt = backend.receipt_in(db)?;
        let operations = args
            .operations
            .into_iter()
            .map(|operation| {
                Ok(TextPatchOperation {
                    path: backend.path_in(db,
                        &format!("workspace/{}/{}", receipt.manuscript_id, operation.path),
                        true,
                    )?,
                    before: operation.before,
                    after: operation.after,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        match journal::begin_sql(db,backend.record.conversation_id,&call_id,&input) {
            Ok(Some(saved))=>return Ok(saved_outcome(saved,&backend.record)),
            Ok(None)=>{},
            Err(error) if error.uncertain()=>{
                let mut detail=error.detail(backend.record.conversation_id,Some(backend.record.request_id),&call_id);
                patch_journal::call_identity(&mut detail,&backend.record,&call_id);
                patch_journal::report(&detail);return Ok(LegacyOutcome::Unknown(patch_diagnostic::detail_error(&backend.record,&detail)));
            }
            Err(error)=>return Err(error.call_message(&backend.record,&call_id)),
        }
        let value = match lease.apply_text_operations(&operations) {
            Ok(changes) => {
                json!({"status":"applied","changes":changes.iter().map(|change|json!({"path":change.path,"kind":change.kind})).collect::<Vec<_>>()})
            }
            Err(error) => {
                json!({"error":"patch_conflict","message":format!("Modification was not confirmed: {error}. Inspect the original copy before retrying."),"status":"unknown"})
            }
        };
        let outcome = saved_outcome(value.clone(),&backend.record);
        let recorded = journal::record_sql(db, backend.record.conversation_id, &call_id, &value, true);
        if matches!(outcome, LegacyOutcome::Unknown(_)) {
            // Even losing the completion receipt cannot downgrade a possibly
            // applied write into ordinary model retry feedback.
            if let Err(error)=recorded {
                let detail=patch_journal::call_diagnostic(&backend.record,&call_id,"legacy-terminal",&error);
                patch_journal::report(&detail);
                return Ok(LegacyOutcome::Unknown(patch_diagnostic::detail_error(&backend.record,&detail)));
            }
            return Ok(outcome);
        }
        if let Err(error)=recorded {
            // Applying the old contract succeeded, but its receipt did not.
            // The original R remains unresolved; never retry the mutation.
            let detail=patch_journal::call_diagnostic(&backend.record,&call_id,"legacy-terminal",&error);
            let unknown=patch_journal::unknown(value,detail);
            if let Err(error)=journal::record_sql(db,backend.record.conversation_id,&call_id,&unknown,false) {
                patch_journal::report(&unknown["storageFailure"]);
                patch_journal::report(&patch_journal::call_diagnostic(&backend.record,&call_id,"legacy-recovery",&error));
            }
            return Ok(LegacyOutcome::Unknown(patch_diagnostic::outcome_error(&backend.record,&unknown,&[])));
        }
        Ok(outcome)
        }).await
    }
}
