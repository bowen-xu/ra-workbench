use super::*;

pub(super) const CITING: &str = "reference-cli-citing";
pub(super) const CITED: &str = "reference-cli-cited";
pub(super) const SOURCE: &str = "reference-cli-source";
pub(super) const USAGE: &str = "reference-cli-usage";
pub(super) const NOTE: &str = "reference-cli-core-note";
const BODY: &str = "Preserved synthetic reference CLI Core Note body";

pub(super) fn note(store: &AppStore) -> Result<Value, Error> {
    let db = store.connection()?;
    Ok(serde_json::to_value(crate::notes::load_document_value(
        &db, NOTE,
    )?)?)
}

pub(super) fn draft() -> ReferenceRecordDraft {
    serde_json::from_value(json!({
        "citingPaperId":CITING,"sourceAnchorId":SOURCE,
        "rawText":"  Synthetic reference raw 引文\u{0000}tail  ",
        "fields":[
            {"name":"title","value":" Synthetic cited title ","sourceKind":"human","confirmedByUser":true},
            {"name":"doi","value":" 10.1000/synthetic ","sourceKind":"reference_text","confirmedByUser":false}
        ],"parseState":"parsed","identityState":"unverified",
        "semanticState":"unreviewed","changeReason":" manual_reference_capture "
    })).expect("fixed synthetic reference draft")
}

pub(super) fn seed(store: &AppStore) -> Result<Value, Error> {
    let mut db = store.connection()?;
    let tx = db.transaction()?;
    tx.execute_batch(
        "insert into bibliographic_items(id,item_type,title,created_at,updated_at)
         values('reference-cli-citing','journalArticle','Synthetic citing paper','before','before'),
         ('reference-cli-cited','journalArticle','Synthetic cited paper','before','before');
         insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at)
         values('cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
         'cc/synthetic-reference.pdf','application/pdf',0,'before');
         insert into attachments(id,item_id,content_sha256,display_name,created_at)
         values('reference-cli-attachment','reference-cli-citing',
         'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
         'synthetic-reference.pdf','before');
         insert into source_anchors
         (id,item_id,attachment_id,content_sha256,page_index,quote_exact,created_at)
         values('reference-cli-source','reference-cli-citing','reference-cli-attachment',
         'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
         0,'Synthetic bibliography quote','before'),
         ('reference-cli-usage','reference-cli-citing','reference-cli-attachment',
         'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
         1,'Synthetic reviewed citation use','before');",
    )?;
    crate::notes::create_page_with_paper_in(
        &tx,
        crate::notes::NotePageDraft {
            title: "Synthetic reference CLI Core Note".into(),
            parent_page_id: None,
            parent_block_position: None,
        },
        Some(CITING),
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
    let note = note(store)?;
    if note["blocks"][0]["content"]["items"][0]["text"] != BODY {
        return Err("Core writer lost synthetic reference note body".into());
    }
    Ok(note)
}
