use super::*;
use rusqlite::{Transaction, params};

pub(super) const NOTE: &str = "cleanup-independent-note";
pub(super) const READING_NOTE: &str = "cleanup-reading-note";
pub(super) const BODY: &str = "Preserved synthetic cleanup Core note body";
pub(super) const FIRST: &str = "cleanup-first";
pub(super) const SECOND: &str = "cleanup-second";
pub(super) const THIRD: &str = "cleanup-third";
pub(super) const RECENT: &str = "cleanup-recent";
pub(super) const ACTIVE: &str = "cleanup-independent-paper";
pub(super) const PRIVATE: &str = "private synthetic cleanup second-item trigger fault";
pub(super) const CONTENT: &str = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";

pub(super) fn notes(store: &AppStore) -> Result<Value, Error> {
    let db = store.connection()?;
    Ok(json!({(NOTE):crate::notes::load_document_value(&db,NOTE)?,
        (READING_NOTE):crate::notes::load_document_value(&db,READING_NOTE)?}))
}

fn create_note(tx: &Transaction<'_>, id: &str, paper: Option<&str>) -> Result<(), Error> {
    crate::notes::create_page_with_paper_in(
        tx,
        crate::notes::NotePageDraft {
            title: "Synthetic retained cleanup note".into(),
            parent_page_id: None,
            parent_block_position: None,
        },
        paper,
        Some(id),
    )?;
    let mut document = crate::notes::load_document_value(tx, id)?;
    document.blocks[0].content =
        json!({"kind":"inline","items":[{"type":"text","text":BODY,"styles":[]}]});
    crate::notes::save_document_in(
        tx,
        crate::notes::NoteDocumentSaveRequest {
            document,
            expected_revision: 0,
            save_reason: "manual".into(),
        },
    )?;
    Ok(())
}

pub(super) fn seed(store: &AppStore) -> Result<Value, Error> {
    let mut db = store.connection()?;
    let tx = db.transaction()?;
    for (id, title, year, date) in [
        (
            FIRST,
            "Synthetic first expired",
            None,
            Some("2000-01-01T00:00:00Z"),
        ),
        (
            SECOND,
            "Synthetic needle second expired",
            None,
            Some("2001-01-01T00:00:00Z"),
        ),
        (
            THIRD,
            "Synthetic third expired",
            Some(2024),
            Some("2002-01-01T00:00:00Z"),
        ),
        (
            RECENT,
            "Synthetic recent trash",
            None,
            Some("2999-01-01T00:00:00Z"),
        ),
        (ACTIVE, "Synthetic independent active", None, None),
    ] {
        tx.execute("insert into bibliographic_items(id,item_type,title,issued_year,created_at,updated_at,trashed_at)
            values(?1,'journalArticle',?2,?3,'2026','2026',?4)",params![id,title,year,date.filter(|_|id!=FIRST)])?;
        tx.execute(
            "insert into literature_search(item_id,title) values(?1,?2)",
            params![id, title],
        )?;
    }
    tx.execute(
        "insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at)
        values(?1,'synthetic-cleanup.pdf','application/pdf',10,'2026')",
        [CONTENT],
    )?;
    for (attachment, paper) in [
        ("cleanup-first-pdf", FIRST),
        ("cleanup-independent-pdf", ACTIVE),
    ] {
        tx.execute(
            "insert into attachments(id,item_id,content_sha256,display_name,created_at)
            values(?1,?2,?3,'Synthetic shared cleanup PDF','2026')",
            params![attachment, paper, CONTENT],
        )?;
    }
    tx.execute("insert into source_anchors(id,item_id,attachment_id,content_sha256,page_index,rects_json,quote_exact,quote_prefix,quote_suffix,created_at)
        values('cleanup-anchor',?1,'cleanup-first-pdf',?2,0,'[]','Synthetic retained quote','','','2026')",params![FIRST,CONTENT])?;
    tx.execute("insert into reading_states(item_id,attachment_id,updated_at) values(?1,'cleanup-first-pdf','2026')",[FIRST])?;
    for paper in [FIRST, SECOND, THIRD, ACTIVE] {
        let context_hash = format!("{:x}", Sha256::digest(paper.as_bytes()));
        tx.execute("insert into paper_context_snapshots(id,paper_id,context_hash,context_schema_version,created_at)
            values(?1,?2,?3,1,'2026')",params![format!("{paper}-context"),paper,context_hash])?;
        tx.execute("insert into paper_reading_profile_selection_revisions(id,paper_id,revision,profile_refs_json,created_at)
            values(?1,?2,1,'[]','2026')",params![format!("{paper}-revision"),paper])?;
    }
    create_note(&tx, NOTE, None)?;
    create_note(&tx, READING_NOTE, Some(FIRST))?;
    tx.execute(
        "insert into reading_note_page_map(legacy_id,page_id) values('cleanup-reading-alias',?1)",
        [READING_NOTE],
    )?;
    tx.execute("insert into reading_note_sources(block_id,page_id,note_type,color,current_anchor_id,paper_id_snapshot,paper_title_snapshot,attachment_id_snapshot,attachment_name_snapshot,content_sha256,page_index,rects_json,quote_exact,quote_prefix,quote_suffix,locator_version,created_at,updated_at)
        values('cleanup-reading-alias',?1,'annotation','#f4cf65','cleanup-anchor',?2,'Synthetic first expired','cleanup-first-pdf','Synthetic shared cleanup PDF',?3,0,'[]','Synthetic retained quote','','',1,'2026','2026')",params![READING_NOTE,FIRST,CONTENT])?;
    tx.execute(
        "update bibliographic_items set trashed_at='2000-01-01T00:00:00Z' where id=?1",
        [FIRST],
    )?;
    tx.commit()?;
    db.execute_batch("create temp trigger cleanup_cli_abort before delete on bibliographic_items
        when old.id='cleanup-second' begin select raise(abort,'private synthetic cleanup second-item trigger fault');end;")?;
    drop(db);
    let documents = notes(store)?;
    for id in [NOTE, READING_NOTE] {
        if documents[id]["blocks"][0]["content"]["items"][0]["text"] != BODY {
            return Err("Core writer lost retained synthetic cleanup body".into());
        }
    }
    Ok(documents)
}
