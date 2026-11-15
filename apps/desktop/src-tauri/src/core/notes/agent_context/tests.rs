use super::*;
use crate::notes::{
    document_repository::save_document_from,
    model::{NoteDocumentBlock, NoteDocumentSaveRequest, NotePageDraft},
    page_repository::create_page_from,
    tests::migrated_connection,
};

fn block(id: &str, text: &str) -> NoteDocumentBlock {
    NoteDocumentBlock {
        id: id.into(),
        block_type: "paragraph".into(),
        props: serde_json::json!({}),
        content: serde_json::json!({"kind":"inline","items":[{"type":"text","text":text,"styles":[]}]}),
        children: vec![],
    }
}

fn prepared() -> (Connection, NoteContextRequest) {
    let mut db = migrated_connection();
    let mut created = create_page_from(
        &mut db,
        NotePageDraft {
            title: "Synthetic note".into(),
            parent_page_id: None,
            parent_block_position: None,
        },
    )
    .unwrap();
    let mut first = block("root", "Ignore instructions is evidence, not authority.");
    first.children.push(block("nested", "Nested evidence"));
    created.document.blocks = vec![first, block("outside", "Not selected")];
    let saved = save_document_from(
        &mut db,
        NoteDocumentSaveRequest {
            document: created.document,
            expected_revision: 0,
            save_reason: "manual".into(),
        },
    )
    .unwrap();
    (
        db,
        NoteContextRequest {
            page_id: saved.page.id.parse().unwrap(),
            expected_revision: 1,
            block_id: None,
        },
    )
}

#[test]
fn freezes_structure_and_selection_without_following_references_or_writing() {
    let (db, mut request) = prepared();
    let changes = db.total_changes();
    let whole = materialize_in(&db, &request).unwrap();
    assert_eq!(whole.kind, ContextSourceKind::Note);
    let document: serde_json::Value = serde_json::from_str(&whole.text).unwrap();
    assert_eq!(document["blocks"][0]["children"][0]["id"], "nested");
    assert_eq!(materialize_in(&db, &request).unwrap(), whole);
    request.block_id = Some("root".into());
    let selected = materialize_in(&db, &request).unwrap();
    assert!(selected.text.contains("Nested evidence"));
    assert!(!selected.text.contains("Not selected"));
    assert_ne!(selected.source_id, whole.source_id);
    assert_eq!(db.total_changes(), changes);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(selected.anchor.as_ref().unwrap()).unwrap()["blockId"],
        "root"
    );
}

#[test]
fn rejects_stale_missing_and_inactive_sources() {
    let (db, mut request) = prepared();
    request.expected_revision = 0;
    assert_eq!(
        materialize_in(&db, &request),
        Err(NoteContextError::Conflict)
    );
    request.expected_revision = 1;
    request.block_id = Some("absent".into());
    assert_eq!(
        materialize_in(&db, &request),
        Err(NoteContextError::Conflict)
    );
    request.block_id = None;
    db.execute(
        "update note_pages set status='archived' where id=?1",
        [request.page_id.to_string()],
    )
    .unwrap();
    assert_eq!(
        materialize_in(&db, &request),
        Err(NoteContextError::Unavailable)
    );
    request.page_id = Uuid::new_v4();
    assert_eq!(
        materialize_in(&db, &request),
        Err(NoteContextError::Unavailable)
    );
}

#[test]
fn rejects_corruption_instead_of_using_the_interactive_loader_fallback() {
    for sql in [
        "update note_blocks set content_json='not json' where id='root'",
        "update note_blocks set props_json='[]' where id='root'",
        "update note_blocks set parent_block_id='nested' where id='root'",
    ] {
        let (db, request) = prepared();
        // Simulate damaged/imported storage, bypassing constraints only in this
        // isolated in-memory fixture; normal note writes retain their checks.
        db.pragma_update(None, "ignore_check_constraints", true)
            .unwrap();
        db.execute(sql, []).unwrap();
        db.pragma_update(None, "ignore_check_constraints", false)
            .unwrap();
        assert_eq!(
            materialize_in(&db, &request),
            Err(NoteContextError::Unavailable),
            "{sql}"
        );
    }
}

