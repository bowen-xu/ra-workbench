//! Freeze only authorized Docling scopes for ordinary-chat evidence editing.
use super::{
    error::AiError,
    model::PaperAiReadingContextItem,
    paper_context::{build_paper_context, persist_snapshot},
};
use rusqlite::Connection;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) struct FrozenStructuredSources {
    pub snapshot_id: String,
    pub text: String,
}

pub(crate) fn freeze_structured_sources(
    source: crate::ai::paper_reading_source::PaperReadingSource,
    db: &mut Connection,
    paper: &str,
    scopes: &[(String, Option<u32>)],
) -> Result<FrozenStructuredSources, String> {
    freeze(source, db, paper, scopes).map_err(|error| error.to_string())
}
fn freeze(
    source: crate::ai::paper_reading_source::PaperReadingSource,
    db: &mut Connection,
    paper: &str,
    scopes: &[(String, Option<u32>)],
) -> Result<FrozenStructuredSources, AiError> {
    let allowed = authorized_pages(scopes)?;
    let attachments = allowed.keys().cloned().collect::<Vec<_>>();
    let mut prepared =
        build_paper_context(source, db, paper, &attachments)?.into_reading_result(None, None);
    if prepared
        .attachment_statuses
        .iter()
        .any(|source| source.parse_status != "available")
    {
        return Err(AiError::InvalidInput(
            "授权 PDF 的 Docling 解析尚不可用".into(),
        ));
    }
    restrict_items(&mut prepared.context_items, &allowed);
    prepared.image_requests.clear();
    prepared.text_item_count = prepared
        .context_items
        .iter()
        .filter(|item| item.kind == "text")
        .count();
    prepared.table_item_count = prepared
        .context_items
        .iter()
        .filter(|item| item.kind == "table")
        .count();
    prepared.picture_item_count = prepared
        .context_items
        .iter()
        .filter(|item| item.kind == "picture")
        .count();
    let snapshot = persist_snapshot(source, db, paper, &prepared, &BTreeMap::new())?;
    let text = serde_json::to_string_pretty(&frozen_value(&snapshot.id, &prepared.context_items))
        .map_err(|_| AiError::TemporaryUnavailable)?;
    Ok(FrozenStructuredSources {
        snapshot_id: snapshot.id,
        text,
    })
}

pub(crate) type AuthorizedPages = BTreeMap<String, Option<BTreeSet<u32>>>;

pub(crate) fn authorized_pages(
    scopes: &[(String, Option<u32>)],
) -> Result<AuthorizedPages, AiError> {
    if scopes.is_empty() || scopes.len() > 256 {
        return Err(AiError::InvalidInput(
            "需要明确授权的 PDF 或页面范围".into(),
        ));
    }
    let mut allowed: BTreeMap<String, Option<BTreeSet<u32>>> = BTreeMap::new();
    for (id, page) in scopes {
        allowed
            .entry(id.clone())
            .and_modify(|pages| match page {
                None => *pages = None,
                Some(page) => {
                    if let Some(pages) = pages {
                        pages.insert(*page);
                    }
                }
            })
            .or_insert_with(|| page.map(|page| BTreeSet::from([page])));
    }
    Ok(allowed)
}

pub(crate) fn restrict_items(
    items: &mut Vec<PaperAiReadingContextItem>,
    allowed: &AuthorizedPages,
) {
    items.retain(|item| within_scope(item, allowed));
    // Ordinary source reads may freeze PDFs independently. Namespace block IDs
    // by immutable parse identity to avoid P1-T1 collisions between attachments.
    for item in items {
        item.id = format!("{}:{}", item.parse_run_id, item.id);
    }
}

pub(crate) fn frozen_value(
    snapshot: &str,
    context_items: &[PaperAiReadingContextItem],
) -> serde_json::Value {
    let items=context_items.iter().map(|item|serde_json::json!({"evidenceItemId":item.id,"attachmentId":item.attachment_id,"attachmentSha256":item.content_sha256,"parseRunId":item.parse_run_id,"rawRef":item.raw_ref,"pageIndex":item.page_index,"regions":item.regions,"kind":item.kind,"quoteExact":item.text.as_ref().cloned().or_else(||item.table_json.as_ref().map(serde_json::Value::to_string)).or_else(||item.label.clone()).unwrap_or_default()})).collect::<Vec<_>>();
    serde_json::json!({"kind":"frozen_structured_evidence_v1","snapshotId":snapshot,"items":items})
}

fn within_scope(
    item: &PaperAiReadingContextItem,
    allowed: &BTreeMap<String, Option<BTreeSet<u32>>>,
) -> bool {
    match allowed.get(&item.attachment_id) {
        None => false,
        Some(None) => true,
        Some(Some(pages)) => {
            let positions = item
                .page_index
                .into_iter()
                .chain(item.regions.iter().map(|region| region.page_index))
                .collect::<BTreeSet<_>>();
            !positions.is_empty() && positions.iter().all(|page| pages.contains(page))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::model::{PaperAiReadingRect, PaperAiReadingRegion};

    #[test]
    fn page_grants_do_not_widen_to_unshared_pages_or_attachments() {
        let mut item = PaperAiReadingContextItem {
            id: "P1-T1".into(),
            paper_id: "paper".into(),
            attachment_id: "pdf".into(),
            content_sha256: "hash".into(),
            parse_run_id: "parse".into(),
            raw_ref: "#/texts/0".into(),
            kind: "text".into(),
            label: None,
            page_index: Some(2),
            regions: vec![],
            charspan: None,
            source_text_hash: None,
            text: Some("exact".into()),
            table_json: None,
        };
        let allowed = BTreeMap::from([("pdf".into(), Some(BTreeSet::from([2])))]);
        assert!(within_scope(&item, &allowed));
        item.page_index = Some(3);
        assert!(!within_scope(&item, &allowed));
        item.page_index = Some(2);
        item.regions.push(PaperAiReadingRegion {
            page_index: 3,
            rect: PaperAiReadingRect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
        });
        assert!(!within_scope(&item, &allowed));
        item.regions.clear();
        item.attachment_id = "not-shared".into();
        assert!(!within_scope(&item, &allowed));
    }
}
