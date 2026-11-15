use super::*;
use sources::{Access, Part, Resource};
#[path = "catalog_tests/test_sources.rs"]
mod test_sources;

#[test]
fn focused_note_recommendation_uses_only_identity_metadata() {
    // No library, manuscript, document body or resource catalog tables: the
    // focused path must not enumerate them even when titles are ambiguous.
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(
        "create table note_pages(id text primary key,title text,status text);
        create table reading_note_sources(page_id text);",
    )
    .unwrap();
    let id = Uuid::new_v4();
    let candidate = || sources::Selection {
        label: "old title".into(),
        resource: Resource::Note { resource_id: id },
    };
    assert!(matches!(
        catalog::recommend_note(&db, candidate()).unwrap().resource,
        Resource::Note { .. }
    ));
    db.execute(
        "insert into note_pages values(?1,'Current title','active')",
        [id.to_string()],
    )
    .unwrap();
    let result = catalog::recommend_note(&db, candidate()).unwrap();
    assert_eq!(result.label, "笔记 · Current title");
    let Resource::NoteGroup {
        resource_id,
        parts,
        group_access,
        ..
    } = result.resource
    else {
        panic!("note group")
    };
    assert_eq!(resource_id, id.to_string());
    assert_eq!(group_access, Some(Access::ReadWrite));
    assert_eq!(parts.len(), 1);
    assert!(parts[0].selected);
    assert!(matches!(parts[0].resource,Resource::Note{resource_id} if resource_id==id));
    db.execute(
        "insert into reading_note_sources values(?1)",
        [id.to_string()],
    )
    .unwrap();
    assert!(matches!(
        catalog::recommend_note(&db, candidate()).unwrap().resource,
        Resource::Note { .. }
    ));
    db.execute("delete from reading_note_sources", []).unwrap();
    db.execute("update note_pages set status='trashed'", [])
        .unwrap();
    assert!(matches!(
        catalog::recommend_note(&db, candidate()).unwrap().resource,
        Resource::Note { .. }
    ));
    let wrong = sources::Selection {
        label: "paper".into(),
        resource: Resource::PaperMetadata {
            resource_id: "paper".into(),
        },
    };
    assert!(matches!(
        catalog::recommend_note(&db, wrong),
        Err(Error::InvalidInput)
    ));
}
fn paper_db() -> Connection {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    db.execute("insert into bibliographic_items(id,item_type,title,created_at,updated_at) values ('paper','document','Synthetic','now','now')",[]).unwrap();
    db
}
#[test]
fn catalog_groups_metadata_and_portrait_even_without_pdf_and_rechecks_child_ownership() {
    let db = paper_db();
    let rows = catalog::search(
        test_sources::catalog(),
        &db,
        "Synthetic",
        Some(crate::agent::writing_resources::test_sources::source()),
    )
    .unwrap();
    assert_eq!(rows.len(), 1);
    let Resource::PaperGroup {
        resource_id, parts, ..
    } = &rows[0].resource
    else {
        panic!("paper group")
    };
    assert_eq!(parts.len(), 5);
    assert!(
        parts
            .iter()
            .all(|part| part.selected && part.access == Access::ReadWrite)
    );
    let foreign = Part {
        resource: Resource::PaperMetadata {
            resource_id: "other".into(),
        },
        label: "Metadata".into(),
        selected: true,
        access: Access::ReadWrite,
    };
    assert_eq!(
        catalog::validate_parts(
            &db,
            &rows[0].resource,
            resource_id,
            &[foreign],
            Some(crate::agent::writing_resources::test_sources::source())
        ),
        Err(Error::InvalidInput)
    );
    assert!(catalog::metadata(&db, "other", test_sources::metadata).is_err());
}
#[test]
fn unchecked_sources_are_not_read_and_permissions_survive_native_freezing() {
    let db = paper_db();
    let root = tempfile::tempdir().unwrap();
    let store = crate::storage::AppStore::for_test(db, root.path().join("objects"))
        .with_test_paper_metadata_source(test_sources::metadata);
    let mut parts = match catalog::search(
        test_sources::catalog(),
        &store.connection().unwrap(),
        "",
        Some(crate::agent::writing_resources::test_sources::source()),
    )
    .unwrap()
    .remove(0)
    .resource
    {
        Resource::PaperGroup { parts, .. } => parts,
        _ => panic!("group"),
    };
    parts[0].access = Access::ReadWrite;
    for part in parts.iter_mut().skip(1) {
        part.selected = false;
    }
    let mut resource = Resource::PaperGroup {
        new_member_access: None,
        group_access: None,
        resource_id: "paper".into(),
        parts,
    };
    let frozen = sources::materialize(
        &store.connection().unwrap(),
        &store,
        std::slice::from_ref(&resource),
    )
    .unwrap();
    assert_eq!(frozen.len(), 1);
    let anchor: serde_json::Value =
        serde_json::from_str(frozen[0].anchor.as_deref().unwrap()).unwrap();
    assert_eq!(anchor["contextAccess"], "readWrite");
    assert_eq!(anchor["content"], "metadata");
    // An unselected metadata source must not run even if its body is unreadable.
    store
        .connection()
        .unwrap()
        .execute_batch("drop table identifiers")
        .unwrap();
    let Resource::PaperGroup { parts, .. } = &mut resource else {
        panic!("paper group")
    };
    parts[0].selected = false;
    assert!(
        sources::materialize(&store.connection().unwrap(), &store, &[resource])
            .unwrap()
            .is_empty()
    );
}

