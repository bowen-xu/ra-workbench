use super::*;
use patch_journal::CallSave;

fn path(hunk: &Hunk) -> String {
    match hunk {
        Hunk::AddFile { path, .. } | Hunk::DeleteFile { path } | Hunk::UpdateFile { path, .. } => {
            path.to_string_lossy().into_owned()
        }
    }
}

fn summary(results: &[TargetResult], previews: &[Value]) -> Value {
    let declined = !results.is_empty()
        && results.iter().all(|result| {
            result.status == TargetStatus::Failed
                && serde_json::from_str::<Value>(&result.message).is_ok_and(|reason| {
                    reason["code"] == "edit_declined" || structured::validation_declined(&reason)
                })
        });
    let success = results.iter().any(|result| {
        matches!(
            result.status,
            TargetStatus::Applied | TargetStatus::Proposed
        )
    });
    let failure = results
        .iter()
        .any(|result| matches!(result.status, TargetStatus::Failed | TargetStatus::Unknown));
    let mut value = json!({"status":if declined{"declined"}else if success&&failure{"partial"}else if results.iter().any(|result|result.status==TargetStatus::Unknown){"unknown"}else if failure{"failed"}else{"complete"},"targets":results});
    structured::public_feedback(&mut value);
    let previews = patch_preview::applied(previews, results);
    if !previews.is_empty() {
        value["fileDiffs"] = json!(previews);
    }
    value
}
impl TextTools {
    pub(super) async fn apply(&self, call_id: &str, patch: &str) -> Result<Value, String> {
        let identity = format!("{}:{call_id}", self.record.request_id);
        let call_id = identity.as_str();
        let hunks = text_patch::parse(patch).map_err(|error| error.to_string())?;
        let prior = {
            let db = match self.store.connection() {
                Ok(db) => db,
                Err(error) => {
                    let detail = patch_journal::owner_diagnostic(
                        &self.record,
                        call_id,
                        "journal-begin-read",
                        &error,
                    );
                    patch_journal::report(&detail);
                    return Ok(patch_journal::unknown(json!({"targets":[]}), detail));
                }
            };
            match journal::begin_sql(&db, self.record.conversation_id, call_id, patch) {
                Ok(prior) => prior,
                Err(error) if error.uncertain() => {
                    let mut detail = error.detail(
                        self.record.conversation_id,
                        Some(self.record.request_id),
                        call_id,
                    );
                    patch_journal::call_identity(&mut detail, &self.record, call_id);
                    patch_journal::report(&detail);
                    return Ok(patch_journal::unknown(json!({"targets":[]}), detail));
                }
                Err(error) => return Err(error.call_message(&self.record, call_id)),
            }
        };
        if let Some(result) = prior {
            if result["status"] != "unknown" {
                if result.get("noteDelivery").is_some() {
                    let targets = structured::decode_targets(result["targets"].clone());
                    return Ok(match targets {
                        Ok(targets) => self.finish_note_delivery(call_id, result, &targets, None),
                        Err(_) => patch_journal::unknown(
                            result,
                            note::delivery::diagnostic(
                                &self.record,
                                call_id,
                                &note::delivery::Error::Invalid,
                            ),
                        ),
                    });
                }
                return Ok(result);
            }
            return self.recover(call_id, &hunks, result).await;
        }
        let mut note_paths = hunks
            .iter()
            .filter(|hunk| self.is_note_path(&path(hunk)))
            .map(path)
            .collect::<Vec<_>>();
        for hunk in &hunks {
            if let Hunk::UpdateFile {
                move_path: Some(path),
                ..
            } = hunk
                && self.is_note_path(&path.to_string_lossy())
            {
                note_paths.push(path.to_string_lossy().into_owned());
            }
        }
        let (editor_guard, note_flush_error) = if !note_paths.is_empty() {
            match (note::NoteBackend {
                scope: None,
                store: self.store.clone(),
                record: self.record.clone(),
            })
            .editor_targets(&note_paths, true)
            {
                Ok(targets) => match self
                    .store
                    .text_editors()
                    .acquire_pages(Some(&targets))
                    .await
                {
                    Ok(guard) => (Some(guard), None),
                    Err(error) => (None, Some(error)),
                },
                Err(error) => (None, Some(error)),
            }
        } else {
            (None, None)
        };
        let mut results = vec![];
        let mut previews = vec![];
        let mut notes = vec![];
        let mut note_failure = false;
        for (index, hunk) in hunks.iter().enumerate() {
            let target = path(hunk);
            let prepared = async {
                if self.is_note_path(&target)
                    && let Some(error) = &note_flush_error
                {
                    return Err(error.clone());
                }
                let backend = self.backend(&target)?;
                let before = if matches!(hunk, Hunk::AddFile { .. }) {
                    None
                } else {
                    Some(backend.read(&target, false, false).await?)
                };
                let change = text_patch::backend::prepare(hunk, before)?;
                previews.push(patch_preview::preview(&change));
                if !self.is_note_path(&target) {
                    backend.preflight(std::slice::from_ref(&change)).await?;
                }
                Ok::<_, String>((backend, change))
            }
            .await;
            match prepared {
                Ok((_, change)) if self.is_note_path(&target) => notes.push(change),
                Ok((backend, change)) => {
                    let request = format!("{call_id}:file:{index}");
                    match backend.commit(&request, vec![change]).await {
                        Ok(value) => results.extend(value),
                        Err(message) => results.push(TargetResult {
                            path: target,
                            status: TargetStatus::Failed,
                            message,
                            change_id: None,
                            generation: None,
                            name: None,
                        }),
                    }
                }
                Err(message) => {
                    if self.is_note_path(&target) {
                        note_failure = true;
                    }
                    results.push(TargetResult {
                        path: target,
                        status: TargetStatus::Failed,
                        message,
                        change_id: None,
                        generation: None,
                        name: None,
                    });
                }
            }
            let value = summary(&results, &previews);
            if let Err(error) = self.save_patch_call(call_id, &value, &results, CallSave::Progress)
            {
                return error.into_result();
            }
            if results
                .iter()
                .any(|result| result.status == TargetStatus::Unknown)
            {
                return Ok(value);
            }
        }
        if !notes.is_empty() {
            if note_failure {
                results.extend(notes.into_iter().map(|change| TargetResult {
                    path: change.path,
                    status: TargetStatus::Failed,
                    message: "Related note targets failed; candidate group was not changed.".into(),
                    change_id: None,
                    generation: None,
                    name: None,
                }));
            } else {
                let backend = self.backend(&notes[0].path)?;
                let request = format!("{call_id}:notes");
                let paths = notes
                    .iter()
                    .map(|change| change.path.clone())
                    .collect::<Vec<_>>();
                match backend.preflight(&notes).await {
                    Err(message) => results.extend(paths.into_iter().map(|path| TargetResult {
                        path,
                        status: TargetStatus::Failed,
                        message: message.clone(),
                        change_id: None,
                        generation: None,
                        name: None,
                    })),
                    Ok(()) => {
                        let mut progress = summary(&results, &previews);
                        note_delivery::mark(&mut progress, call_id);
                        if let Err(error) =
                            self.save_patch_call(call_id, &progress, &results, CallSave::Progress)
                        {
                            return error.into_result();
                        }
                        match backend.commit(&request, notes).await {
                            Ok(value) => results.extend(value),
                            Err(message) => {
                                results.extend(paths.into_iter().map(|path| TargetResult {
                                    path,
                                    status: TargetStatus::Failed,
                                    message: message.clone(),
                                    change_id: None,
                                    generation: None,
                                    name: None,
                                }))
                            }
                        }
                    }
                }
            }
        }
        let mut result = summary(&results, &previews);
        if results.iter().any(|target| {
            note_paths.contains(&target.path)
                && matches!(target.status, TargetStatus::Applied | TargetStatus::Unknown)
        }) {
            note_delivery::mark(&mut result, call_id);
        }
        match self.save_patch_call(call_id, &result, &results, CallSave::Terminal) {
            Ok(db) => drop(db),
            Err(error) => return error.into_result(),
        };
        Ok(self.finish_note_delivery(call_id, result, &results, editor_guard))
    }
    async fn recover(&self, call_id: &str, hunks: &[Hunk], prior: Value) -> Result<Value, String> {
        let previews = prior["fileDiffs"].as_array().cloned().unwrap_or_default();
        let mut results = match structured::decode_targets(prior["targets"].clone()) {
            Ok(results) => results,
            Err(_) => {
                let mut detail = journal::BeginFailure::Corrupt.detail(
                    self.record.conversation_id,
                    Some(self.record.request_id),
                    call_id,
                );
                patch_journal::call_identity(&mut detail, &self.record, call_id);
                patch_journal::report(&detail);
                return Ok(patch_journal::unknown(json!({"targets":[]}), detail));
            }
        };
        let mut failures = vec![];
        for (index, hunk) in hunks.iter().enumerate() {
            let target = path(hunk);
            if results
                .iter()
                .any(|result| result.path == target && result.status != TargetStatus::Unknown)
            {
                continue;
            }
            let previous = results.iter().find(|result| result.path == target).cloned();
            results.retain(|result| result.path != target);
            let backend = self.backend(&target);
            let target_request = format!(
                "{call_id}:{}",
                if self.is_note_path(&target) {
                    "notes".into()
                } else {
                    format!("file:{index}")
                }
            );
            let receipt = if let Ok(backend) = backend {
                backend.query(&target_request).await
            } else {
                Ok(None)
            };
            if let Ok(Some(receipt)) = &receipt {
                for result in receipt {
                    if !results.iter().any(|saved| saved.path == result.path) {
                        results.push(result.clone());
                    }
                }
            } else {
                let mut detail = patch_journal::unavailable_receipt(
                    &self.record,
                    &target_request,
                    receipt.as_ref().err().map(String::as_str),
                );
                patch_journal::call_identity(&mut detail, &self.record, call_id);
                results.push(previous.unwrap_or(TargetResult {
                    path: target,
                    status: TargetStatus::Unknown,
                    message: detail.to_string(),
                    change_id: None,
                    generation: None,
                    name: None,
                }));
                if receipt.is_err() {
                    failures.push(detail);
                }
            }
        }
        let mut value = summary(&results, &previews);
        if let Some(intent) = prior.get("noteDelivery") {
            value["noteDelivery"] = intent.clone();
        }
        for detail in failures {
            value = patch_journal::unknown(value, detail);
        }
        if let Err(error) = self.save_patch_call(call_id, &value, &results, CallSave::Recovery) {
            return error.into_result();
        }
        Ok(self.finish_note_delivery(call_id, value, &results, None))
    }
}