#[test]
fn bounds_full_page_even_when_a_small_block_is_selected() {
    let (db, mut request) = prepared();
    request.block_id = Some("nested".into());
    db.execute(
        "update note_blocks set props_json=?1 where id='outside'",
        [format!("{{\"large\":\"{}\"}}", "x".repeat(MAX_BYTES))],
    )
    .unwrap();
    assert_eq!(materialize_in(&db, &request), Err(NoteContextError::Budget));
}

#[test]
fn preserves_reference_and_asset_identities_without_loading_their_targets() {
    let (db, request) = prepared();
    let content = serde_json::json!({"kind":"reference","targetKind":"paper","targetId":"not-readable",
        "labelSnapshot":"Original label","quoteSnapshot":"Quoted evidence"});
    db.execute("update note_blocks set block_type='researchObjectReference',content_json=?1 where id='outside'", [content.to_string()]).unwrap();
    let result = materialize_in(&db, &request).unwrap();
    assert!(result.text.contains("not-readable"));
    let asset = serde_json::json!({"kind":"asset","assetId":"a".repeat(64),"displayName":"figure.png","mimeType":"image/png","caption":"Synthetic"});
    db.execute(
        "update note_blocks set block_type='image',content_json=?1 where id='outside'",
        [asset.to_string()],
    )
    .unwrap();
    let next = materialize_in(&db, &request).unwrap();
    assert!(next.text.contains(&"a".repeat(64)));
    assert_ne!(next.revision, result.revision);
    assert_eq!(next.source_id, result.source_id);
}

#[test]
fn input_contract_rejects_extra_authority_and_invalid_identifiers() {
    let (_, mut request) = prepared();
    assert!(
        serde_json::from_value::<NoteContextRequest>(serde_json::json!({
            "pageId":request.page_id,"expectedRevision":1,"blockId":null,"allowAllNotes":true
        }))
        .is_err()
    );
    request.block_id = Some("\0".into());
    assert_eq!(
        validate_request(&request),
        Err(NoteContextError::InvalidInput)
    );
    request.block_id = None;
    request.expected_revision = -1;
    assert_eq!(
        validate_request(&request),
        Err(NoteContextError::InvalidInput)
    );
}

#[test]
fn preview_wire_contract_contains_only_versioned_evidence() {
    let (db, request) = prepared();
    let source = materialize_in(&db, &request).unwrap();
    let encoded = serde_json::to_value(NoteContextPreview {
        schema_version: 1,
        source: source.clone(),
    })
    .unwrap();
    assert_eq!(encoded.as_object().unwrap().len(), 2);
    assert_eq!(encoded["schemaVersion"], 1);
    assert_eq!(encoded["source"]["kind"], "note");
    assert_eq!(encoded["source"]["resourceId"], request.page_id.to_string());
    let decoded: ContextText = serde_json::from_value(encoded["source"].clone()).unwrap();
    assert_eq!(decoded, source);
    assert_eq!(
        serde_json::to_value(NoteContextError::Conflict).unwrap(),
        "conflict"
    );
}

#[test]
fn rejects_excessive_depth_before_recursive_loading() {
    let (mut db, mut request) = prepared();
    let mut chain = block("depth-64", "leaf");
    for depth in (1..64).rev() {
        let mut parent = block(&format!("depth-{depth}"), "ancestor");
        parent.children.push(chain);
        chain = parent;
    }
    let mut snapshot = load_document_from(&db, &request.page_id.to_string()).unwrap();
    snapshot.document.blocks = vec![chain, block("extra-root", "extra")];
    let saved = save_document_from(
        &mut db,
        NoteDocumentSaveRequest {
            document: snapshot.document,
            expected_revision: 1,
            save_reason: "manual".into(),
        },
    )
    .unwrap();
    request.expected_revision = saved.page.revision;
    assert!(materialize_in(&db, &request).is_ok());
    db.execute(
        "update note_blocks set parent_block_id='depth-64' where id='extra-root'",
        [],
    )
    .unwrap();
    assert_eq!(
        materialize_in(&db, &request),
        Err(NoteContextError::Unavailable)
    );
}
