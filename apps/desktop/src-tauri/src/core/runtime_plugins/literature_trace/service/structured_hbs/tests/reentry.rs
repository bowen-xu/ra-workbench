use super::*;
use crate::ai::structured_resource_validation::DecisionOrigin;
use research_workbench_native_sdk::candidate_contracts::CandidateValidationError;

#[tokio::test]
async fn unsettled_transaction_upgrades_existing_read_failure_to_unknown_and_fences_owner() {
    let fixture = Fixture::new(&[], false, RuntimeEvents::new(|_, _| Ok(())));
    {
        let db = fixture.store.connection().unwrap();
        db.execute_batch("begin").unwrap();
        assert!(!db.is_autocommit());
    }
    let error = match fixture
        .service
        .structured_hbs(
            fixture.store.clone(),
            "00000000-0000-4000-8000-000000000712".into(),
            HbsOperation::ReadPreservation,
        )
        .await
    {
        Err(error) => error,
        Ok(_) => panic!("missing resource with an unsettled transaction"),
    };
    let HbsOperationError::Boundary(boundary) = &error else {
        panic!("original read failure must be retained")
    };
    assert!(boundary.failure.error.starts_with("MODEL_NOT_FOUND"));
    assert!(matches!(
        boundary.failure.cause,
        crate::ai::structured_resource_validation::FailureCause::Original
    ));
    assert_eq!(boundary.phase, "read");
    assert!(boundary.rollback_error.is_none());
    assert_eq!(boundary.outcome, HbsOutcome::Unknown);
    assert!(boundary.needs_recovery());
    let (_, diagnostic) = error.into_wire();
    let diagnostic = diagnostic.expect("unknown settlement keeps its technical marker");
    assert_eq!(diagnostic.kind, "technical");
    assert_eq!(diagnostic.outcome, HbsOutcome::Unknown);
    assert!(diagnostic.native_request_id.is_none());
    assert!(fixture.store.connection().is_err());
    assert!(fixture.service.host.lock().await.is_none());
    assert!(fixture.lifecycle().is_empty());
    fixture.service.shutdown().await.unwrap();
}

#[tokio::test]
async fn durable_receipt_and_cas_take_priority_over_a_pending_cached_rejection() {
    let values = [portrait(0), portrait(1)];
    let fixture = Fixture::new(&values, false, RuntimeEvents::new(|_, _| Ok(())));
    let request = || crate::hbs_storage::Commit {
        id: "core-reentry:1".into(),
        expected_version: 0,
        saved: values[0].clone(),
        baseline: values[1].clone(),
    };
    let receipt = fixture
        .service
        .structured_hbs(
            fixture.store.clone(),
            fixture.id.clone(),
            HbsOperation::Commit(request()),
        )
        .await
        .unwrap_or_else(|_| panic!("initial actual owner commit"));
    let sources = || {
        let mut sources = RuntimeHbsSources {
            reading: Some(selection::select(&fixture.service.catalog, Some(&fixture.key)).unwrap()),
            ..RuntimeHbsSources::default()
        };
        assert!(sources.policy.store(
            0,
            input(&values[0]),
            Err(CandidateValidationError::InvalidInput(
                "cached rejection must remain unused".into()
            )),
            DecisionOrigin {
                package_key: fixture.key.clone(),
                instance_id: "synthetic-unused-origin".into(),
                native_request_id: "synthetic-unused-native:1".into()
            }
        ));
        sources
    };
    let (_, replay) = worker::attempt(
        fixture.store.clone(),
        fixture.id.clone(),
        Arc::new(HbsOperation::Commit(request())),
        sources(),
    )
    .await
    .unwrap_or_else(|_| panic!("original receipt replay"));
    assert_eq!(replay.unwrap(), receipt);
    let mut stale = request();
    stale.id = "core-reentry:2".into();
    let (_, conflict) = worker::attempt(
        fixture.store.clone(),
        fixture.id.clone(),
        Arc::new(HbsOperation::Commit(stale)),
        sources(),
    )
    .await
    .unwrap_or_else(|_| panic!("original CAS"));
    assert_eq!(conflict.unwrap()["type"], "conflict");
    let mut reused = request();
    reused.saved = portrait(99);
    let (_, reused) = worker::attempt(
        fixture.store.clone(),
        fixture.id.clone(),
        Arc::new(HbsOperation::Commit(reused)),
        sources(),
    )
    .await
    .unwrap_or_else(|_| panic!("original request fingerprint"));
    match reused {
        Err(BoundaryError::Failure(error)) => {
            assert!(error.failure.error.starts_with("MODEL_REQUEST_ID_REUSED"))
        }
        _ => panic!("request fingerprint refusal before cached policy"),
    }
    assert_eq!(
        fixture
            .lifecycle()
            .lines()
            .filter(|line| line.starts_with("call "))
            .count(),
        2
    );
    fixture.service.shutdown().await.unwrap();
}
