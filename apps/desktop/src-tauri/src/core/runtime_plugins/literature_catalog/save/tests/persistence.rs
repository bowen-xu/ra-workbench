use super::{fixture::*, *};
use research_workbench_native_sdk::data::DataValue;
use rusqlite::functions::FunctionFlags;
use std::sync::{Arc, Mutex};

#[test]
fn saves_complete_plan_with_original_fts_timestamps_and_reopens_sql_log_pair() {
    let fixture = Fixture::new();
    let result = fixture.persist(&plan(None)).unwrap();
    uuid::Uuid::parse_str(&result.item_id).unwrap();
    assert_eq!(result.items.len(), 1);
    assert_eq!(
        result.items[0].main.rows[0][0],
        DataValue::Text(result.item_id.clone())
    );
    let db = fixture.db.lock().unwrap();
    assert!(db.is_autocommit());
    let (created, updated): (String, String) = db
        .query_row(
            "select created_at,updated_at from bibliographic_items where id=?1",
            [&result.item_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(created, updated);
    assert!(created.contains('T') && created.ends_with('Z'));
    for table in ["creators", "identifiers", "tags"] {
        let mut statement = db.prepare(&format!("select id from {table}")).unwrap();
        for id in statement.query_map([], |r| r.get::<_, String>(0)).unwrap() {
            uuid::Uuid::parse_str(&id.unwrap()).unwrap();
        }
    }
    assert_eq!(
        db.query_row(
            "select count(*) from metadata_provenance where confirmed_by_user=1",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        17
    );
    for query in ["Synthetic", "Family", "First"] {
        let ids = db
            .prepare("select item_id from literature_search where literature_search match ?1")
            .unwrap()
            .query_map([query], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(ids.as_slice(), std::slice::from_ref(&result.item_id));
    }
    drop(db);
    let fixture = fixture.reopen();
    assert_eq!(fixture.count("bibliographic_items"), 1);
    assert_eq!(fixture.count("item_tags"), 2);
    assert_eq!(fixture.count("metadata_provenance"), 17);
}

#[test]
fn update_retains_created_reading_raw_bibtex_and_original_confirmed_max() {
    let fixture = Fixture::new();
    fixture.persist(&plan(Some("original-id"))).unwrap();
    fixture.db.lock().unwrap().execute_batch(
        "update bibliographic_items set created_at='original-created',reading_status='read' where id='original-id';",
    ).unwrap();
    let mut update = plan(Some("original-id"));
    update.item.title = "Replacement title".into();
    update.item.raw_bibtex = None;
    update.creators.clear();
    update.tags.clear();
    update.doi = None;
    update.provenance.source_kind = " source bytes ".into();
    update.provenance.confirmed_by_user = false;
    let output = fixture.persist(&update).unwrap();
    assert_eq!(output.item_id, "original-id");
    let db = fixture.db.lock().unwrap();
    let values: (String, String, String, String) = db.query_row(
        "select title,raw_bibtex,reading_status,created_at from bibliographic_items where id='original-id'", [],
        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
    ).unwrap();
    assert_eq!(
        values,
        (
            "Replacement title".into(),
            "@article{original}".into(),
            "read".into(),
            "original-created".into()
        )
    );
    let provenance: (String, bool) = db.query_row("select source_kind,confirmed_by_user from metadata_provenance where item_id='original-id' and field_name='title'", [], |r| Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(provenance, (" source bytes ".into(), true));
    assert_eq!(
        db.query_row("select count(*) from item_creators", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("select count(*) from identifiers", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(db);
    assert_eq!(
        fixture.count("creators"),
        1,
        "original orphan creators are retained"
    );
    assert_eq!(
        fixture.count("tags"),
        2,
        "original orphan tags are retained"
    );
    assert_eq!(fixture.reopen().count("bibliographic_items"), 1);
}

#[test]
fn candidate_order_and_provenance_use_plan_values_without_core_policy() {
    let fixture = Fixture::new();
    fixture.item("z-doi-first", "Other first", None);
    fixture.item("a-doi-second", "Other second", None);
    fixture.item("title-last", "POLICY TITLE", Some(2025));
    fixture.doi("z-doi-first", "doi-first");
    fixture.doi("a-doi-second", "doi-second");
    let observed = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
    let received = Arc::clone(&observed);
    let db = fixture.db.lock().unwrap();
    db.create_scalar_function(
        "observe_catalog_duplicate",
        2,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_INNOCUOUS,
        move |c| {
            received.lock().unwrap().push((c.get(0)?, c.get(1)?));
            Ok(1)
        },
    )
    .unwrap();
    db.execute_batch("create trigger observe_catalog_duplicate after insert on duplicate_candidates begin select observe_catalog_duplicate(new.reason,new.right_item_id); end;").unwrap();
    drop(db);
    let mut prepared = plan(Some("saved"));
    prepared.provenance.fields = vec!["from-native-field".into()];
    prepared.provenance.source_kind = "native source".into();
    prepared.provenance.confirmed_by_user = true;
    prepared.duplicates.normalized_title = "policy title".into();
    prepared.duplicates.doi_reason = "native doi".into();
    prepared.duplicates.doi_confidence = 0.41;
    prepared.duplicates.title_reason = "native title".into();
    prepared.duplicates.title_confidence = 0.22;
    fixture.persist(&prepared).unwrap();
    assert_eq!(
        *observed.lock().unwrap(),
        [
            ("native doi".into(), "z-doi-first".into()),
            ("native doi".into(), "a-doi-second".into()),
            ("native title".into(), "title-last".into()),
        ]
    );
    let db = fixture.db.lock().unwrap();
    let stored: (String, String, bool) = db
        .query_row(
            "select field_name,source_kind,confirmed_by_user from metadata_provenance",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        stored,
        ("from-native-field".into(), "native source".into(), true)
    );
    for (reason, confidence) in [("native doi", 0.41), ("native title", 0.22)] {
        let value: f64 = db
            .query_row(
                "select confidence from duplicate_candidates where reason=?1",
                [reason],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(value, confidence);
    }
}

#[test]
fn doi_candidates_are_collected_before_any_candidate_insert() {
    let fixture = Fixture::new();
    fixture.item("valid-first", "First", None);
    fixture.doi("valid-first", "first-doi");
    let observed = Arc::new(Mutex::new(0));
    let received = Arc::clone(&observed);
    let db = fixture.db.lock().unwrap();
    db.create_scalar_function(
        "observe_catalog_insert",
        0,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_INNOCUOUS,
        move |_| {
            *received.lock().unwrap() += 1;
            Ok(1)
        },
    )
    .unwrap();
    db.execute_batch("create trigger observe_catalog_insert after insert on duplicate_candidates begin select observe_catalog_insert(); end;").unwrap();
    let bytes = vec![0x01_u8, 0x02];
    db.execute("insert into bibliographic_items(id,item_type,title,created_at,updated_at) values(?1,'document','blob-id','before','before')", [&bytes]).unwrap();
    db.execute("insert into identifiers(id,item_id,kind,normalized_value,display_value) values('last-doi',?1,'doi','10.0/synthetic','10.0/Synthetic')", [&bytes]).unwrap();
    drop(db);
    let failure = fixture.persist(&plan(Some("new-item"))).unwrap_err();
    assert_eq!(failure.outcome, SaveOutcome::RolledBack);
    let SaveCause::Data(cause) = &failure.cause else {
        panic!("expected candidate read error");
    };
    assert_eq!(
        cause.resource_id,
        "literature.catalog-save.duplicate-doi-ids"
    );
    assert_eq!(
        *observed.lock().unwrap(),
        0,
        "the first row must not be inserted before later rows decode"
    );
    assert_eq!(fixture.count("bibliographic_items"), 2);
    assert_eq!(fixture.count("duplicate_candidates"), 0);
}

#[test]
fn hidden_updates_commit_before_domain_not_found_without_clearing_visibility() {
    let fixture = Fixture::new();
    for (id, column) in [("archived", "archived_at"), ("trashed", "trashed_at")] {
        fixture.persist(&plan(Some(id))).unwrap();
        fixture
            .db
            .lock()
            .unwrap()
            .execute(
                &format!("update bibliographic_items set {column}='hidden' where id=?1"),
                [id],
            )
            .unwrap();
        let mut update = plan(Some(id));
        update.item.title = "Committed hidden title".into();
        let result = fixture.persist(&update).unwrap();
        assert_eq!(result.item_id, id);
        assert_eq!(result.items.len(), 1);
        assert!(result.items[0].main.rows.is_empty());
        assert!(result.items[0].related.is_none());
        let value: String = fixture
            .db
            .lock()
            .unwrap()
            .query_row(
                "select title from bibliographic_items where id=?1",
                [id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(value, "Committed hidden title");
    }
    assert_eq!(fixture.reopen().count("bibliographic_items"), 2);
}

#[test]
fn capture_failure_keeps_explicit_commit_outcome_and_original_item_identity() {
    let fixture = Fixture::new();
    fixture.db.lock().unwrap().execute_batch("create trigger corrupt_creator_for_read after insert on creators begin update creators set family_name=x'01' where id=new.id; end;").unwrap();
    let failure = fixture.persist(&plan(Some("committed-item"))).unwrap_err();
    assert_eq!(
        failure.outcome,
        SaveOutcome::Committed {
            item_id: "committed-item".into()
        }
    );
    let SaveCause::Data(cause) = failure.cause else {
        panic!("capture must preserve data error");
    };
    assert_eq!(cause.resource_id, "literature.catalog.item-creators");
    assert!(fixture.db.lock().unwrap().is_autocommit());
    assert_eq!(fixture.reopen().count("bibliographic_items"), 1);
}
