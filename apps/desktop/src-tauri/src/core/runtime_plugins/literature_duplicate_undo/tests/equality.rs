use super::*;
use rusqlite::functions::FunctionFlags;

#[test]
fn noncanonical_fact_key_order_is_compared_as_the_original_typed_vector() {
    let fixture = Fixture::new();
    let value = fixture.seed_nested();
    fixture.seed_snapshot(&value);
    let note = fixture.data.note();
    let before = revisions(&fixture);
    fixture.apply().unwrap();
    assert_eq!(revisions(&fixture), (before.0 + 1, before.1 + 1));
    assert_eq!(fixture.data.note(), note);
    let after = fixture.state();
    assert!(after["statements"].as_array().unwrap().is_empty());
    assert!(after["annotations"].as_array().unwrap().is_empty());
    assert!(after["collections"].as_array().unwrap().is_empty());
    assert!(after["assignments"].as_array().unwrap().is_empty());
    assert_eq!(fixture.reopen().state(), after);
}

#[test]
fn changed_typed_fields_or_extra_after_duplicates_refuse_the_original_undo() {
    for variant in 0..7 {
        let fixture = Fixture::new();
        let mut value = fixture.seed_nested();
        match variant {
            0 => value["classification"]["afterMemberships"][0]["itemId"] = json!("source"),
            1 => {
                let row = value["classification"]["afterAssignments"][0].clone();
                value["classification"]["afterAssignments"]
                    .as_array_mut()
                    .unwrap()
                    .push(row);
            }
            2 => value["classification"]["nal"]["after"][0]["origin"] = json!("legacy"),
            3 => {
                let row = value["classification"]["nal"]["after"][0].clone();
                value["classification"]["nal"]["after"]
                    .as_array_mut()
                    .unwrap()
                    .push(row);
            }
            4 => {
                value["classification"]["nal"]["after_annotations"][0]["notePageId"] =
                    json!("different-note")
            }
            5 => {
                let row = value["classification"]["afterMemberships"][0].clone();
                value["classification"]["afterMemberships"]
                    .as_array_mut()
                    .unwrap()
                    .push(row);
            }
            _ => {
                let row = value["classification"]["nal"]["after_annotations"][0].clone();
                value["classification"]["nal"]["after_annotations"]
                    .as_array_mut()
                    .unwrap()
                    .push(row);
            }
        }
        fixture.seed_snapshot(&value);
        let before = fixture.state();
        let failure = fixture.apply().unwrap_err();
        assert!(
            matches!(failure.problem.cause, UndoCause::Conflict),
            "variant {variant}"
        );
        assert!(matches!(
            failure.problem.phase,
            UndoPhase::ClassificationCheck | UndoPhase::NalCheck
        ));
        assert_eq!(failure.outcome, UndoOutcome::RolledBack);
        assert_eq!(fixture.state(), before, "variant {variant}");
        assert_eq!(fixture.reopen().state(), before, "variant {variant}");
    }
}

#[test]
fn structurally_valid_unresolved_fact_entities_remain_accepted() {
    let fixture = Fixture::new();
    let mut nal = fixture.seed_nal();
    let subject = json!({"kind":"product","terms":[
        {"kind":"paper","id":"survivor"},
        {"kind":"concept","id":"missing-concept-entity"}]});
    let predicate = json!({"kind":"paper","id":"missing-paper-entity"});
    fixture
        .data
        .db
        .lock()
        .unwrap()
        .execute(
            "update nal_statements set subject=?1,predicate=?2 where id='s'",
            rusqlite::params![subject.to_string(), predicate.to_string()],
        )
        .unwrap();
    nal["after"][0]["subject"] = subject;
    nal["after"][0]["predicate"] = predicate;
    let mut value = snapshot();
    value["classification"] = classification();
    value["classification"]["nal"] = nal;
    fixture.seed_snapshot(&value);
    let note = fixture.data.note();
    fixture.apply().unwrap();
    assert_eq!(fixture.data.note(), note);
    let after = fixture.state();
    assert!(after["statements"].as_array().unwrap().is_empty());
    assert_eq!(fixture.reopen().state(), after);
}