#[test]
fn metadata_source_keeps_core_identity_revision_and_the_callers_transaction() {
    use sha2::{Digest, Sha256};
    let root = tempfile::tempdir().unwrap();
    let store = crate::storage::AppStore::for_test(paper_db(), root.path().join("objects"))
        .with_test_paper_metadata_source(test_sources::metadata);
    let mut db = store.connection().unwrap();
    let resources = [Resource::PaperMetadata {
        resource_id: "paper".into(),
    }];
    let before = sources::materialize(&db, &store, &resources)
        .unwrap()
        .remove(0);
    let tx = db.transaction().unwrap();
    tx.execute(
        "update bibliographic_items set title='Uncommitted Ω',archived_at='now' where id='paper'",
        [],
    )
    .unwrap();
    let current = sources::materialize(&tx, &store, &resources)
        .unwrap()
        .remove(0);
    let expected: serde_json::Value = serde_json::from_str(
        r#"{"title":"Uncommitted Ω","itemType":"document","abstract":"","year":null,"container":"","volume":"","issue":"","pages":"","url":"","language":"","bibtexKey":"","creators":[],"identifiers":[]}"#,
    )
    .unwrap();
    assert_eq!(current.text, expected.to_string());
    assert_eq!(current.source_id, "paper:paper:metadata");
    assert_eq!(current.resource_id, "paper");
    assert!(matches!(
        current.kind,
        research_agent_core::submission::ContextSourceKind::Paper
    ));
    assert_eq!(current.anchor.as_deref(), Some(r#"{"content":"metadata"}"#));
    assert_eq!(
        current.revision,
        format!("sha256:{:x}", Sha256::digest(current.text.as_bytes()))
    );
    assert_ne!(current.revision, before.revision);
    tx.rollback().unwrap();
    let restored = sources::materialize(&db, &store, &resources)
        .unwrap()
        .remove(0);
    assert_eq!(restored.text, before.text);
    assert_eq!(restored.revision, before.revision);
    assert_eq!(restored.source_id, before.source_id);
}

#[test]
fn full_metadata_preflight_precedes_portrait_and_reading_note_checks() {
    use crate::agent::paper_metadata::PaperMetadataError;
    let empty = Connection::open_in_memory().unwrap();
    let source_unavailable = |_: &Connection, _: &str| Err(PaperMetadataError::SourceUnavailable);
    assert!(matches!(
        catalog::metadata(&empty, "paper", source_unavailable),
        Err(Error::SourceUnavailable)
    ));
    // There are no portrait tables: the required metadata error wins first.
    assert!(matches!(
        catalog::portrait(&empty, "paper", source_unavailable, |_, _| panic!(
            "metadata preflight must precede portrait source"
        )),
        Err(Error::SourceUnavailable)
    ));
    let db = paper_db();
    db.execute_batch("drop table identifiers").unwrap();
    let root = tempfile::tempdir().unwrap();
    let store = crate::storage::AppStore::for_test(db, root.path().join("objects"))
        .with_test_paper_metadata_source(test_sources::metadata);
    let db = store.connection().unwrap();
    // The base item exists, but its third metadata query fails. Neither an
    // empty portrait nor invalid exclusions may bypass that complete preflight.
    assert!(matches!(
        catalog::portrait(&db, "paper", test_sources::metadata, |_, _| panic!(
            "metadata preflight must precede portrait source"
        )),
        Err(Error::Unavailable)
    ));
    assert!(matches!(
        catalog::reading_notes(&db, &store, "paper", &["not-a-uuid".into()]),
        Err(Error::Unavailable)
    ));
}
#[test]
fn pdf_write_and_nested_groups_are_rejected_even_when_unchecked() {
    let db = paper_db();
    let owner = Resource::PaperGroup {
        new_member_access: None,
        group_access: None,
        resource_id: "paper".into(),
        parts: vec![],
    };
    for resource in [
        Resource::Paper {
            resource_id: "paper".into(),
            attachment_id: Some("pdf".into()),
            page_index: None,
        },
        owner.clone(),
    ] {
        let part = Part {
            resource,
            label: "PDF".into(),
            selected: false,
            access: Access::ReadWrite,
        };
        assert_eq!(
            catalog::validate_parts(
                &db,
                &owner,
                "paper",
                &[part],
                Some(crate::agent::writing_resources::test_sources::source())
            ),
            Err(Error::InvalidInput)
        );
    }
}
