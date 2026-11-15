use super::{error::CaptureError, resources::BootstrapResources};
use crate::runtime_plugins::RuntimePluginCatalog;
use research_workbench_native_sdk::{data::DataValue, nal_bootstrap::*};
use rusqlite::{
    Connection,
    hooks::{AuthAction, AuthContext, Authorization},
};

mod fixture;
mod metadata;
mod prepare;
mod source_observation;
use fixture::Fixture;

#[test]
fn original_done_and_empty_branches_skip_unavailable_legacy_tables() {
    let fixture = Fixture::new();
    let mut db = fixture.db.lock().unwrap();
    db.execute(
        "update nal_state set legacy_imported=1,relation_semantics_version=1",
        [],
    )
    .unwrap();
    db.authorizer(Some(|context: AuthContext<'_>| match context.action {
        AuthAction::Read { table_name, .. }
            if table_name.starts_with("collection")
                || table_name == "item_relation_assignments" =>
        {
            Authorization::Deny
        }
        _ => Authorization::Allow,
    }));
    let source = fixture.resources.capture_source(&db).source;
    assert!(matches!(source.collections, SourceProbe::Skipped {}));
    assert!(matches!(source.relation_types, SourceProbe::Skipped {}));
    assert!(matches!(source.relations, SourceProbe::Skipped {}));
    assert!(matches!(source.memberships, SourceProbe::Skipped {}));
    assert!(matches!(source.assignments, SourceProbe::Skipped {}));
    assert!(matches!(source.repair_records, SourceProbe::Skipped {}));
    assert!(matches!(
        source.configured_embedding,
        SourceProbe::Skipped {}
    ));
    db.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    db.execute("update nal_state set legacy_imported=0", [])
        .unwrap();
    let source = fixture.resources.capture_source(&db).source;
    assert!(matches!(source.collections,SourceProbe::Available{value} if value.is_empty()));
    assert!(matches!(source.relation_types, SourceProbe::Skipped {}));
    let previous = fixture.resources.capture_source(&db);
    let (preparation, identities) = fixture::empty_preparation();
    let tx = db.transaction().unwrap();
    let prepared = fixture
        .resources
        .capture_probes_in(&tx, &previous, &preparation, &identities)
        .unwrap();
    tx.commit().unwrap();
    let actual = super::rows::read(
        &db,
        &fixture.resources.legacy_assignments,
        vec![],
        crate::data_interface::plugin_resources::RowPolicy::All,
        &[crate::data_interface::plugin_resources::ColumnCodec::String; 2],
    )
    .unwrap();
    assert!(actual.rows.is_empty());
    assert!(matches!(
        prepared.check_source_rows(
            BootstrapPhaseV1::Assignments,
            &fixture.resources.legacy_assignments,
            &[],
            &actual
        ),
        Err(CaptureError::SnapshotChanged { .. })
    ));
}

#[test]
fn source_capture_keeps_invalid_fact_json_raw_until_original_load() {
    let fixture = Fixture::new();
    let db = fixture.db.lock().unwrap();
    let invalid = r#"{"kind":"variable","name":"not-a-fact"}"#;
    db.execute(
        "insert into nal_statements(id,subject,predicate,origin) values('raw',?1,?1,'human')",
        [invalid],
    )
    .unwrap();
    let source = fixture.resources.capture_source(&db).source;
    let SourceProbe::Available { value } = source.raw_state else {
        panic!("raw capture decoded facts")
    };
    assert_eq!(value.statements[0].subject_json, invalid);
    assert_eq!(value.statements[0].predicate_json, invalid);
}