#[cfg(test)]
mod outcome_tests {
    use super::*;
    fn invalid(code: &str) -> TargetResult {
        TargetResult {
            path: "source://synthetic-resource".into(), name: Some("Synthetic resource".into()),
            status: TargetStatus::Failed,
            message: json!({"status":"invalid","resourceId":"synthetic-resource","draftId":"synthetic-draft","errors":[{"code":code,"path":"$.field","message":"synthetic reason"}],"guidance":"Draft retained"}).to_string(),
            change_id: None, generation: None,
        }
    }
    #[test]
    fn validation_rejections_preserve_diagnostics_without_becoming_execution_failures() {
        for code in [
            "invalid_json",
            "invalid_type",
            "invalid_content",
            "invalid_manual_content",
            "invalid_evidence",
            "template_mismatch",
        ] {
            let result = summary(&[invalid(code)], &[]);
            assert_eq!(result["status"], "declined", "{result}");
            assert_eq!(
                result["targets"][0]["structuredResource"]["errors"][0]["code"],
                code
            );
            assert_eq!(
                result["targets"][0]["structuredResource"]["draftId"],
                "synthetic-draft"
            );
        }
        for code in [
            "storage_error",
            "permission_denied",
            "revision_conflict",
            "unknown_future_error",
        ] {
            let result = summary(&[invalid(code)], &[]);
            assert_eq!(result["status"], "failed", "{result}");
        }
        assert_eq!(
            summary(
                &[invalid("invalid_evidence"), invalid("storage_error")],
                &[]
            )["status"],
            "failed"
        );
    }
}
