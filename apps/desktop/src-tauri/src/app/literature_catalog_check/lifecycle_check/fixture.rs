use super::*;
use rusqlite::params;

pub(super) const NOTE: &str = "lifecycle-core-note";
pub(super) const BODY: &str = "Preserved synthetic lifecycle Core note body";
pub(super) const MAIN: &str = "lifecycle-main";
pub(super) const NUL_ITEM: &str = "lifecycle-nul\0tail";
pub(super) const NUL_PROJECT: &str = "lifecycle-project\0tail";
pub(super) const FAIL: &str = "lifecycle-fail";
pub(super) const PRIVATE: &str = "private synthetic lifecycle trigger fault";
const CONTENT: &str = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";

pub(super) fn note(store: &AppStore) -> Result<Value, Error> {
    let db = store.connection()?;
    Ok(serde_json::to_value(crate::notes::load_document_value(
        &db, NOTE,
    )?)?)
}

pub(super) fn seed(store: &AppStore) -> Result<Value, Error> {
    let mut db = store.connection()?;
    let tx = db.transaction()?;
    for (id, archived, trashed) in [
        (MAIN, None, None),
        (NUL_ITEM, None, None),
        (FAIL, None, None),
        ("lifecycle-batch-a", None, None),
        ("lifecycle-batch-b", None, None),
        ("lifecycle-batch-c", None, None),
        ("lifecycle-archived", Some("2026"), None),
        ("lifecycle-mixed", Some("2026"), Some("2026")),
    ] {
        tx.execute("insert into bibliographic_items(id,item_type,title,created_at,updated_at,archived_at,trashed_at)
            values(?1,'journalArticle',?1,'2026','2026',?2,?3)",params![id,archived,trashed])?;
        tx.execute(
            "insert into literature_search(item_id,title) values(?1,?1)",
            [id],
        )?;
    }
    for project in ["lifecycle-project-a", "lifecycle-project-b", NUL_PROJECT] {
        tx.execute("insert into research_projects(id,title,summary,created_at) values(?1,?1,'Synthetic lifecycle project','2026')",[project])?;
    }
    tx.execute("insert into project_items(project_id,item_id,added_at) values('lifecycle-project-a',?1,'2026')",[MAIN])?;
    tx.execute("insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at) values(?1,'synthetic-lifecycle.pdf','application/pdf',0,'2026')",[CONTENT])?;
    tx.execute("insert into attachments(id,item_id,content_sha256,display_name,created_at) values('lifecycle-attachment',?1,?2,'Synthetic lifecycle PDF','2026')",params![MAIN,CONTENT])?;
    tx.execute("insert into source_anchors(id,item_id,attachment_id,content_sha256,page_index,quote_exact,created_at) values('lifecycle-anchor',?1,'lifecycle-attachment',?2,0,'Synthetic quote','2026')",params![MAIN,CONTENT])?;
    tx.execute("insert into reading_states(item_id,attachment_id,updated_at) values(?1,'lifecycle-attachment','2026')",[MAIN])?;
    tx.execute("insert into identifiers(id,item_id,kind,normalized_value,display_value) values('lifecycle-doi',?1,'doi','synthetic-lifecycle','synthetic-lifecycle')",[MAIN])?;
    tx.execute(
        "insert into tags(id,name) values('lifecycle-tag','Synthetic lifecycle tag')",
        [],
    )?;
    tx.execute(
        "insert into item_tags(item_id,tag_id) values(?1,'lifecycle-tag')",
        [MAIN],
    )?;
    tx.execute("insert into metadata_provenance(item_id,field_name,source_kind,updated_at) values(?1,'url','user','2026')",[MAIN])?;
    crate::notes::create_page_with_paper_in(
        &tx,
        crate::notes::NotePageDraft {
            title: "Synthetic lifecycle Core note".into(),
            parent_page_id: None,
            parent_block_position: None,
        },
        Some(MAIN),
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
    tx.execute_batch(
        "create table synthetic_lifecycle_effects(item_id text not null,kind text not null);",
    )?;
    tx.commit()?;
    drop(db);
    let original = note(store)?;
    if original["blocks"][0]["content"]["items"][0]["text"] != BODY {
        return Err("Core writer lost the synthetic lifecycle note body".into());
    }
    Ok(original)
}
