use super::*;
use research_workbench_native_sdk::{
    duplicate_merge::{
        CollectionAssignment, CollectionMembership, DuplicateMergeSnapshot,
        MergeClassificationSnapshot, MergeNalSnapshot,
    },
    nal_annotation::NalAnnotation,
    nal_state::{NalStatement, NalTerm},
};
use rusqlite::params;

pub(super) const NOTE: &str = "duplicate-undo-core-note";
pub(super) const BODY: &str = "Preserved synthetic Core-owned duplicate undo body";
const CONTENT: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

#[derive(Clone, Copy)]
pub(super) enum Mode {
    Full,
    NoClassification,
    NoNal,
}

pub(super) struct Case {
    pub id: String,
    pub survivor: String,
    pub source: String,
    pub snapshot: DuplicateMergeSnapshot,
    pub mode: Mode,
}

impl Case {
    pub(super) fn identity(&self, suffix: &str) -> String {
        format!("{}-{suffix}", self.id)
    }
}

fn stored_term(term: &NalTerm) -> Result<String, serde_json::Error> {
    // Explicitly noncanonical field order, while retaining original values.
    Ok(match term {
        NalTerm::Concept { id } => format!(
            "{{\"id\":{},\"kind\":\"concept\"}}",
            serde_json::to_string(id)?
        ),
        NalTerm::Paper { id } => format!(
            "{{\"id\":{},\"kind\":\"paper\"}}",
            serde_json::to_string(id)?
        ),
        NalTerm::Product { terms } => format!(
            "{{\"terms\":[{}],\"kind\":\"product\"}}",
            terms
                .iter()
                .map(stored_term)
                .collect::<Result<Vec<_>, _>>()?
                .join(",")
        ),
    })
}

pub(super) fn note(store: &AppStore) -> Result<Value, Error> {
    let db = store.connection()?;
    Ok(serde_json::to_value(crate::notes::load_document_value(
        &db, NOTE,
    )?)?)
}

