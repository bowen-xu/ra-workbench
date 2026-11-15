use super::*;
use crate::runtime_events::RuntimeEvents;
mod fixture;
mod lifecycle;
mod reentry;
mod review_flow;
mod write_flow;
use fixture::*;

#[tokio::test]
async fn all_null_preservation_accepts_resources_without_starting_a_host_or_pure_frame() {
    let fixture = Fixture::new(&[], false, RuntimeEvents::new(|_, _| Ok(())));
    let checkpoint = checkpoint(&vec![Value::Null; 6]);
    let result = fixture
        .service
        .structured_hbs(
            fixture.store.clone(),
            fixture.id.clone(),
            HbsOperation::SavePreservation(checkpoint.clone()),
        )
        .await
        .unwrap_or_else(|_| panic!("all-null preservation"));
    assert_eq!(result["revision"], 1);
    assert!(fixture.service.host.lock().await.is_none());
    assert!(fixture.lifecycle().is_empty());
    let restored = fixture
        .service
        .structured_hbs(
            fixture.store.clone(),
            fixture.id.clone(),
            HbsOperation::ReadPreservation,
        )
        .await
        .unwrap_or_else(|_| panic!("read all-null"));
    assert_eq!(restored, checkpoint);
    fixture.service.shutdown().await.unwrap();
}

#[tokio::test]
async fn six_native_needs_replay_the_full_stable_prefix_and_preserve_the_original_checkpoint() {
    let values = (0..6).map(portrait).collect::<Vec<_>>();
    let fixture = Fixture::new(&values, false, RuntimeEvents::new(|_, _| Ok(())));
    let value = checkpoint(&values);
    fixture
        .service
        .structured_hbs(
            fixture.store.clone(),
            fixture.id.clone(),
            HbsOperation::SavePreservation(value.clone()),
        )
        .await
        .unwrap_or_else(|_| panic!("six validation needs"));
    assert_eq!(
        fixture
            .lifecycle()
            .lines()
            .filter(|line| line.starts_with("call "))
            .count(),
        6
    );
    let db = fixture.store.connection().unwrap();
    let raw: String = db
        .query_row(
            "select checkpoint_json from hbs_preservations where resource_id=?1",
            [&fixture.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(serde_json::from_str::<Value>(&raw).unwrap(), value);
    assert_eq!(
        crate::structured_resources::get(&db, &fixture.id)
            .unwrap()
            .revision,
        0
    );
    drop(db);
    fixture.service.shutdown().await.unwrap();
}

#[tokio::test]
async fn native_rejected_frame_keeps_the_matched_origin_and_does_not_write_a_checkpoint() {
    let values = (0..6).map(portrait).collect::<Vec<_>>();
    let fixture = Fixture::new(&values[..1], true, RuntimeEvents::new(|_, _| Ok(())));
    let error = match fixture
        .service
        .structured_hbs(
            fixture.store.clone(),
            fixture.id.clone(),
            HbsOperation::SavePreservation(checkpoint(&values)),
        )
        .await
    {
        Err(error) => error,
        Ok(_) => panic!("original pure rejection"),
    };
    let (message, diagnostic) = error.into_wire();
    assert!(message.starts_with("MODEL_RECOVERY_INVALID"));
    let diagnostic = diagnostic.unwrap();
    assert_eq!(diagnostic.kind, "pureRejected");
    assert_eq!(diagnostic.outcome, HbsOutcome::RolledBack);
    assert_eq!(
        diagnostic.package_key.as_deref(),
        Some(fixture.key.as_str())
    );
    assert!(diagnostic.instance_id.is_some());
    assert!(diagnostic.native_request_id.is_some());
    let db = fixture.store.connection().unwrap();
    assert_eq!(
        db.query_row(
            "select count(*) from hbs_preservations where resource_id=?1",
            [&fixture.id],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert!(db.is_autocommit());
    drop(db);
    fixture.service.shutdown().await.unwrap();
}

#[tokio::test]
async fn latest_source_codec_failure_is_data_not_policy_and_does_not_initialize() {
    let fixture = Fixture::new(&[], false, RuntimeEvents::new(|_, _| Ok(())));
    fixture.store.connection().unwrap().execute("insert into paper_context_snapshots(id,paper_id,context_hash,context_schema_version,created_at) values(x'00','paper',?1,1,'now')",["d".repeat(64)]).unwrap();
    let values = (0..6).map(portrait).collect::<Vec<_>>();
    let error = match fixture
        .service
        .structured_hbs(
            fixture.store.clone(),
            fixture.id.clone(),
            HbsOperation::SavePreservation(checkpoint(&values)),
        )
        .await
    {
        Err(error) => error,
        Ok(_) => panic!("source codec failure"),
    };
    let (_, diagnostic) = error.into_wire();
    let diagnostic = diagnostic.unwrap();
    assert_eq!(diagnostic.kind, "sourceData");
    assert_eq!(
        diagnostic.data_error.unwrap().resource_id,
        "literature.reading-evidence.validation-latest"
    );
    assert!(diagnostic.native_request_id.is_none());
    assert!(fixture.service.host.lock().await.is_none());
    assert!(fixture.lifecycle().is_empty());
    fixture.service.shutdown().await.unwrap();
}
