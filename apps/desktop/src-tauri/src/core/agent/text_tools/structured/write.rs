//! Original journal admission, write preparation and receipt projection shared by both routes.
use super::*;
use crate::agent::context_session::resource_tools::FrozenReadingEvidence;

pub(super) struct PreparedTarget {
    pub path: String,
    pub created: bool,
}

pub(super) struct PreparedWrite {
    pub target: PreparedTarget,
    pub draft: crate::structured_resources::WriteDraft,
}

impl StructuredBackend {
    pub(super) fn begin_commit(
        &self,
        db: &Connection,
        request_id: &str,
        changes: &[TextChange],
    ) -> Result<Option<Vec<TargetResult>>, String> {
        let input = serde_json::to_string(changes).map_err(|error| error.to_string())?;
        match journal::lookup_sql(db, self.record.conversation_id, request_id, &input) {
            Ok(Some(result)) if result["status"] != "unknown" => {
                return serde_json::from_value(result["targets"].clone())
                    .map(Some)
                    .map_err(|_| "invalid structured target receipt".into());
            }
            Ok(Some(_)) => {
                return Ok(Some(patch_journal::unknown_changes(
                    changes,
                    &patch_journal::unavailable_receipt(&self.record, request_id, None),
                )));
            }
            Ok(None) => {}
            Err(error) if error.uncertain() => {
                let mut detail = error.detail(
                    self.record.conversation_id,
                    Some(self.record.request_id),
                    request_id,
                );
                patch_journal::target_identity(&mut detail, &self.record, request_id);
                return Ok(Some(patch_journal::unknown_changes(changes, &detail)));
            }
            Err(error) => return Err(error.target_message(&self.record, request_id)),
        }
        self.preflight_in(db, changes)?;
        journal::insert_sql(db, self.record.conversation_id, request_id, &input)
            .map_err(|error| error.target_message(&self.record, request_id))?;

        Ok(None)
    }
    pub(super) fn prepare_write(
        &self,
        db: &Connection,
        request_id: &str,
        change: TextChange,
        results: &mut Vec<TargetResult>,
        reading: Option<&dyn FrozenReadingEvidence>,
    ) -> Result<Option<PreparedWrite>, String> {
        let target = self.target(db, &change.path)?;
        let created = target == scope::TEMPLATE_CATALOG_ID;
        let body = change.after.ok_or("structured resource requires content")?;
        let operation = format!("text:{:x}", Sha256::digest(request_id));
        let resource = if created {
            let title = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|value| {
                    value["name"]
                        .as_str()
                        .or(value["title"].as_str())
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| "新阅读模板".into());
            match crate::structured_resources::create_template(
                db,
                &title,
                &format!("create:{operation}"),
                Some(&self.record.conversation_id.to_string()),
            ) {
                Ok(resource) => resource,
                Err(error) => {
                    let mut detail = receipt::uncertain(
                        &self.record,
                        request_id,
                        "structured-create",
                        Some(&error),
                    );
                    // Invalid creation input is a pre-effect rejection.
                    // A database failure remains uncertain even when a
                    // later query cannot find its creation receipt.
                    let absent = matches!(&error, crate::structured_resources::Error::Invalid(_)) && db.is_autocommit() && db.query_row(
                                "select not exists(select 1 from structured_resource_creations where request_id=?1)",
                                [format!("create:{operation}")], |row|row.get::<_,bool>(0),
                            ).is_ok_and(|absent|absent);
                    if absent {
                        detail["code"] = json!("TEXT_PATCH_EFFECT_REJECTED");
                        detail["safeCause"] = json!("structured_creation_rejected");
                        detail["cause"] = json!(error.to_string());
                        results.push(TargetResult {
                            path: change.path,
                            name: None,
                            status: TargetStatus::Failed,
                            message: detail.to_string(),
                            change_id: None,
                            generation: None,
                        });
                    } else {
                        receipt::append_unknown(results, change.path, detail);
                    }
                    return Ok(None);
                }
            }
        } else {
            crate::structured_resources::get(db, &target).map_err(|error| error.to_string())?
        };
        let read = match crate::structured_resources::read_text(
            db,
            &resource.id,
            Some(&self.record.conversation_id.to_string()),
            false,
        ) {
            Ok(read) => read,
            Err(error) if created => {
                receipt::append_unknown(
                    results,
                    change.path,
                    receipt::uncertain(&self.record, request_id, "structured-read", Some(&error)),
                );
                return Ok(None);
            }
            Err(error) => return Err(error.to_string()),
        };
        let mut snapshot = match reading {
            Some(reading) => self.source_snapshot_with(db, resource.paper_id.as_deref(), reading),
            None => self.source_snapshot(db, resource.paper_id.as_deref()),
        };
        if resource.kind != "readingTemplate"
            && snapshot.is_none()
            && has_new_citations(
                resource.content.as_ref(),
                read.draft.as_ref().map(|draft| draft.content.as_str()),
                &body,
            )
        {
            // An explicit missing snapshot makes validation retain the raw
            // draft instead of consulting unrelated paper history.
            snapshot = Some(format!("unread-source:{}", self.record.request_id));
        }

        Ok(Some(PreparedWrite {
            target: PreparedTarget {
                path: change.path,
                created,
            },
            draft: crate::structured_resources::WriteDraft {
                resource_id: resource.id.clone(),
                expected_revision: resource.revision,
                content: body,
                request_id: operation,
                conversation_id: Some(self.record.conversation_id.to_string()),
                expected_draft_revision: read.draft.as_ref().map(|draft| draft.revision),
                source_snapshot_id: snapshot,
                submission_id: Some(self.record.request_id.to_string()),
                editor_kind: "agent".into(),
            },
        }))
    }
    pub(super) fn publish_write(
        &self,
        db: &Connection,
        request_id: &str,
        prepared: PreparedTarget,
        outcome: crate::structured_resources::WriteResult,
        results: &mut Vec<TargetResult>,
    ) -> bool {
        let created = prepared.created;
        let change = prepared;
        if created
            && configuration::add_created_in(
                db,
                self.record.conversation_id,
                &Resource::StructuredObject {
                    resource_id: outcome.resource.id.clone(),
                    access: Some(Access::ReadWrite),
                },
                self.store.writing_resource_source(),
            )
            .is_err()
        {
            receipt::append_unknown(
                results,
                change.path,
                receipt::uncertain(&self.record, request_id, "structured-scope", None),
            );
            return false;
        }
        self.store.notify_structured_resource_changed(
            &outcome.resource.id,
            outcome.resource.paper_id.as_deref(),
        );
        results.push(target_result(db, outcome, change.path));
        true
    }
    pub(super) fn finish_commit(
        &self,
        db: &Connection,
        request_id: &str,
        mut results: Vec<TargetResult>,
    ) -> Vec<TargetResult> {
        if let Err(error) = journal::record_sql(
            db,
            self.record.conversation_id,
            request_id,
            &json!({"targets":results}),
            !results
                .iter()
                .any(|target| target.status == TargetStatus::Unknown),
        ) {
            let detail = patch_journal::target_diagnostic(
                &self.record,
                request_id,
                "target-terminal",
                &error,
            );
            receipt::lost_receipt(&mut results, &detail);
        }
        results
    }
}
