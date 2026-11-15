use super::*;

fn fixture() -> (Connection, Request) {
    let db = crate::notes::tests::migrated_connection();
    let id = Uuid::new_v4();
    crate::notes::tests::insert_reading_fixture(&db, &id.to_string(), "My interpretation");
    (
        db,
        Request {
            reading_note_id: id,
            expected_revision: 0,
        },
    )
}
#[test]
fn freezes_note_and_source_separately_without_following_missing_paper_or_writing() {
    let (mut db, request) = fixture();
    let changes = db.total_changes();
    let tx = db.transaction().unwrap();
    let frozen = materialize_in(&tx, &request).unwrap();
    assert_eq!(frozen, materialize_in(&tx, &request).unwrap());
    assert_eq!(frozen.resource_id, request.reading_note_id.to_string());
    assert_eq!(frozen.kind, ContextSourceKind::Note);
    assert_eq!(
        frozen.source_id,
        format!("reading-note:{}", request.reading_note_id)
    );
    let value: Value = serde_json::from_str(&frozen.text).unwrap();
    assert_eq!(
        value["content"]["blocks"][0]["content"]["items"][0]["text"],
        "My interpretation"
    );
    assert_eq!(value["source"]["quoteExact"], "Original quoted evidence");
    assert_eq!(value["source"]["paperId"], "missing-paper");
    assert_eq!(value["source"]["pageIndex"], 2);
    assert!(value["source"]["selector"].is_null());
    assert_eq!(tx.total_changes(), changes);
}
#[test]
fn source_changes_change_hash_even_when_note_revision_stays_the_same() {
    let (db, request) = fixture();
    let first = materialize_in(&db, &request).unwrap();
    db.execute(
        "update reading_note_sources set quote_exact='Updated retained quote'",
        [],
    )
    .unwrap();
    let second = materialize_in(&db, &request).unwrap();
    assert_eq!(first.source_id, second.source_id);
    assert_ne!(first.revision, second.revision);
    db.execute("update note_pages set revision=1", []).unwrap();
    assert_eq!(materialize_in(&db, &request).unwrap_err(), Error::Conflict);
}
#[test]
fn refuses_inactive_foreign_and_missing_records() {
    let (db, request) = fixture();
    for status in ["archived", "trashed"] {
        db.execute("update note_pages set status=?1", [status])
            .unwrap();
        assert_eq!(
            materialize_in(&db, &request).unwrap_err(),
            Error::Unavailable
        );
    }
    assert_eq!(
        materialize_in(
            &db,
            &Request {
                reading_note_id: Uuid::new_v4(),
                expected_revision: 0
            }
        )
        .unwrap_err(),
        Error::Unavailable
    );
    db.execute("delete from reading_note_sources", []).unwrap();
    assert_eq!(
        materialize_in(&db, &request).unwrap_err(),
        Error::Unavailable
    );
}
#[test]
fn rejects_damaged_json_instead_of_inventing_empty_evidence() {
    for sql in [
        "update note_blocks set schema_version=2",
        "update note_blocks set content_json='[]'",
        "update reading_note_sources set rects_json='{}'",
        "update reading_note_sources set selector_json='[]'",
        "update reading_note_sources set selector_json='not-json'",
        "update reading_note_sources set content_sha256='invalid'",
    ] {
        let (db, request) = fixture();
        // Simulate an externally damaged/legacy database, not a valid write path.
        db.execute_batch("pragma ignore_check_constraints=on")
            .unwrap();
        db.execute_batch(sql).unwrap();
        assert_eq!(
            materialize_in(&db, &request).unwrap_err(),
            Error::Unavailable,
            "{sql}"
        );
    }
}

#[test]
fn preserves_valid_two_page_selector_and_rejects_mismatched_quote() {
    let (db, request) = fixture();
    let selector = json!({"schemaVersion":2,"attachmentSha256":"a".repeat(64),"segments":[
        {"pageIndex":2,"rects":[{"x":0.1,"y":0.2,"width":0.3,"height":0.05}],"exactText":"Original quoted evidence"},
        {"pageIndex":3,"rects":[],"exactText":"Continued evidence"}],"retainedExtension":"raw field"});
    db.execute("update reading_note_sources set selector_json=?1,locator_version=2,quote_exact='Original quoted evidence'||char(10)||'Continued evidence'",[selector.to_string()]).unwrap();
    let source = materialize_in(&db, &request).unwrap();
    let body: Value = serde_json::from_str(&source.text).unwrap();
    assert_eq!(body["source"]["selector"], selector);
    db.execute("update reading_note_sources set quote_exact='Mismatch'", [])
        .unwrap();
    assert_eq!(
        materialize_in(&db, &request).unwrap_err(),
        Error::Unavailable
    );
}
#[test]
fn refuses_oversized_evidence_and_invalid_authority_fields() {
    let (db, request) = fixture();
    db.execute(
        "update note_blocks set content_json=?1",
        [json!({"body":"a".repeat(MAX_BYTES as usize)}).to_string()],
    )
    .unwrap();
    assert_eq!(materialize_in(&db, &request).unwrap_err(), Error::Budget);
    for revision in [-1, 9_007_199_254_740_992] {
        assert_eq!(
            materialize_in(
                &db,
                &Request {
                    reading_note_id: request.reading_note_id,
                    expected_revision: revision
                }
            )
            .unwrap_err(),
            Error::InvalidInput
        );
    }
    assert_eq!(
        materialize_in(
            &db,
            &Request {
                reading_note_id: Uuid::nil(),
                expected_revision: 0
            }
        )
        .unwrap_err(),
        Error::InvalidInput
    );
    assert!(serde_json::from_value::<Request>(json!({"readingNoteId":request.reading_note_id,"expectedRevision":0,"allowDisclosure":true})).is_err());
}
