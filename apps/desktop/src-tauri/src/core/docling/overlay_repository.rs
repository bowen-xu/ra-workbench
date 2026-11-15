//! Read only the requested page at the SQLite boundary. Full parse/audit export
//! keeps its separate detail API; an overlay never hydrates raw_response.
use rusqlite::{Connection, OptionalExtension};
use serde_json::Value;

use super::{
    error::DoclingError,
    model::DoclingPageOverlay,
    overlay::{ITEM_COLLECTIONS, build_document_overlay},
};

pub(super) fn read_page(
    db: &Connection,
    attachment_id: &str,
    page_index: u32,
    requested_run: Option<&str>,
) -> Result<DoclingPageOverlay, DoclingError> {
    let tx = db.unchecked_transaction()?;
    let requested = requested_run.filter(|id| !id.trim().is_empty());
    let row: Option<(String, String, bool)> = match requested {
        Some(id) => tx.query_row(
            "select id,attachment_id,json_content is not null from docling_parse_runs where id=?1",
            [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ).optional()?,
        None => tx.query_row(
            "select id,attachment_id,json_content is not null from docling_parse_runs
             where attachment_id=?1 and parse_status in ('success','partial_success') and json_content is not null
             order by created_at desc,finished_at desc limit 1",
            [attachment_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ).optional()?,
    };
    let (run_id, owner, has_document) = row.ok_or(DoclingError::ParseRunNotFound)?;
    if owner != attachment_id {
        return Err(DoclingError::Attachment(
            "Docling 解析记录不属于当前附件".into(),
        ));
    }
    if !has_document {
        return Err(DoclingError::InvalidResponse(
            "Docling 解析记录没有 json_content".into(),
        ));
    }
    let document = page_document(&tx, &run_id, page_index)?;
    let overlay = build_document_overlay(&run_id, attachment_id, page_index, &document);
    tx.commit()?;
    Ok(overlay)
}

fn page_document(db: &Connection, run_id: &str, page_index: u32) -> Result<Value, DoclingError> {
    // MATERIALIZED JSONB parses the source once per statement; only the small
    // page projection crosses into Rust. No persisted JSON format is changed.
    // Collection identifiers below are the shared fixed allowlist, never input.
    let mut fields = vec![String::from(
        "'pages',json((select json_group_array(json(p.value)) from doc,json_each(doc.value,'$.pages') p where p.type='object' and json_type(case when p.type='object' then p.value else '{}' end,'$.page_no')='integer' and json_extract(case when p.type='object' then p.value else '{}' end,'$.page_no')=?2))",
    )];
    for (collection, _) in ITEM_COLLECTIONS {
        fields.push(format!(
            "'{collection}',json((select json_group_array(json(case when json_type(obj,'$.self_ref')='text' then obj else json_set(obj,'$.self_ref','#/{collection}/'||original_index) end))
             from (select item.key as original_index,case when item.type='object' then item.value else '{{}}' end as obj
               from doc,json_each(case when json_type(doc.value,'$.{collection}')='array' then json_extract(doc.value,'$.{collection}') else '[]' end) item)
             where exists(select 1 from json_each(case when json_type(obj,'$.prov')='array' then json_extract(obj,'$.prov') else '[]' end) prov
               where prov.type='object' and json_type(case when prov.type='object' then prov.value else '{{}}' end,'$.page_no')='integer'
               and json_extract(case when prov.type='object' then prov.value else '{{}}' end,'$.page_no')=?2)))"
        ));
    }
    let sql = format!(
        "with doc as materialized(select jsonb(json_content) as value from docling_parse_runs where id=?1)
         select json_object({}) from doc", fields.join(",")
    );
    let projected: String = db
        .query_row(
            &sql,
            rusqlite::params![run_id, u64::from(page_index) + 1],
            |row| row.get(0),
        )
        .map_err(|cause| {
            if cause.to_string().contains("malformed JSON") {
                DoclingError::InvalidResponse(format!(
                    "docling_overlay_invalid_json：run={run_id}；{cause}"
                ))
            } else {
                cause.into()
            }
        })?;
    serde_json::from_str(&projected)
        .map_err(|cause| DoclingError::InvalidResponse(cause.to_string()))
}

#[cfg(test)]
#[path = "overlay_repository_tests.rs"]
mod tests;
