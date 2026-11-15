//! Structured PDF views freeze only this submission's authorized attachment scopes.
use super::*;
use crate::ai::{
    paper_reading_contracts::SnapshotItemGeometry,
    structured_resource_validation::{
        ExplicitReadingEvidence, Failure, FailureCause, explicit_evidence,
    },
};
use rusqlite::OptionalExtension;

/// The source reader exposes only the two extra observations this tool consumes.
pub(crate) trait FrozenReadingEvidence: ExplicitReadingEvidence {
    fn frozen_belongs(
        &self,
        db: &Connection,
        snapshot: &str,
        paper: &str,
    ) -> Result<bool, Failure<Error>>;
    fn item_geometry(
        &self,
        db: &Connection,
        snapshot: &str,
        item: &str,
    ) -> Result<Option<SnapshotItemGeometry>, Failure<Error>>;
}
impl FrozenReadingEvidence for crate::ai::paper_reading_source::PaperReadingSource {
    fn frozen_belongs(
        &self,
        db: &Connection,
        snapshot: &str,
        paper: &str,
    ) -> Result<bool, Failure<Error>> {
        // The original static predicate treats a failed membership as false.
        Ok((self.frozen_evidence_belongs)(db, snapshot, paper).unwrap_or(false))
    }
    fn item_geometry(
        &self,
        db: &Connection,
        snapshot: &str,
        item: &str,
    ) -> Result<Option<SnapshotItemGeometry>, Failure<Error>> {
        (self.item_geometry)(db, snapshot, item).map_err(|_| Failure::original(Error::Unavailable))
    }
}

/// Only native submission metadata can select an internal immutable source.
pub(crate) fn frozen_source_snapshot(
    reading_source: crate::ai::paper_reading_source::PaperReadingSource,
    db: &Connection,
    record: &SubmissionRecord,
    paper: &str,
) -> Option<String> {
    frozen_source_snapshot_with(&reading_source, db, record, paper)
        .ok()
        .flatten()
}

pub(crate) fn frozen_source_snapshot_with(
    reading: &dyn FrozenReadingEvidence,
    db: &Connection,
    record: &SubmissionRecord,
    paper: &str,
) -> Result<Option<String>, Failure<Error>> {
    let bound = db.query_row("select b.id from ai_evidence_bundles b join agent_legacy_ai_run_bindings r on r.ai_run_id=b.run_id where r.conversation_id=?1 and r.request_id=?2 limit 1",params![record.conversation_id.to_string(),record.request_id.to_string()],|row|row.get::<_,String>(0)).optional().ok().flatten();
    if let Some(id) = bound
        && reading.frozen_belongs(db, &id, paper)?
    {
        return Ok(Some(id));
    }
    for id in record
        .input
        .context
        .iter()
        .filter(|source| resource_grants::resource(source).is_some())
        .filter_map(|source| source.anchor.as_deref())
        .filter_map(|raw| serde_json::from_str::<Value>(raw).ok())
        .filter_map(|anchor| anchor["sourceSnapshotId"].as_str().map(str::to_owned))
    {
        if reading.frozen_belongs(db, &id, paper)? {
            return Ok(Some(id));
        }
    }
    Ok(None)
}

fn within_scope(file: &str, pages: &[u32], scopes: &[(String, Option<u32>)]) -> bool {
    scopes.iter().any(|(id, page)| id == file && page.is_none())
        || (!pages.is_empty()
            && pages.iter().all(|page| {
                scopes
                    .iter()
                    .any(|(id, allowed)| id == file && *allowed == Some(*page))
            }))
}

fn frozen_view(
    reading_source: crate::ai::paper_reading_source::PaperReadingSource,
    db: &Connection,
    paper: &str,
    snapshot: &str,
    file: &str,
    page: Option<u32>,
    scopes: &[(String, Option<u32>)],
) -> Result<Value, Error> {
    frozen_view_with(&reading_source, db, paper, snapshot, file, page, scopes)
        .map_err(|failure| failure.error)
}