fn product_subject(depth: usize) -> String {
    let mut raw = r#"{"kind":"paper","id":"survivor"}"#.to_owned();
    for _ in 0..depth {
        raw = format!(r#"{{"kind":"product","terms":[{raw}]}}"#);
    }
    raw
}

#[test]
fn original_depth_63_fact_load_reaches_the_after_conflict() {
    let fixture = Fixture::new();
    let value = fixture.seed_nested();
    fixture.seed_snapshot(&value);
    let raw = product_subject(63);
    // The original loader decodes the stored subject as a bare term. Its
    // accepted depth must not shrink when a scoped row adds a statement object.
    serde_json::from_str::<research_workbench_native_sdk::nal_state::NalTerm>(&raw).unwrap();
    fixture
        .data
        .db
        .lock()
        .unwrap()
        .execute("update nal_statements set subject=?1 where id='s'", [raw])
        .unwrap();
    let before = fixture.state();
    let note = fixture.data.note();
    let failure = fixture.apply().unwrap_err();
    assert_eq!(failure.problem.code(), "invalid_input");
    assert!(matches!(failure.problem.phase, UndoPhase::NalCheck));
    assert!(matches!(failure.problem.cause, UndoCause::Conflict));
    assert_eq!(failure.outcome, UndoOutcome::RolledBack);
    assert_eq!(fixture.data.note(), note);
    assert_eq!(fixture.state(), before);
    assert_eq!(fixture.reopen().state(), before);
}

#[test]
fn original_depth_64_fact_rejection_remains_a_storage_error() {
    let fixture = Fixture::new();
    let value = fixture.seed_nested();
    fixture.seed_snapshot(&value);
    let raw = product_subject(64);
    let original_error =
        serde_json::from_str::<research_workbench_native_sdk::nal_state::NalTerm>(&raw)
            .unwrap_err();
    fixture
        .data
        .db
        .lock()
        .unwrap()
        .execute("update nal_statements set subject=?1 where id='s'", [raw])
        .unwrap();
    let before = fixture.state();
    let note = fixture.data.note();
    let failure = fixture.apply().unwrap_err();
    assert_eq!(failure.problem.code(), "storage_error");
    assert!(matches!(failure.problem.phase, UndoPhase::NalLoad));
    let UndoCause::StructureCodec(codec) = &failure.problem.cause else {
        panic!("original fact codec diagnostic was lost");
    };
    assert_eq!(codec.category, "syntax");
    assert_eq!(codec.line, original_error.line());
    assert_eq!(codec.column, original_error.column());
    assert_eq!(failure.outcome, UndoOutcome::RolledBack);
    assert_eq!(fixture.data.note(), note);
    assert_eq!(fixture.state(), before);
    assert_eq!(fixture.reopen().state(), before);
}

#[test]
fn real_annotation_cascade_and_repeated_outer_ids_allow_zero_row_deletes() {
    let fixture = Fixture::new();
    let mut value = fixture.seed_nested();
    fixture.seed_reading_source();
    value["anchorIds"] = json!(["anchor", "anchor"]);
    value["attachmentIds"] = json!(["attachment", "attachment"]);
    for field in [
        "identifierIds",
        "addedProjectIds",
        "addedTagIds",
        "addedProvenanceFields",
    ] {
        value[field] = json!(["missing", "missing"]);
    }
    fixture.seed_snapshot(&value);
    let note = fixture.data.note();
    let deletions = Arc::new(AtomicUsize::new(0));
    let count = deletions.clone();
    {
        let db = fixture.data.db.lock().unwrap();
        db.create_scalar_function(
            "observe_undo_annotation_delete",
            0,
            FunctionFlags::SQLITE_UTF8,
            move |_| {
                count.fetch_add(1, Ordering::SeqCst);
                Ok(0)
            },
        )
        .unwrap();
        db.execute_batch(
            "create temp trigger observe_undo_annotation_delete after delete on nal_annotations
            begin select observe_undo_annotation_delete();end;",
        )
        .unwrap();
    }
    fixture.apply().unwrap();
    // Statement deletion already removed the row before the explicit annotation delete.
    assert_eq!(deletions.load(Ordering::SeqCst), 1);
    let db = fixture.data.db.lock().unwrap();
    for sql in [
        "select item_id from source_anchors where id='anchor'",
        "select item_id from attachments where id='attachment'",
        "select paper_id_snapshot from reading_note_sources where block_id='source-alias'",
    ] {
        assert_eq!(
            db.query_row(sql, [], |row| row.get::<_, String>(0))
                .unwrap(),
            "source"
        );
    }
    assert_eq!(
        db.query_row(
            "select source_label from note_page_catalog where id=?1",
            [crate::runtime_plugins::literature_duplicates::tests::fixture::PAGE],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "Synthetic source"
    );
    drop(db);
    assert_eq!(fixture.data.note(), note);
    let after = fixture.state();
    assert_eq!(fixture.reopen().state(), after);
}
