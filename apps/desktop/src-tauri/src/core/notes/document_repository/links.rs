//! Reconcile derived references without replacing unchanged identities or metadata.
use super::*;
use uuid::Uuid;
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct SemanticLink {
    source_block_id: String,
    target_kind: String,
    target_id: String,
    relation_type: String,
    label_snapshot: String,
    quote_snapshot: String,
}

fn collect_inline_mentions(
    value: &serde_json::Value,
    block_id: &str,
    output: &mut Vec<SemanticLink>,
) {
    if value.get("type").and_then(serde_json::Value::as_str) == Some("mention")
        && let (Some(kind), Some(id)) = (
            value.get("targetKind").and_then(serde_json::Value::as_str),
            value.get("targetId").and_then(serde_json::Value::as_str),
        )
        && !id.is_empty()
    {
        output.push(SemanticLink {
            source_block_id: block_id.to_owned(),
            target_kind: kind.to_owned(),
            target_id: id.to_owned(),
            relation_type: "mention".to_owned(),
            label_snapshot: value
                .get("labelSnapshot")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("研究对象")
                .to_owned(),
            quote_snapshot: String::new(),
        });
    }
    if let Some(items) = value.as_array() {
        for item in items {
            collect_inline_mentions(item, block_id, output);
        }
    }
    if let Some(object) = value.as_object() {
        for item in object.values() {
            collect_inline_mentions(item, block_id, output);
        }
    }
}

fn collect_semantic_links(blocks: &[NoteDocumentBlock], output: &mut Vec<SemanticLink>) {
    for block in blocks {
        if !crate::notes::document_validation::known_block(&block.block_type) {
            collect_semantic_links(&block.children, output);
            continue;
        }
        if block
            .content
            .get("kind")
            .and_then(serde_json::Value::as_str)
            == Some("reference")
        {
            let kind = block
                .content
                .get("targetKind")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let id = block
                .content
                .get("targetId")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            if !kind.is_empty() && !id.is_empty() {
                output.push(SemanticLink {
                    source_block_id: block.id.clone(),
                    target_kind: kind.to_owned(),
                    target_id: id.to_owned(),
                    relation_type: "reference".to_owned(),
                    label_snapshot: block
                        .content
                        .get("labelSnapshot")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("研究对象")
                        .to_owned(),
                    quote_snapshot: block
                        .content
                        .get("quoteSnapshot")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("")
                        .to_owned(),
                });
            }
        }
        collect_inline_mentions(&block.content, &block.id, output);
        collect_semantic_links(&block.children, output);
    }
}

pub(super) fn replace_semantic_links(
    transaction: &Transaction<'_>,
    page_id: &str,
    blocks: &[NoteDocumentBlock],
) -> Result<(), NotesError> {
    let mut existing = {
        let mut statement = transaction.prepare(
            "select id, source_block_id, target_kind, target_id, relation_type, target_snapshot_json
             from note_links where source_page_id = ?1 and source_block_id is not null
             and relation_type in ('reference', 'mention')",
        )?;
        statement
            .query_map([page_id], |row| {
                Ok((
                    (
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ),
                    (row.get::<_, String>(0)?, row.get::<_, String>(5)?),
                ))
            })?
            .collect::<Result<HashMap<_, _>, _>>()?
    };
    let mut links = Vec::new();
    collect_semantic_links(blocks, &mut links);
    let mut seen = HashSet::new();
    fn opaque(blocks: &[NoteDocumentBlock], ids: &mut HashSet<String>) {
        for block in blocks {
            if block.block_type == "unsupported"
                || !crate::notes::document_validation::known_block(&block.block_type)
            {
                ids.insert(block.id.clone());
            }
            opaque(&block.children, ids);
        }
    }
    let mut opaque_ids = HashSet::new();
    opaque(blocks, &mut opaque_ids);
    for link in links {
        if !matches!(
            link.target_kind.as_str(),
            "note_page" | "reading_note" | "paper" | "claim" | "manuscript"
        ) {
            return Err(NotesError::InvalidInput("笔记链接目标类型无效".to_owned()));
        }
        let key = (
            link.source_block_id.clone(),
            link.target_kind.clone(),
            link.target_id.clone(),
            link.relation_type.clone(),
        );
        if !seen.insert(key.clone()) {
            continue;
        }
        let (id, raw) = existing
            .remove(&key)
            .unwrap_or_else(|| (Uuid::new_v4().to_string(), "{}".into()));
        let mut snapshot: serde_json::Value = serde_json::from_str(&raw)
            .map_err(|error| NotesError::InvalidInput(error.to_string()))?;
        let value = snapshot
            .as_object_mut()
            .ok_or_else(|| NotesError::InvalidInput("笔记链接快照无法无损更新".into()))?;
        value.insert(
            "labelSnapshot".into(),
            serde_json::json!(link.label_snapshot),
        );
        value.insert(
            "quoteSnapshot".into(),
            serde_json::json!(link.quote_snapshot),
        );
        transaction.execute(
            "insert into note_links
             (id, source_page_id, source_block_id, target_kind, target_id, relation_type, target_snapshot_json, created_at)
             values (?1, ?2, ?3, ?4, ?5, ?6, ?7, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
             on conflict(id) do update set target_snapshot_json=excluded.target_snapshot_json
             where note_links.target_snapshot_json<>excluded.target_snapshot_json",
            params![id, page_id, link.source_block_id, link.target_kind, link.target_id, link.relation_type, snapshot.to_string()],
        )?;
    }
    for (key, (id, _)) in existing {
        if !opaque_ids.contains(&key.0) {
            transaction.execute("delete from note_links where id=?1", [id])?;
        }
    }
    Ok(())
}
