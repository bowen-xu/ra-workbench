//! Reuse original text journals and Core HBS writes, releasing SQL for Native waits.
use super::*;
use crate::runtime_plugins::literature_trace::binding::NoteTraceBinding;

impl StructuredBackend {
    pub(super) async fn commit_native(
        &self,
        request: String,
        changes: Vec<TextChange>,
        binding: NoteTraceBinding,
    ) -> Result<Vec<TargetResult>, String> {
        let owner = self.clone();
        let initial = changes.clone();
        let key = request.clone();
        let begin = tokio::task::spawn_blocking(move || {
            let db = match owner.store.connection() {
                Ok(db) => db,
                Err(error) => {
                    return Ok((
                        Some(patch_journal::unknown_changes(
                            &initial,
                            &patch_journal::query_owner_diagnostic(&owner.record, &key, &error),
                        )),
                        false,
                    ));
                }
            };
            let prior = owner.begin_commit(&db, &key, &initial)?;
            let mut reading = false;
            if prior.is_none() {
                for change in &initial {
                    let target = owner.target(&db, &change.path)?;
                    if target != scope::TEMPLATE_CATALOG_ID {
                        reading |= crate::structured_resources::get_header(&db, &target)
                            .map_err(|error| error.to_string())?
                            .paper_id
                            .is_some();
                    }
                }
            }
            Ok((prior, reading))
        })
        .await;
        let requires_reading = match begin {
            Ok(Ok((Some(prior), _))) => return Ok(prior),
            Ok(Ok((None, reading))) => reading,
            Ok(Err(error)) => return Err(error),
            Err(_) => return Ok(self.unsettled(&request, &changes, "structured-admission")),
        };
        let reading = if requires_reading {
            match binding
                .service
                .paper_context_read_resources(binding.package_key)
                .await
            {
                Ok(reading) => Some(Arc::new(reading)),
                Err(error) => {
                    let mut detail =
                        patch_journal::unavailable_receipt(&self.record, &request, None);
                    detail["nativeFailure"] = json!(error);
                    return Ok(patch_journal::unknown_changes(&changes, &detail));
                }
            }
        } else {
            None
        };
        let all = changes.clone();
        let mut results = Vec::new();
        for change in changes {
            let owner = self.clone();
            let key = request.clone();
            let reading = reading.clone();
            let prepared = tokio::task::spawn_blocking(move || {
                let db = owner
                    .store
                    .connection()
                    .map_err(|error| error.to_string())?;
                owner.preflight_in(&db, std::slice::from_ref(&change))?;
                let prepared =
                    owner.prepare_write(&db, &key, change, &mut results, reading.as_deref().map(|reading| reading as &dyn crate::agent::context_session::resource_tools::FrozenReadingEvidence))?;
                Ok::<_, String>((prepared, results))
            })
            .await;
            let (prepared, returned) = match prepared {
                Ok(result) => result?,
                Err(_) => return Ok(self.unsettled(&request, &all, "structured-prepare")),
            };
            results = returned;
            let Some(prepared) = prepared else {
                break;
            };
            let outcome = match binding
                .service
                .write_text_resource(self.store.clone(), prepared.draft)
                .await
            {
                Ok(outcome) => outcome,
                Err(error) => {
                    let mut detail =
                        receipt::uncertain(&self.record, &request, "structured-write", None);
                    detail["nativeFailure"] = json!(error.native_failure);
                    detail["causeCode"] = json!(error.code);
                    detail["cause"] = json!(error.message);
                    detail["structuredRequestId"] = json!(error.request_id);
                    receipt::append_unknown(&mut results, prepared.target.path, detail);
                    break;
                }
            };
            let owner = self.clone();
            let key = request.clone();
            let published = tokio::task::spawn_blocking(move || {
                let db = match owner.store.connection() {
                    Ok(db) => db,
                    Err(error) => {
                        receipt::append_unknown(
                            &mut results,
                            prepared.target.path,
                            patch_journal::query_owner_diagnostic(&owner.record, &key, &error),
                        );
                        return (false, results);
                    }
                };
                let proceed =
                    owner.publish_write(&db, &key, prepared.target, outcome, &mut results);
                (proceed, results)
            })
            .await;
            let (proceed, returned) = match published {
                Ok(value) => value,
                Err(_) => return Ok(self.unsettled(&request, &all, "structured-publication")),
            };
            results = returned;
            if !proceed {
                break;
            }
        }
        let owner = self.clone();
        let key = request.clone();
        match tokio::task::spawn_blocking(move || match owner.store.connection() {
            Ok(db) => owner.finish_commit(&db, &key, results),
            Err(error) => {
                let detail = patch_journal::query_owner_diagnostic(&owner.record, &key, &error);
                receipt::lost_receipt(&mut results, &detail);
                results
            }
        })
        .await
        {
            Ok(results) => Ok(results),
            Err(_) => Ok(self.unsettled(&request, &all, "structured-terminal")),
        }
    }

    fn unsettled(
        &self,
        request: &str,
        changes: &[TextChange],
        phase: &'static str,
    ) -> Vec<TargetResult> {
        let mut detail = patch_journal::unavailable_receipt(&self.record, request, None);
        detail["code"] = json!("TEXT_PATCH_EFFECT_UNCONFIRMED");
        detail["phase"] = json!(phase);
        detail["safeCause"] = json!("structured_worker_unsettled");
        patch_journal::unknown_changes(changes, &detail)
    }
}
