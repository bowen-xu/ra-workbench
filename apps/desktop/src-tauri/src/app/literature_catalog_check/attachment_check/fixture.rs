use super::*;

pub(super) const PAPER: &str = "attachment-cli-paper";
pub(super) const OTHER: &str = "attachment-cli-other";
pub(super) const PARTIAL: &str = "attachment-cli-partial";
pub(super) const NOTE: &str = "attachment-cli-core-note";
const BODY: &str = "Preserved synthetic attachment CLI Core Note body";

pub(super) fn note(store: &AppStore) -> Result<Value, Error> {
    let db = store.connection()?;
    Ok(serde_json::to_value(crate::notes::load_document_value(
        &db, NOTE,
    )?)?)
}

pub(super) fn seed(store: &AppStore) -> Result<Value, Error> {
    let mut db = store.connection()?;
    let tx = db.transaction()?;
    tx.execute_batch(
        "insert into bibliographic_items(id,item_type,title,created_at,updated_at)
        values('attachment-cli-paper','document','Synthetic attachment target','before','before'),
        ('attachment-cli-other','document','Synthetic attachment other','before','before'),
        ('attachment-cli-partial','document','Synthetic partial target','before','before');
        insert into literature_search(item_id,title,full_text) values
        ('attachment-cli-paper','Synthetic attachment target','before'),
        ('attachment-cli-other','Synthetic attachment other','before'),
        ('attachment-cli-partial','Synthetic partial target','before');",
    )?;
    crate::notes::create_page_with_paper_in(
        &tx,
        crate::notes::NotePageDraft {
            title: "Synthetic attachment CLI Note".into(),
            parent_page_id: None,
            parent_block_position: None,
        },
        Some(PAPER),
        Some(NOTE),
    )?;
    let mut document = crate::notes::load_document_value(&tx, NOTE)?;
    document.blocks[0].content =
        json!({"kind":"inline","items":[{"type":"text","text":BODY,"styles":[]}]});
    crate::notes::save_document_in(
        &tx,
        crate::notes::NoteDocumentSaveRequest {
            document,
            expected_revision: 0,
            save_reason: "manual".into(),
        },
    )?;
    tx.commit()?;
    drop(db);
    let value = note(store)?;
    if value["blocks"][0]["content"]["items"][0]["text"] != BODY {
        return Err("synthetic Core writer lost body".into());
    }
    Ok(value)
}

/// A finite one-page PDF fixture with calculated xref offsets, not a file parser.
pub(super) fn pdf(label: &str) -> Vec<u8> {
    let stream = format!("BT /F1 12 Tf 20 80 Td ({label}) Tj ET\n");
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 120] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>".to_owned(),
        format!("<< /Length {} >>\nstream\n{stream}endstream",stream.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
    ];
    let mut bytes = b"%PDF-1.4\n%synthetic\n".to_vec();
    let mut offsets = vec![0];
    for (index, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
    }
    let xref = bytes.len();
    bytes.extend_from_slice(b"xref\n0 6\n0000000000 65535 f \n");
    for offset in offsets.iter().skip(1) {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!("trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    bytes
}

pub(super) fn source(
    directory: &Path,
    name: &str,
    bytes: &[u8],
) -> Result<std::path::PathBuf, Error> {
    use std::io::Write;
    let path = directory.join(name);
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?
        .write_all(bytes)?;
    Ok(path)
}

pub(super) fn stored(store: &AppStore, attachment: &str) -> Result<std::path::PathBuf, Error> {
    let db = store.connection()?;
    let relative: String = db.query_row("select o.relative_path from content_objects o join attachments a on a.content_sha256=o.sha256 where a.id=?1",[attachment],|r|r.get(0))?;
    Ok(
        crate::content::StoredObjectRead::resolve(store.content_directory(), &relative)?
            .into_legacy_path(),
    )
}
pub(super) fn draft(title: &str) -> research_workbench_native_sdk::catalog_save::CatalogDraft {
    serde_json::from_value(
        json!({"id":PAPER,"itemType":"document","title":title,"sourceKind":"pdf_filename"}),
    )
    .expect("fixed synthetic PDF draft")
}

pub(super) fn protect(store: &AppStore, attachment: &CatalogAttachment) -> Result<(), Error> {
    let mut db = store.connection()?;
    let tx = db.transaction()?;
    tx.execute("insert into source_anchors(id,item_id,attachment_id,content_sha256,page_index,quote_exact,created_at)
        values('attachment-cli-anchor',?1,?2,?3,0,'Synthetic retained source quote','before')",rusqlite::params![PAPER,attachment.id,attachment.content_sha256])?;
    tx.execute("insert into docling_parse_runs(id,item_id,attachment_id,content_sha256,service_base_url,docling_status,parse_status,raw_response,created_at,finished_at)
        values('attachment-cli-parse',?1,?2,?3,'http://127.0.0.1','done','success','{}','before','before')",rusqlite::params![PAPER,attachment.id,attachment.content_sha256])?;
    tx.execute("insert into paper_context_snapshots(id,paper_id,context_hash,context_schema_version,created_at)
        values('attachment-cli-context',?1,?2,1,'before')",rusqlite::params![PAPER,"c".repeat(64)])?;
    tx.execute("insert into paper_context_attachments(snapshot_id,ordinal,attachment_id,content_sha256,parse_run_id)
        values('attachment-cli-context',0,?1,?2,'attachment-cli-parse')",rusqlite::params![attachment.id,attachment.content_sha256])?;
    tx.commit()?;
    Ok(())
}