pub(crate) fn frozen_view_with(
    reading: &dyn FrozenReadingEvidence,
    db: &Connection,
    paper: &str,
    snapshot: &str,
    file: &str,
    page: Option<u32>,
    scopes: &[(String, Option<u32>)],
) -> Result<Value, Failure<Error>> {
    let evidence = explicit_evidence(reading, db, paper, snapshot)
        .map_err(|error| error.into_failure().map_error(|_| Error::SourceUnavailable))?;
    let mut items = vec![];
    for item in evidence.items {
        if item.attachment_id != file {
            continue;
        }
        let metadata:Option<(String,String,String,String)>=db.query_row("select parse_run_id,raw_ref,item_kind,regions_json from ai_docling_evidence_context_items where evidence_item_id=?1",[&item.id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).optional().map_err(|error| Failure {
            error: Error::Unavailable,
            cause: FailureCause::SourceDatabase(Box::new(crate::runtime_plugins::native::SafeDatabaseError::sql("source-geometry", &error))),
        })?;
        let metadata = match metadata {
            Some(metadata) => Some(metadata),
            None => reading.item_geometry(db, snapshot, &item.citation_key)?,
        };
        let regions: Value = metadata
            .as_ref()
            .map(|row| serde_json::from_str(&row.3))
            .transpose()
            .map_err(|_| Failure::original(Error::Unavailable))?
            .unwrap_or_else(|| json!([]));
        let mut pages = u32::try_from(item.page_index)
            .ok()
            .into_iter()
            .collect::<Vec<_>>();
        for region in regions
            .as_array()
            .ok_or_else(|| Failure::original(Error::Unavailable))?
        {
            let region_page = region["pageIndex"]
                .as_u64()
                .and_then(|value| u32::try_from(value).ok())
                .ok_or_else(|| Failure::original(Error::Unavailable))?;
            pages.push(region_page);
        }
        if !within_scope(file, &pages, scopes)
            || page.is_some_and(|page| pages.iter().any(|position| *position != page))
        {
            continue;
        }
        items.push(json!({"evidenceItemId":item.citation_key,"attachmentId":item.attachment_id,"attachmentSha256":item.content_sha256,"parseRunId":metadata.as_ref().map(|row|&row.0),"rawRef":metadata.as_ref().map(|row|&row.1),"pageIndex":item.page_index,"regions":regions,"kind":metadata.as_ref().map_or("text",|row|row.2.as_str()),"quoteExact":item.quote_exact}));
    }
    Ok(json!({"kind":"frozen_structured_evidence_v1","snapshotId":snapshot,"items":items}))
}

impl ResourceTools {
    pub(super) fn structured_pdf_source(
        &self,
        db: &mut Connection,
        entry: &Entry,
    ) -> Result<ContextText, Error> {
        let Some(Resource::Paper {
            resource_id: paper,
            attachment_id: Some(file),
            page_index,
        }) = &entry.resource
        else {
            return Err(Error::InvalidInput);
        };
        let mut scopes = self
            .entries(db)?
            .into_iter()
            .filter_map(|entry| match entry.resource {
                Some(Resource::Paper {
                    resource_id,
                    attachment_id: Some(attachment),
                    page_index,
                }) if resource_id == *paper => Some((attachment, page_index)),
                _ => None,
            })
            .collect::<Vec<_>>();
        scopes.sort();
        scopes.dedup();
        let (snapshot, value) = if let Some(snapshot) = frozen_source_snapshot(
            self.store
                .require_paper_reading_source()
                .map_err(|_| Error::SourceUnavailable)?,
            db,
            &self.record,
            paper,
        ) {
            let value = frozen_view(
                self.store
                    .require_paper_reading_source()
                    .map_err(|_| Error::SourceUnavailable)?,
                db,
                paper,
                &snapshot,
                file,
                *page_index,
                &scopes,
            )?;
            (snapshot, value)
        } else {
            let frozen = crate::ai::freeze_structured_sources(
                self.store
                    .require_paper_reading_source()
                    .map_err(|_| Error::SourceUnavailable)?,
                db,
                paper,
                &scopes,
            )
            .map_err(|_| Error::SourceUnavailable)?;
            let mut value: Value =
                serde_json::from_str(&frozen.text).map_err(|_| Error::Unavailable)?;
            value["items"]
                .as_array_mut()
                .ok_or(Error::Unavailable)?
                .retain(|item| {
                    item["attachmentId"] == *file
                        && page_index.is_none_or(|page| {
                            item["pageIndex"] == page
                                && item["regions"].as_array().is_some_and(|regions| {
                                    regions.iter().all(|region| region["pageIndex"] == page)
                                })
                        })
                });
            (frozen.snapshot_id, value)
        };
        let proof = json!({"kind":"structuredEvidenceRead","paperId":paper,"snapshotId":snapshot,"scopes":scopes});
        let path = research_agent_core::text_patch::address::object(
            research_agent_core::text_patch::address::SourceRoot::Library,
            std::slice::from_ref(paper),
        );
        db.execute("insert into agent_text_read_scopes(conversation_id,scope_request_id,path,version,snapshot_json) values(?1,?2,?3,?4,?5) on conflict(conversation_id,scope_request_id,path) do update set version=excluded.version,snapshot_json=excluded.snapshot_json",params![self.record.conversation_id.to_string(),self.record.request_id.to_string(),path,snapshot,proof.to_string()]).map_err(|_|Error::Unavailable)?;
        let mut source = entry.identity.clone();
        source.text = value.to_string();
        source.revision = snapshot;
        Ok(source)
    }
}
