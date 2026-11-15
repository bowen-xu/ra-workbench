//! Structured resources adapt the shared text tools; JSON validity never discards a draft.
use super::*;
use crate::agent::context_session::{
    configuration,
    sources::{Access, Resource},
    structured as scope,
};
use rusqlite::{Connection, OptionalExtension, params};

#[derive(Clone)]
pub(super) struct StructuredBackend {
    pub store: AppStore,
    pub record: SubmissionRecord,
}

pub(crate) fn read_view(
    store: &AppStore,
    record: &SubmissionRecord,
    db: &Connection,
    id: &str,
) -> Result<TextSnapshot, String> {
    let resource =
        crate::structured_resources::get_header(db, id).map_err(|error| error.to_string())?;
    let snapshot = StructuredBackend {
        store: store.clone(),
        record: record.clone(),
    }
    .snapshot(db, &resource.source_uri, false)?;
    journal::remember(db, record, &snapshot)?;
    Ok(snapshot)
}

impl StructuredBackend {
    fn target(&self, db: &Connection, path: &str) -> Result<String, String> {
        match addresses::resolve(db, &self.record, path)? {
            addresses::Target::Structured(id) => Ok(id),
            addresses::Target::TemplateCatalog => Ok(scope::TEMPLATE_CATALOG_ID.into()),
            _ => Err("source is not a structured resource".into()),
        }
    }
    fn authorize(&self, db: &Connection, id: &str, write: bool) -> Result<(), String> {
        let active: bool = db
            .query_row(
                "select exists(select 1 from agent_conversations where id=?1 and archived=0)",
                [self.record.conversation_id.to_string()],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        let finished: bool = db.query_row("select exists(select 1 from agent_items where conversation_id=?1 and json_extract(payload_json,'$.payload.type')='turnFinished' and json_extract(payload_json,'$.payload.turnId')=?2)", params![self.record.conversation_id.to_string(),self.record.turn_id.to_string()], |row| row.get(0)).map_err(|error|error.to_string())?;
        if !active || finished || !scope::access(db, &self.record, id, write)? {
            return Err("structured resource is outside the active submission access".into());
        }
        if write {
            super::structured_freeze::check_binding(db, &self.record, id)?;
        }
        Ok(())
    }
    fn snapshot(&self, db: &Connection, path: &str, saved: bool) -> Result<TextSnapshot, String> {
        let id = self.target(db, path)?;
        self.authorize(db, &id, false)?;
        if id == scope::TEMPLATE_CATALOG_ID {
            return Err("template directory has no text body; use source discovery and Add File to create a private draft".into());
        }
        let read = super::structured_freeze::read(db, &self.record, &id, saved)?;
        Ok(TextSnapshot {
            path: path.into(),
            name: Some(read.resource.title),
            text: read.text,
            version: format!(
                "structured:{}:{}:{}",
                read.resource.revision,
                read.draft.as_ref().map_or("", |draft| draft.id.as_str()),
                read.draft.as_ref().map_or(0, |draft| draft.revision)
            ),
        })
    }
    fn preflight_in(&self, db: &Connection, changes: &[TextChange]) -> Result<(), String> {
        for change in changes {
            let id = self.target(db, &change.path)?;
            self.authorize(db, &id, true)?;
            if change.destination.is_some() || change.after.is_none() {
                return Err("structured objects cannot be moved or deleted by text patches".into());
            }
            if change.before.is_none() {
                if id != scope::TEMPLATE_CATALOG_ID {
                    return Err("Add File requires the authorized template directory; existing objects use Update File".into());
                }
                if configuration::read_in(db, self.record.conversation_id)
                    .map_err(|error| error.to_string())?
                    .is_some_and(|configuration| {
                        ["persistent", "temporary"]
                            .into_iter()
                            .map(|key| configuration.state[key].as_array().map_or(0, Vec::len))
                            .sum::<usize>()
                            >= 32
                    })
                {
                    return Err("context selection is full; remove a resource before creating a template draft".into());
                }
            } else {
                let current = self.snapshot(db, &change.path, false)?;
                if change.before.as_ref().is_none_or(|before| {
                    before.version != current.version || before.text != current.text
                }) {
                    return Err(
                        "structured object changed after patch preparation; read again".into(),
                    );
                }
            }
        }
        Ok(())
    }
    fn source_snapshot(&self, db: &Connection, paper: Option<&str>) -> Option<String> {
        let reading = self.store.paper_reading_source()?;
        self.source_snapshot_with(db, paper, &reading)
    }
    fn source_snapshot_with(
        &self,
        db: &Connection,
        paper: Option<&str>,
        reading: &dyn crate::agent::context_session::resource_tools::FrozenReadingEvidence,
    ) -> Option<String> {
        let paper = paper?;
        crate::agent::context_session::resource_tools::frozen_source_snapshot_with(reading, db,&self.record,paper).ok().flatten()
            .or_else(|| {
                let path=research_agent_core::text_patch::address::object(research_agent_core::text_patch::address::SourceRoot::Library, &[paper.to_owned()]);
                let proof:String=db.query_row("select snapshot_json from agent_text_read_scopes where conversation_id=?1 and scope_request_id=?2 and path=?3",params![self.record.conversation_id.to_string(),self.record.request_id.to_string(),path],|row|row.get(0)).optional().ok()??;
                let proof:Value=serde_json::from_str(&proof).ok()?;
                (proof["kind"]=="structuredEvidenceRead" && proof["paperId"]==paper).then(||proof["snapshotId"].as_str().map(str::to_owned)).flatten().filter(|id|reading.snapshot_hash(db,id,paper).map(|value|value.is_some()).unwrap_or(false))
            })
    }
}

#[path = "structured/backend.rs"]
mod backend;
#[path = "structured/native.rs"]
mod native;
#[path = "structured/receipt.rs"]
mod receipt;
#[path = "structured/write.rs"]
mod write;

fn target_result(
    db: &Connection,
    outcome: crate::structured_resources::WriteResult,
    path: String,
) -> TargetResult {
    let resource = &outcome.resource;
    let status = match outcome.status.as_str() {
        "applied" => TargetStatus::Applied,
        _ => TargetStatus::Failed,
    };
    let mut feedback = json!({"status":outcome.status,"resourceId":resource.id,"sourceUrl":resource.source_uri,"revision":outcome.resource.revision,"draftId":outcome.draft.as_ref().map(|draft|&draft.id),"draftRevision":outcome.draft.as_ref().map(|draft|draft.revision),"errors":outcome.errors,"receipt":outcome.receipt,"guidance":if status==TargetStatus::Applied {"Saved with pending review. Keep/Undo remains available."} else {"Draft retained. Read candidate view and patch the same source address to repair the listed errors."}});
    if status == TargetStatus::Applied {
        let before = if resource.revision == 1 {
            Ok(None)
        } else {
            crate::structured_resources::get_version(db, &resource.id, resource.revision - 1)
                .map(|version| version.content)
        };
        feedback["savedDiff"] = match before {
            Ok(before) => patch_preview::saved_structured(
                &path,
                &resource.title,
                before.as_ref(),
                resource.content.as_ref(),
            ),
            Err(error) => {
                json!({"path":path,"name":resource.title,"comparison":"saved","omitted":true,"omissionReason":"无法读取当次保存前快照（snapshot_unavailable）。","diagnostic":{"code":"snapshot_unavailable","cause":error.to_string()}})
            }
        };
    }
    TargetResult {
        path,
        name: Some(resource.title.clone()),
        status,
        message: feedback.to_string(),
        change_id: outcome
            .receipt
            .as_ref()
            .map(|receipt| receipt.request_id.clone()),
        generation: None,
    }
}

fn has_new_citations(before: Option<&Value>, draft: Option<&str>, after: &str) -> bool {
    fn ids(value: &Value, output: &mut std::collections::HashSet<String>) {
        match value {
            Value::Array(values) => {
                for value in values {
                    ids(value, output)
                }
            }
            Value::Object(values) => {
                for (key, value) in values {
                    if key == "evidenceItemId" {
                        if let Some(id) = value.as_str() {
                            output.insert(id.into());
                        }
                    } else if key == "evidenceItemIds" {
                        if let Some(values) = value.as_array() {
                            output
                                .extend(values.iter().filter_map(Value::as_str).map(str::to_owned));
                        }
                    } else {
                        ids(value, output);
                    }
                }
            }
            _ => {}
        }
    }
    let mut known = std::collections::HashSet::new();
    if let Some(before) = before {
        ids(before, &mut known);
    }
    if let Some(draft) = draft.and_then(|draft| serde_json::from_str::<Value>(draft).ok()) {
        ids(&draft, &mut known);
    }
    let mut proposed = std::collections::HashSet::new();
    if let Ok(after) = serde_json::from_str::<Value>(after) {
        ids(&after, &mut proposed);
    }
    !proposed.is_subset(&known)
}

pub(super) fn read_metadata(
    store: &AppStore,
    record: &SubmissionRecord,
    path: &str,
    saved: bool,
    result: &mut Value,
) -> Result<(), String> {
    if !path.starts_with("source://") {
        return Ok(());
    }
    let db = store.connection().map_err(|error| error.to_string())?;
    if let Ok(addresses::Target::Structured(id)) = addresses::resolve(&db, record, path) {
        let read = super::structured_freeze::read(&db, record, &id, saved)?;
        let version = format!(
            "structured:{}:{}:{}",
            read.resource.revision,
            read.draft.as_ref().map_or("", |draft| draft.id.as_str()),
            read.draft.as_ref().map_or(0, |draft| draft.revision)
        );
        if result["version"] != version {
            return Err("structured object changed while reading; read again".into());
        }
        result["resourceId"] = json!(id);
        result["resourceType"] = json!(read.resource.kind);
        result["savedRevision"] = json!(read.resource.revision);
        result["pendingFieldIds"] = json!(read.resource.pending_field_ids);
        result["reviewedFieldIds"] = json!(read.resource.reviewed_field_ids);
        if let Some(template) = read.resource.template_ref.as_ref() {
            let header = crate::structured_resources::get_header(&db, &template.resource_id)
                .map_err(|error| error.to_string())?;
            result["templateRef"] = json!({"resourceId":template.resource_id,"revision":template.revision,"sourceUrl":header.source_uri});
        }
        let reference = match read.resource.kind.as_str() {
            "portrait" => "references/portrait.md",
            "structuredReading" => "references/reading.md",
            _ => "references/templates.md",
        };
        result["format"] = json!({"mediaType":"application/json","schemaVersion":1,"skillSourceId":".agents/skills/edit-structured-resources/SKILL.md","reference":reference,"sourceInstructions":"For new citations, read an authorized PDF using research_read_source with view=structured to obtain exact evidenceItemId values and original quotes. Unshared paper history cannot authorize new evidence.","instructions":"Load the skill and its listed reference for the exact format. Preserve required fields and source references. Patch the candidate view. Valid changes are autosaved as pending review differences; only the user resolves them with Keep or Undo. Invalid text is retained for repair."});
        if let Some(draft) = read.draft {
            result["draft"] = json!({"id":draft.id,"revision":draft.revision,"baseRevision":draft.base_revision,"errors":draft.errors});
        }
    }
    Ok(())
}

/// Invalid submitted content is declined; storage/permission/revision faults
/// remain failed even when an older write receipt also calls them "invalid".
pub(super) fn validation_declined(feedback: &Value) -> bool {
    feedback["status"] == "invalid"
        && feedback["resourceId"].is_string()
        && feedback["errors"].as_array().is_some_and(|errors| {
            !errors.is_empty()
                && errors.iter().all(|issue| {
                    matches!(
                        issue["code"].as_str(),
                        Some(
                            "invalid_json"
                                | "invalid_type"
                                | "invalid_content"
                                | "invalid_manual_content"
                                | "invalid_evidence"
                                | "template_mismatch"
                        )
                    )
                })
        })
}
pub(super) fn public_feedback(result: &mut Value) {
    if let Some(targets) = result["targets"].as_array_mut() {
        for target in targets {
            if let Some(mut feedback) = target["message"]
                .as_str()
                .and_then(|message| serde_json::from_str::<Value>(message).ok())
                .filter(|value| value["resourceId"].is_string())
            {
                // Frozen display belongs in bounded fileDiffs, not duplicated
                // inside the model-facing structured-resource diagnostics.
                if let Some(object) = feedback.as_object_mut() {
                    object.remove("savedDiff");
                }
                target["structuredResource"] = feedback.clone();
                target["message"] = feedback["guidance"].clone();
            }
        }
    }
}

/// Reverse only our public structured-resource projection before consuming the
/// strict shared target contract. Other extra target fields remain invalid.
pub(super) fn decode_targets(mut value: Value) -> Result<Vec<TargetResult>, String> {
    for target in value.as_array_mut().ok_or("invalid patch target list")? {
        if let Some(feedback) = target
            .as_object_mut()
            .and_then(|target| target.remove("structuredResource"))
        {
            if !feedback["resourceId"].is_string()
                || !feedback["guidance"].is_string()
                || feedback["guidance"] != target["message"]
            {
                return Err("invalid owned structured result projection".into());
            }
            target["message"] = json!(feedback.to_string());
        }
    }
    serde_json::from_value(value).map_err(|_| "invalid patch target result".into())
}