pub(super) fn seed_note(store: &AppStore) -> Result<Value, Error> {
    let mut db = store.connection()?;
    let tx = db.transaction()?;
    crate::notes::create_page_with_paper_in(
        &tx,
        crate::notes::NotePageDraft {
            title: "Synthetic duplicate undo Core note".into(),
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
    tx.execute("insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at) values(?1,'synthetic-undo.pdf','application/pdf',0,'2026')",[CONTENT])?;
    tx.commit()?;
    drop(db);
    let document = note(store)?;
    if document["blocks"][0]["content"]["items"][0]["text"] != BODY {
        return Err("Core writer did not preserve the synthetic undo body".into());
    }
    Ok(document)
}

pub(super) fn seed(store: &AppStore, label: &str, mode: Mode) -> Result<Case, Error> {
    let id = format!("undo-{label}");
    let survivor = format!("{id}-survivor");
    let source = format!("{id}-source");
    let identity = |suffix: &str| format!("{id}-{suffix}");
    let keep = CollectionMembership {
        item_id: survivor.clone(),
        collection_id: identity("collection-a-keep"),
    };
    let added = CollectionMembership {
        item_id: survivor.clone(),
        collection_id: identity("collection-b-added"),
    };
    let assignment = CollectionAssignment {
        item_id: survivor.clone(),
        relation_id: identity("relation"),
    };
    let first = NalStatement {
        id: identity("statement-keep"),
        subject: NalTerm::Paper {
            id: survivor.clone(),
        },
        predicate: NalTerm::Product {
            terms: vec![
                NalTerm::Paper { id: source.clone() },
                NalTerm::Concept {
                    id: "shape-valid-missing-concept".into(),
                },
            ],
        },
        origin: "human".into(),
    };
    let second = NalStatement {
        id: identity("statement-added"),
        subject: NalTerm::Product {
            terms: vec![
                NalTerm::Paper {
                    id: survivor.clone(),
                },
                NalTerm::Product {
                    terms: vec![NalTerm::Paper { id: source.clone() }],
                },
            ],
        },
        predicate: NalTerm::Concept {
            id: "shape-valid-missing-concept".into(),
        },
        origin: "legacy".into(),
    };
    let annotation = NalAnnotation {
        id: identity("annotation"),
        target_kind: "statement".into(),
        target_id: second.id.clone(),
        note_page_id: NOTE.into(),
    };
    let nal = MergeNalSnapshot {
        before: vec![first.clone()],
        after: vec![first.clone(), second.clone()],
        before_annotations: vec![],
        after_annotations: vec![annotation.clone()],
    };
    let classification = if matches!(mode, Mode::NoClassification) {
        None
    } else {
        Some(MergeClassificationSnapshot {
            nal: if matches!(mode, Mode::NoNal) {
                None
            } else {
                Some(nal)
            },
            before_memberships: vec![keep.clone()],
            before_assignments: vec![],
            after_memberships: vec![keep.clone(), added.clone()],
            after_assignments: vec![assignment.clone()],
        })
    };
    let snapshot = DuplicateMergeSnapshot {
        classification,
        candidate_id: identity("candidate"),
        attachment_ids: vec![identity("attachment")],
        identifier_ids: vec![identity("identifier")],
        anchor_ids: vec![identity("anchor")],
        added_project_ids: vec![identity("project")],
        added_tag_ids: vec![identity("tag")],
        added_provenance_fields: vec!["url".into()],
        created_reading_state: false,
        merged_archived_at: None,
    };
    let mut db = store.connection()?;
    let tx = db.transaction()?;
    for (paper, title, archived) in [
        (&survivor, format!("Survivor {label}"), None),
        (&source, format!("Source {label}"), Some("merged-archive")),
    ] {
        tx.execute("insert into bibliographic_items(id,item_type,title,created_at,updated_at,archived_at) values(?1,'journalArticle',?2,'2026','2026',?3)",params![paper,title,archived])?;
        tx.execute(
            "insert into literature_search(item_id,title) values(?1,?2)",
            params![paper, title],
        )?;
    }
    for member in [&keep, &added] {
        tx.execute(
            "insert into collections(id,name) values(?1,?2)",
            params![member.collection_id, member.collection_id],
        )?;
        tx.execute(
            "insert into collection_memberships(item_id,collection_id) values(?1,?2)",
            params![survivor, member.collection_id],
        )?;
    }
    tx.execute("insert into collection_relations(id,source_id,target_id,type_id) values(?1,?2,?3,'related-field')",params![assignment.relation_id,keep.collection_id,added.collection_id])?;
    tx.execute(
        "insert into item_relation_assignments(item_id,relation_id) values(?1,?2)",
        params![survivor, assignment.relation_id],
    )?;
    for statement in [&first, &second] {
        let subject = stored_term(&statement.subject)?;
        let predicate = stored_term(&statement.predicate)?;
        tx.execute(
            "insert into nal_statements(id,subject,predicate,origin) values(?1,?2,?3,?4)",
            params![statement.id, subject, predicate, statement.origin],
        )?;
    }
    tx.execute(
        "insert into nal_annotations(id,statement_id,note_page_id) values(?1,?2,?3)",
        params![annotation.id, annotation.target_id, NOTE],
    )?;
    tx.execute("insert into attachments(id,item_id,content_sha256,display_name,created_at) values(?1,?2,?3,'Synthetic undo PDF','2026')",params![identity("attachment"),survivor,CONTENT])?;
    tx.execute("insert into source_anchors(id,item_id,attachment_id,content_sha256,page_index,quote_exact,created_at) values(?1,?2,?3,?4,0,'Synthetic undo quote','2026')",params![identity("anchor"),survivor,identity("attachment"),CONTENT])?;
    tx.execute(
        "insert into reading_states(item_id,attachment_id,updated_at) values(?1,?2,'2026')",
        params![survivor, identity("attachment")],
    )?;
    tx.execute("insert into identifiers(id,item_id,kind,normalized_value,display_value) values(?1,?2,'doi',?3,?3)",params![identity("identifier"),survivor,identity("doi")])?;
    tx.execute("insert into research_projects(id,title,summary,created_at) values(?1,?2,'Synthetic undo scope','2026')",params![identity("project"),identity("project")])?;
    tx.execute(
        "insert into project_items(project_id,item_id,added_at) values(?1,?2,'2026')",
        params![identity("project"), survivor],
    )?;
    tx.execute("insert into tags(id,name) values(?1,?1)", [identity("tag")])?;
    tx.execute(
        "insert into item_tags(item_id,tag_id) values(?1,?2)",
        params![survivor, identity("tag")],
    )?;
    tx.execute("insert into metadata_provenance(item_id,field_name,source_kind,updated_at) values(?1,'url','user','2026')",[&survivor])?;
    tx.execute("insert into duplicate_candidates(id,left_item_id,right_item_id,reason,confidence,decision,created_at,decided_at) values(?1,?2,?3,'doi',1,'merge','2026','2026')",params![snapshot.candidate_id,survivor,source])?;
    tx.execute("insert into merge_operations(id,survivor_item_id,merged_item_id,snapshot_json,state,created_at) values(?1,?2,?3,?4,'applied','2026')",params![id,survivor,source,serde_json::to_string(&snapshot)?])?;
    tx.execute("insert into reading_note_sources(block_id,page_id,note_type,color,current_anchor_id,paper_id_snapshot,paper_title_snapshot,attachment_id_snapshot,attachment_name_snapshot,content_sha256,page_index,rects_json,quote_exact,quote_prefix,quote_suffix,locator_version,created_at,updated_at)
        values('undo-reading-source',?1,'annotation','#f4cf65',?2,?3,?4,?5,'Synthetic undo PDF',?6,0,'[]','Synthetic undo quote','','',1,'2026','2026')
        on conflict(block_id) do update set current_anchor_id=excluded.current_anchor_id,paper_id_snapshot=excluded.paper_id_snapshot,paper_title_snapshot=excluded.paper_title_snapshot,attachment_id_snapshot=excluded.attachment_id_snapshot,updated_at=excluded.updated_at",
        params![NOTE,identity("anchor"),survivor,format!("Survivor {label}"),identity("attachment"),CONTENT])?;
    tx.commit()?;
    Ok(Case {
        id,
        survivor,
        source,
        snapshot,
        mode,
    })
}

pub(super) fn change_snapshot(store: &AppStore, case: &Case, raw: &str) -> Result<(), Error> {
    store.connection()?.execute(
        "update merge_operations set snapshot_json=?2 where id=?1",
        params![case.id, raw],
    )?;
    Ok(())
}