#[test]
fn r1_recaptures_full_source_even_when_nal_revision_is_unchanged() {
    let fixture = Fixture::new();
    fixture.collection();
    let mut db = fixture.db.lock().unwrap();
    let source = fixture.resources.capture_source(&db);
    let SourceProbe::Available { value: before } = &source.source.raw_state else {
        panic!("the baseline state was not captured")
    };
    let revision_before = before.revision;
    let SourceProbe::Available { value: collections } = &source.source.collections else {
        panic!("the original collection input was not captured")
    };
    assert_eq!(collections[0].description, "Original description");
    // Migration 0171 legitimately initialized the NAL revision above zero.
    // The collections.description input has no NAL revision trigger: changing
    // this captured field must still invalidate R0 even with the same revision.
    db.execute(
        "update collections set description='changed-without-nal-revision' where id='source'",
        [],
    )
    .unwrap();
    assert_eq!(
        db.query_row("select revision from nal_state where id=1", [], |row| row
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        revision_before
    );
    assert_eq!(
        db.query_row(
            "select description from collections where id='source'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "changed-without-nal-revision"
    );
    let tx = db.transaction().unwrap();
    let (preparation, identities) = fixture::empty_preparation();
    let result = fixture
        .resources
        .capture_probes_in(&tx, &source, &preparation, &identities);
    assert!(matches!(result, Err(CaptureError::SnapshotChanged { .. })));
    tx.rollback().unwrap();
}

#[test]
fn r1_absence_probe_is_owned_and_t1_rechecks_the_whole_result() {
    let fixture = Fixture::new();
    fixture.collection();
    let mut db = fixture.db.lock().unwrap();
    let source = fixture.resources.capture_source(&db);
    let (preparation, identities) = fixture::preparation(&["absent"]);
    let tx = db.transaction().unwrap();
    let prepared = fixture
        .resources
        .capture_probes_in(&tx, &source, &preparation, &identities)
        .unwrap();
    assert!(matches!(
        prepared.facts.terms[0].result,
        SourceProbe::Available { value: None }
    ));
    tx.commit().unwrap();
    drop(db);
    fixture.term("new-term", "absent", "Actual term");
    let mut db = fixture.db.lock().unwrap();
    let tx = db.transaction().unwrap();
    assert!(matches!(
        prepared.recheck_in(&tx),
        Err(CaptureError::SnapshotChanged { .. })
    ));
    tx.rollback().unwrap();
}

#[test]
fn a_future_failed_query_is_replayed_only_at_its_original_phase() {
    let fixture = Fixture::new();
    fixture.collection();
    let mut db = fixture.db.lock().unwrap();
    db.authorizer(Some(|context: AuthContext<'_>| match context.action {
        AuthAction::Read {
            table_name: "collection_relation_types",
            ..
        } => Authorization::Deny,
        _ => Authorization::Allow,
    }));
    let source = fixture.resources.capture_source(&db);
    assert!(matches!(
        source.source.relation_types,
        SourceProbe::Deferred {}
    ));
    let (preparation, identities) = fixture::empty_preparation();
    let tx = db.transaction().unwrap();
    let prepared = fixture
        .resources
        .capture_probes_in(&tx, &source, &preparation, &identities)
        .unwrap();
    tx.commit().unwrap();
    db.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    let tx = db.transaction().unwrap();
    prepared.recheck_in(&tx).unwrap();
    prepared
        .require_phase_in(&tx, BootstrapPhaseV1::TermValidation)
        .unwrap();
    let actual = super::rows::read(
        &tx,
        &fixture.resources.legacy_types,
        vec![],
        crate::data_interface::plugin_resources::RowPolicy::All,
        &[
            crate::data_interface::plugin_resources::ColumnCodec::String,
            crate::data_interface::plugin_resources::ColumnCodec::String,
            crate::data_interface::plugin_resources::ColumnCodec::String,
            crate::data_interface::plugin_resources::ColumnCodec::Bool,
        ],
    )
    .unwrap();
    assert!(matches!(
        prepared.check_source_rows(
            BootstrapPhaseV1::RelationTypes,
            &fixture.resources.legacy_types,
            &[],
            &actual
        ),
        Err(CaptureError::SnapshotChanged { .. })
    ));
    assert!(matches!(
        prepared.require_phase_in(&tx, BootstrapPhaseV1::RelationTypes),
        Err(CaptureError::SnapshotChanged { .. })
    ));
    tx.rollback().unwrap();
}

#[test]
fn later_term_readback_failure_cannot_replace_an_earlier_probe() {
    let fixture = Fixture::new();
    fixture.collection();
    fixture.term("bad", "later", "later");
    let mut db = fixture.db.lock().unwrap();
    db.execute(
        "update terms set text=cast(x'FF' as text) where id='bad'",
        [],
    )
    .unwrap();
    let source = fixture.resources.capture_source(&db);
    let (preparation, identities) = fixture::preparation(&["first", "later"]);
    let tx = db.transaction().unwrap();
    let prepared = fixture
        .resources
        .capture_probes_in(&tx, &source, &preparation, &identities)
        .unwrap();
    assert_eq!(
        prepared.facts.terms[1].failure_phase,
        Some(BootstrapPhaseV1::TermReadback)
    );
    prepared.recheck_in(&tx).unwrap();
    prepared
        .require_probe_in(
            &tx,
            BootstrapPhaseV1::TermLookup,
            &fixture.resources.term_lookup,
            &[
                DataValue::Text("first".into()),
                DataValue::Text("en".into()),
            ],
        )
        .unwrap();
    assert!(matches!(
        prepared.require_probe_in(
            &tx,
            BootstrapPhaseV1::TermReadback,
            &fixture.resources.term_read,
            &[DataValue::Text("bad".into())]
        ),
        Err(CaptureError::Data(_))
    ));
    tx.rollback().unwrap();
}

#[test]
fn occupied_names_use_sqlite_trim_bytes_without_decoding_unrelated_bad_utf8() {
    let fixture = Fixture::new();
    fixture.collection();
    fixture.concept("utf8", " \tA\u{2003} ");
    fixture.concept("bad", "temporary");
    let db = fixture.db.lock().unwrap();
    db.execute(
        "update concepts set label=cast(x'FF00' as text) where id='bad'",
        [],
    )
    .unwrap();
    let source = fixture.resources.capture_source(&db).source;
    assert!(matches!(source.raw_state, SourceProbe::Deferred {}));
    let SourceProbe::Available { value: keys } = source.occupied_name_keys else {
        panic!("name key capture failed")
    };
    assert!(keys.contains(&"FF00".into()));
    assert!(keys.contains(&"0941E28083".into()));
}
