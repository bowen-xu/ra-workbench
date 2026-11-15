//! Exact synthetic rows only; production migrations and Core writers remain owners.
use super::*;
use rusqlite::params;

pub(super) const NOTE: &str = "duplicate-merge-core-note";
pub(super) const BODY: &str = "Preserved synthetic Core-owned duplicate merge body";
const CONTENT: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

pub(super) struct Case {
    pub candidate: String,
    pub survivor: String,
    pub source: String,
}

impl Case {
    pub(super) fn id(&self, suffix: &str) -> String {
        format!("merge-main-{suffix}")
    }
}

pub(super) fn case() -> Case {
    Case {
        candidate: "merge-main-candidate".into(),
        survivor: "merge-main-survivor".into(),
        source: "merge-main-source".into(),
    }
}

pub(super) fn note(store: &AppStore) -> Result<Value, Error> {
    let db = store.connection()?;
    Ok(serde_json::to_value(crate::notes::load_document_value(
        &db, NOTE,
    )?)?)
}

pub(super) fn seed(store: &AppStore) -> Result<Value, Error> {
    let case = case();
    let mut db = store.connection()?;
    let tx = db.transaction()?;
    crate::notes::create_page_with_paper_in(
        &tx,
        crate::notes::NotePageDraft {
            title: "Synthetic duplicate merge Core note".into(),
            parent_page_id: None,
            parent_block_position: None,
        },
        None,
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
    tx.execute("insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at) values(?1,'synthetic-merge.pdf','application/pdf',0,'2026')",[CONTENT])?;
    for (id, title) in [
        (&case.survivor, "Synthetic merge survivor"),
        (&case.source, "Synthetic merge source"),
    ] {
        tx.execute("insert into bibliographic_items(id,item_type,title,created_at,updated_at) values(?1,'journalArticle',?2,'2026','2026')",params![id,title])?;
        tx.execute(
            "insert into literature_search(item_id,title) values(?1,?2)",
            params![id, title],
        )?;
    }
    tx.execute("insert into attachments(id,item_id,content_sha256,display_name,created_at) values(?1,?2,?3,'Synthetic merge PDF','2026')",params![case.id("attachment"),case.source,CONTENT])?;
    tx.execute("insert into source_anchors(id,item_id,attachment_id,content_sha256,page_index,quote_exact,created_at) values(?1,?2,?3,?4,0,'Synthetic merge quote','2026')",params![case.id("anchor"),case.source,case.id("attachment"),CONTENT])?;
    tx.execute(
        "insert into reading_states(item_id,attachment_id,updated_at) values(?1,?2,'2026')",
        params![case.source, case.id("attachment")],
    )?;
    tx.execute("insert into identifiers(id,item_id,kind,normalized_value,display_value) values(?1,?2,'doi',?3,?3)",params![case.id("identifier"),case.source,case.id("doi")])?;
    tx.execute("insert into research_projects(id,title,summary,created_at) values(?1,?1,'Synthetic merge scope','2026')",[case.id("project")])?;
    tx.execute(
        "insert into project_items(project_id,item_id,added_at) values(?1,?2,'2026')",
        params![case.id("project"), case.source],
    )?;
    tx.execute("insert into tags(id,name) values(?1,?1)", [case.id("tag")])?;
    tx.execute(
        "insert into item_tags(item_id,tag_id) values(?1,?2)",
        params![case.source, case.id("tag")],
    )?;
    tx.execute("insert into metadata_provenance(item_id,field_name,source_kind,updated_at) values(?1,'url','user','2026')",[&case.source])?;
    tx.execute("insert into duplicate_candidates(id,left_item_id,right_item_id,reason,confidence,created_at) values(?1,?2,?3,'doi',0.75,'2026')",params![case.candidate,case.survivor,case.source])?;
    let paper = json!({"kind":"paper","id":case.source}).to_string();
    tx.execute(
        "insert into nal_statements(id,subject,predicate,origin) values(?1,?2,?2,'human')",
        params![case.id("statement"), paper],
    )?;
    tx.execute(
        "insert into nal_annotations(id,statement_id,note_page_id) values(?1,?2,?3)",
        params![case.id("annotation"), case.id("statement"), NOTE],
    )?;
    tx.execute("insert into reading_note_sources(block_id,page_id,note_type,color,current_anchor_id,paper_id_snapshot,paper_title_snapshot,attachment_id_snapshot,attachment_name_snapshot,content_sha256,page_index,rects_json,quote_exact,quote_prefix,quote_suffix,locator_version,created_at,updated_at)
        values('merge-reading-source',?1,'annotation','#f4cf65',?2,?3,'Synthetic merge source',?4,'Synthetic merge PDF',?5,0,'[]','Synthetic merge quote','','',1,'2026','2026')",
        params![NOTE,case.id("anchor"),case.source,case.id("attachment"),CONTENT])?;
    if tx.execute("update ai_provider_settings set enabled=1,region='cn-beijing',model_id='text-embedding-v4' where provider_id='qwen-embedding'",[])? != 1 {
        return Err("synthetic embedding configuration row missing".into());
    }
    tx.commit()?;
    db.execute_batch("create temp trigger merge_cli_dynamic_collection after update of archived_at on bibliographic_items
        when new.id='merge-main-source' and old.archived_at is null and new.archived_at is not null
        and not exists(select 1 from collections where id='merge-dynamic-collection') begin
        insert into collections(id,name,description) values('merge-dynamic-collection','Dynamic merge source collection','Actual T1 collection description');
        insert into collection_memberships(item_id,collection_id) values('merge-main-source','merge-dynamic-collection'); end;
        create temp trigger merge_cli_term_metadata after insert on terms when new.text='Dynamic merge source collection' begin
        update terms set revision=8,provenance_kind='ai_accepted_after_edit' where id=new.id; end;
        create temp trigger merge_cli_concept_metadata after insert on concepts when new.label='Dynamic merge source collection' begin
        update concepts set revision=7,provenance_kind='ai_accepted_after_edit' where id=new.id; end;")?;
    drop(db);
    let note = note(store)?;
    if note["blocks"][0]["content"]["items"][0]["text"] != BODY {
        return Err("Core writer did not preserve the synthetic merge note body".into());
    }
    Ok(note)
}
