use super::*;
use rusqlite::{functions::FunctionFlags, params};
use std::sync::atomic::{AtomicUsize, Ordering};

pub(super) const PLACEHOLDER: &str = "bootstrap-obsolete";
pub(super) const REPAIR: &str = "bootstrap-repair";
pub(super) const BAD: &str = "bootstrap-invalid-fact";
pub(super) const EXPLANATION: &str = "从旧集合关系迁移。反向显示：related-field；有向：false。";

pub(super) fn seed(store: &AppStore) -> Result<(String, Arc<AtomicUsize>), Error> {
    let mut db = store.connection()?;
    let tx = db.transaction()?;
    for (id, text, normalized) in [
        ("bootstrap-obsolete-term", "related-field", "related-field"),
        (
            "bootstrap-endpoint-term",
            "Synthetic original endpoint",
            "synthetic original endpoint",
        ),
        (
            "bootstrap-existing-term",
            "Synthetic collection A",
            "synthetic collection a",
        ),
    ] {
        tx.execute("insert into terms(id,text,normalized_text,language,provenance_kind,revision,created_at,updated_at)
            values(?1,?2,?3,'','human_original',1,'1999-01-02T03:04:05.000Z','2000-02-03T04:05:06.000Z')", params![id,text,normalized])?;
    }
    for (id, label, explanation, term) in [
        (
            PLACEHOLDER,
            "related-field",
            EXPLANATION,
            "bootstrap-obsolete-term",
        ),
        (
            "bootstrap-endpoint",
            "Synthetic original endpoint",
            "Synthetic original explanation",
            "bootstrap-endpoint-term",
        ),
    ] {
        tx.execute(
            "insert into concepts(id,label,explanation,created_at,updated_at)
            values(?1,?2,?3,'2001-01-01T00:00:00.000Z','2002-01-01T00:00:00.000Z')",
            params![id, label, explanation],
        )?;
        tx.execute(
            "insert into concept_terms(concept_id,term_id,relation_kind,created_at)
            values(?1,?2,'preferred','2001-01-01T00:00:00.000Z')",
            params![id, term],
        )?;
    }
    tx.execute(
        "insert into concept_search(concept_id,terms,label,explanation,qualifiers)
        values(?1,'related-field','related-field',?2,'')",
        params![PLACEHOLDER, EXPLANATION],
    )?;
    tx.execute_batch("insert into bibliographic_items(id,item_type,title,created_at,updated_at)
        values('bootstrap-paper','journalArticle','Synthetic bootstrap paper','2026-01-01','2026-01-01');
        insert into collections(id,name,description,concept_id)
        values('bootstrap-collection-a','Synthetic collection A','Synthetic original description','bootstrap-obsolete'),
        ('bootstrap-collection-b','Synthetic collection B','Synthetic second description',null);
        insert into collection_relation_types(id,name,inverse_name,directed,built_in)
        values('bootstrap-symmetric','Synthetic symmetric relation','Synthetic symmetric relation',0,0);
        insert into collection_relations(id,source_id,target_id,type_id)
        values('bootstrap-pair','bootstrap-collection-a','bootstrap-collection-b','bootstrap-symmetric'),
        ('bootstrap-subfield','bootstrap-collection-b','bootstrap-collection-a','sub-field');
        insert into collection_memberships(item_id,collection_id)
        values('bootstrap-paper','bootstrap-collection-a'),('bootstrap-paper','bootstrap-collection-b');
        insert into item_relation_assignments(item_id,relation_id) values('bootstrap-paper','bootstrap-pair');")?;
    tx.execute(
        "insert into nal_statements(id,subject,predicate,origin) values(?1,?2,?2,'legacy')",
        params![
            REPAIR,
            json!({"kind":"concept","id":"bootstrap-endpoint"}).to_string()
        ],
    )?;
    tx.execute(
        "insert into nal_statements(id,subject,predicate,origin) values(?1,?2,?3,'human')",
        params![
            BAD,
            json!({"kind":"variable","name":"not-a-fact"}).to_string(),
            json!({"kind":"concept","id":"bootstrap-endpoint"}).to_string()
        ],
    )?;
    let note = crate::notes::create_page_with_paper_in(
        &tx,
        crate::notes::NotePageDraft {
            title: "Synthetic bootstrap Core note".into(),
            parent_page_id: None,
            parent_block_position: None,
        },
        None,
        None,
    )?;
    let mut document = crate::notes::load_document_value(&tx, &note)?;
    document.blocks[0].content = json!({"kind":"inline","items":[{"type":"text","text":"Synthetic preserved bootstrap Core body","styles":[]}]});
    crate::notes::save_document_in(
        &tx,
        crate::notes::NoteDocumentSaveRequest {
            document,
            expected_revision: 0,
            save_reason: "manual".into(),
        },
    )?;
    for (id, statement) in [
        ("bootstrap-repair-note", REPAIR),
        ("bootstrap-bad-note", BAD),
    ] {
        tx.execute(
            "insert into nal_annotations(id,statement_id,note_page_id) values(?1,?2,?3)",
            params![id, statement, note],
        )?;
    }
    if tx.execute(
        "update ai_provider_settings set enabled=1,region='cn-beijing',model_id='text-embedding-v4'
        where provider_id='qwen-embedding'",
        [],
    )? != 1
    {
        return Err("synthetic embedding settings missing".into());
    }
    tx.commit()?;
    let attempts = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&attempts);
    db.create_scalar_function(
        "observe_bootstrap_cli_queue",
        0,
        FunctionFlags::SQLITE_UTF8,
        move |_| {
            observed.fetch_add(1, Ordering::SeqCst);
            Ok(0)
        },
    )?;
    db.execute_batch("create temp trigger observe_bootstrap_cli_queue_insert after insert on embedding_outbox_jobs
        begin select observe_bootstrap_cli_queue();end;")?;
    Ok((note, attempts))
}

pub(super) fn note(store: &AppStore, id: &str) -> Result<Value, Error> {
    let db = store.connection()?;
    Ok(serde_json::to_value(crate::notes::load_document_value(
        &db, id,
    )?)?)
}
