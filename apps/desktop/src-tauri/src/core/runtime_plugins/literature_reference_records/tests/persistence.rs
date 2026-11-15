use super::*;
use rusqlite::params;

#[test]
fn create_update_preserve_original_draft_history_and_core_prefix_after_reopen() {
    let fixture = Fixture::new();
    let prefix = fixture.prefix();
    let original = draft();
    let created = fixture.save(original.clone()).unwrap();
    uuid::Uuid::parse_str(&created.id).unwrap();
    assert_eq!(created.raw_text, original.raw_text.trim());
    assert_eq!(created.revision, 1);
    assert_eq!(created.provenance_kind, "human_original");
    assert_eq!(created.matched_item_id, None);
    assert_eq!(created.usage_kind, None);
    assert_eq!(created.usage_anchor_id, None);
    let update = ReferenceRecordDraft {
        id: Some(created.id.clone()),
        fields: vec![ReferenceMetadataField {
            name: "title".into(),
            value: "  Human field stays untrimmed  ".into(),
            source_kind: "human".into(),
            confirmed_by_user: true,
        }],
        parse_state: "parsed".into(),
        identity_state: "matched_existing".into(),
        matched_item_id: Some("cited".into()),
        semantic_state: "reviewed".into(),
        usage_kind: Some("uses_method".into()),
        usage_summary: "  Explicit human assessment  ".into(),
        usage_anchor_id: Some("usage-anchor".into()),
        expected_revision: Some(1),
        change_reason: "  manual_review  ".into(),
        ..original.clone()
    };
    let reviewed = fixture.save(update.clone()).unwrap();
    assert_eq!(reviewed.id, created.id);
    assert_eq!(reviewed.created_at, created.created_at);
    assert_eq!(reviewed.revision, 2);
    assert_eq!(reviewed.fields, update.fields);
    assert_eq!(reviewed.usage_summary, "Explicit human assessment");
    assert_eq!(reviewed.matched_item_id.as_deref(), Some("cited"));
    let snapshots: Vec<(i64, String, String, String)> = {
        let db = fixture.db.lock().unwrap();
        let mut statement = db
            .prepare(
                "select revision,snapshot_json,editor_kind,change_reason
             from reference_record_revisions where reference_record_id=?1 order by revision",
            )
            .unwrap();
        statement
            .query_map([&created.id], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    };
    assert_eq!(
        snapshots,
        vec![
            (
                1,
                serde_json::to_string(&original).unwrap(),
                "human".into(),
                "manual_capture".into()
            ),
            (
                2,
                serde_json::to_string(&update).unwrap(),
                "human".into(),
                "manual_review".into()
            ),
        ]
    );
    assert_eq!(fixture.prefix(), prefix);
    let fixture = fixture.reopen();
    assert_eq!(fixture.prefix(), prefix);
    assert_eq!(fixture.records("citing").unwrap(), vec![reviewed]);
    assert_eq!(fixture.count("reference_record_revisions"), 2);
}

#[test]
fn readback_uses_actual_trigger_values_without_revalidating_stored_field_policy() {
    let fixture = Fixture::new();
    fixture.db.lock().unwrap().execute_batch(
        "create temp trigger synthetic_reference_after_insert after insert on reference_records
         begin update reference_records set raw_text='Changed by actual SQL trigger',
         fields_json='[{\"name\":\"legacy-name\",\"value\":\"legacy value\",\"sourceKind\":\"legacy-source\",\"confirmedByUser\":true}]'
         where id=new.id; end;",
    ).unwrap();
    let original = draft();
    let returned = fixture.save(original.clone()).unwrap();
    assert_eq!(returned.raw_text, "Changed by actual SQL trigger");
    assert_eq!(
        returned.fields,
        vec![ReferenceMetadataField {
            name: "legacy-name".into(),
            value: "legacy value".into(),
            source_kind: "legacy-source".into(),
            confirmed_by_user: true,
        }]
    );
    let snapshot: String = fixture
        .db
        .lock()
        .unwrap()
        .query_row(
            "select snapshot_json from reference_record_revisions where reference_record_id=?1",
            [&returned.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(snapshot, serde_json::to_string(&original).unwrap());
    assert_eq!(fixture.records("citing").unwrap(), vec![returned.clone()]);
    assert_eq!(fixture.reopen().records("citing").unwrap(), vec![returned]);
}

#[test]
fn signed_revisions_remain_exact_above_javascript_range_and_at_i64_max() {
    let fixture = Fixture::new();
    let record = fixture.save(draft()).unwrap();
    for current in [9_007_199_254_740_993_i64, i64::MAX - 1] {
        fixture
            .db
            .lock()
            .unwrap()
            .execute(
                "update reference_records set current_revision=?1 where id=?2",
                params![current, record.id],
            )
            .unwrap();
        let saved = fixture
            .save(ReferenceRecordDraft {
                id: Some(record.id.clone()),
                expected_revision: Some(current),
                ..draft()
            })
            .unwrap();
        assert_eq!(saved.revision, current + 1);
        assert_eq!(fixture.records("citing").unwrap()[0].revision, current + 1);
    }
    let fixture = fixture.reopen();
    assert_eq!(fixture.records("citing").unwrap()[0].revision, i64::MAX);
    let revisions: Vec<i64> = {
        let db = fixture.db.lock().unwrap();
        let mut statement = db
            .prepare("select revision from reference_record_revisions order by revision")
            .unwrap();
        statement
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    };
    assert_eq!(revisions, [1, 9_007_199_254_740_994_i64, i64::MAX]);
}

#[test]
fn list_keeps_sql_order_and_does_not_normalize_the_original_paper_argument() {
    let fixture = Fixture::new();
    let first = fixture.save(draft()).unwrap();
    let second = fixture
        .save(ReferenceRecordDraft {
            raw_text: "Second synthetic record".into(),
            ..draft()
        })
        .unwrap();
    {
        let db = fixture.db.lock().unwrap();
        db.execute(
            "update reference_records set updated_at='2020' where id=?1",
            [&first.id],
        )
        .unwrap();
        db.execute(
            "update reference_records set updated_at='2030' where id=?1",
            [&second.id],
        )
        .unwrap();
    }
    assert_eq!(
        fixture
            .records("citing")
            .unwrap()
            .iter()
            .map(|record| record.id.as_str())
            .collect::<Vec<_>>(),
        [second.id.as_str(), first.id.as_str()]
    );
    assert!(fixture.records(" citing ").unwrap().is_empty());
    assert!(fixture.records("").unwrap().is_empty());
    assert!(fixture.records("missing").unwrap().is_empty());
}
