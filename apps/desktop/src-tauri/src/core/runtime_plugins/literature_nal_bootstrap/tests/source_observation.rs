use super::{CaptureError, Fixture, fixture};
use crate::{
    data_interface::plugin_resources::{ColumnCodec as C, RowPolicy as P, write_in},
    runtime_plugins::literature_nal_bootstrap::rows::{call, read},
};
use research_workbench_native_sdk::{data::DataValue as V, nal_bootstrap::BootstrapPhaseV1};

#[test]
fn original_source_read_detects_changes_caused_by_an_earlier_fixed_write() {
    let fixture = Fixture::new();
    fixture.collection();
    let mut db = fixture.db.lock().unwrap();
    let previous = fixture.resources.capture_source(&db);
    let (probes, identities) = fixture::empty_preparation();
    let tx = db.transaction().unwrap();
    let captured = fixture
        .resources
        .capture_probes_in(&tx, &previous, &probes, &identities)
        .unwrap();
    tx.commit().unwrap();

    let tx = db.transaction().unwrap();
    captured.recheck_in(&tx).unwrap();
    let resource = &fixture.resources.legacy_types;
    let codecs = &[C::String, C::String, C::String, C::Bool];
    let unchanged = read(&tx, resource, vec![], P::All, codecs).unwrap();
    captured
        .check_source_rows(BootstrapPhaseV1::RelationTypes, resource, &[], &unchanged)
        .unwrap();

    // The original term insert is earlier than the original relation-types
    // read. Its test trigger changes a later source after the global T1 proof.
    tx.execute_batch("create temp trigger test_earlier_term_changes_type after insert on terms begin update collection_relation_types set inverse_name='changed by the earlier term insert' where id='sub-field';end;").unwrap();
    write_in(
        &tx,
        &fixture.resources.term_insert,
        &call(
            &fixture.resources.term_insert,
            vec![
                V::Text("source-trigger-term".into()),
                V::Text("Trigger".into()),
                V::Text("trigger".into()),
                V::Text(String::new()),
                V::Text("human_original".into()),
            ],
        ),
    )
    .unwrap();

    let actual = read(&tx, resource, vec![], P::All, codecs).unwrap();
    assert_eq!(actual.rows.len(), unchanged.rows.len());
    assert_ne!(actual, unchanged);
    assert!(matches!(
        captured.check_source_rows(BootstrapPhaseV1::RelationTypes, resource, &[], &actual),
        Err(CaptureError::SnapshotChanged { cause: None })
    ));
    tx.rollback().unwrap();
}
