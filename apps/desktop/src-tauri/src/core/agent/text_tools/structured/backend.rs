//! Structured text ports retain original effects across lost receipts.
use super::*;
impl TextBackend for StructuredBackend {
    fn read<'a>(
        &'a self,
        path: &'a str,
        saved: bool,
        remember: bool,
    ) -> BackendFuture<'a, TextSnapshot> {
        Box::pin(async move {
            let db = self.store.connection().map_err(|error| error.to_string())?;
            let snapshot = self.snapshot(&db, path, saved)?;
            if remember {
                journal::remember(&db, &self.record, &snapshot)?;
            }
            Ok(snapshot)
        })
    }
    fn preflight<'a>(&'a self, changes: &'a [TextChange]) -> BackendFuture<'a, ()> {
        Box::pin(async move {
            let db = self.store.connection().map_err(|error| error.to_string())?;
            self.preflight_in(&db, changes)
        })
    }
    fn commit<'a>(
        &'a self,
        request_id: &'a str,
        changes: Vec<TextChange>,
    ) -> BackendFuture<'a, Vec<TargetResult>> {
        Box::pin(async move {
            match self.query(request_id).await {
                Ok(Some(prior)) => return Ok(prior),
                Ok(None) => {}
                Err(error) => {
                    return Ok(patch_journal::unknown_changes(
                        &changes,
                        &patch_journal::unavailable_receipt(&self.record, request_id, Some(&error)),
                    ));
                }
            }
            if let Some(binding) = self.store.native_note_trace().cloned() {
                return self
                    .commit_native(request_id.to_owned(), changes, binding)
                    .await;
            }
            let mut db = match self.store.connection() {
                Ok(db) => db,
                Err(error) => {
                    return Ok(patch_journal::unknown_changes(
                        &changes,
                        &patch_journal::query_owner_diagnostic(&self.record, request_id, &error),
                    ));
                }
            };
            if let Some(prior) = self.begin_commit(&db, request_id, &changes)? {
                return Ok(prior);
            }
            let mut results = Vec::new();
            for change in changes {
                let Some(prepared) =
                    self.prepare_write(&db, request_id, change, &mut results, None)?
                else {
                    break;
                };
                let outcome = match crate::structured_resources::write(
                    self.store
                        .require_generation_policy_source()
                        .map_err(|error| error.to_string())?,
                    self.store
                        .require_paper_reading_source()
                        .map_err(|error| error.to_string())?,
                    &mut db,
                    prepared.draft,
                ) {
                    Ok(outcome) => outcome,
                    Err(error) => {
                        receipt::append_unknown(
                            &mut results,
                            prepared.target.path,
                            receipt::uncertain(
                                &self.record,
                                request_id,
                                "structured-write",
                                Some(&error),
                            ),
                        );
                        break;
                    }
                };
                if !self.publish_write(&db, request_id, prepared.target, outcome, &mut results) {
                    break;
                }
            }
            Ok(self.finish_commit(&db, request_id, results))
        })
    }
    fn query<'a>(&'a self, request_id: &'a str) -> BackendFuture<'a, Option<Vec<TargetResult>>> {
        Box::pin(async move {
            let db = self.store.connection().map_err(|error| {
                patch_journal::query_owner_diagnostic(&self.record, request_id, &error).to_string()
            })?;
            let value:Option<String>=db.query_row("select results_json from agent_text_patch_calls where conversation_id=?1 and request_id=?2 and state='complete'",params![self.record.conversation_id.to_string(),request_id],|row|row.get(0)).optional().map_err(|error|patch_journal::target_diagnostic(&self.record,request_id,"target-query",&error).to_string())?;
            if let Some(value) = value {
                let results: Vec<TargetResult> = serde_json::from_str::<Value>(&value)
                    .and_then(|value| serde_json::from_value(value["targets"].clone()))
                    .map_err(|_| {
                        patch_journal::unavailable_receipt(&self.record, request_id, None)
                            .to_string()
                    })?;
                return if results.is_empty() {
                    Err(
                        patch_journal::unavailable_receipt(&self.record, request_id, None)
                            .to_string(),
                    )
                } else {
                    Ok(Some(results))
                };
            }
            let operation = format!("text:{:x}", Sha256::digest(request_id));
            let Some(outcome) = crate::structured_resources::query_outcome(&db, &operation)
                .map_err(|error| {
                    receipt::uncertain(&self.record, request_id, "structured-query", Some(&error))
                        .to_string()
                })?
            else {
                return Ok(None);
            };
            let created:bool=db.query_row("select exists(select 1 from structured_resource_creations where request_id=?1 and creator_conversation_id=?2)",params![format!("create:{operation}"),self.record.conversation_id.to_string()],|row|row.get(0)).map_err(|error|patch_journal::target_diagnostic(&self.record,request_id,"target-query",&error).to_string())?;
            let path = if created {
                configuration::add_created_in(
                    &db,
                    self.record.conversation_id,
                    &Resource::StructuredObject {
                        resource_id: outcome.resource.id.clone(),
                        access: Some(Access::ReadWrite),
                    },
                    self.store.writing_resource_source(),
                )
                .map_err(|_| {
                    receipt::uncertain(&self.record, request_id, "structured-scope", None)
                        .to_string()
                })?;
                "source://agents/skills/edit-structured-resources/assets/reading".to_owned()
            } else {
                outcome.resource.source_uri.clone()
            };
            self.store.notify_structured_resource_changed(
                &outcome.resource.id,
                outcome.resource.paper_id.as_deref(),
            );
            Ok(Some(vec![target_result(&db, outcome, path)]))
        })
    }
}
