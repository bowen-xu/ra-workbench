use super::*;
use fixture::{Fixture, WAIT};
use research_agent_core::tool_registry::OwnedToolRegistration;
mod compatibility;
mod fixture;
mod lifecycle;
mod paper_preparation;
mod paper_preview;
mod paper_retrieval;
mod paper_submission;
mod paper_turn_sources;
mod revalidation;

#[tokio::test]
async fn original_tool_registration_reads_only_granted_page_and_reuses_canonical_snapshot_after_reopen()
 {
    let mut fixture = Fixture::new();
    let prepared = fixture.prepared();
    let expected = prepared.projection.snapshot_id.clone();
    fixture.configure(
        &[
            prepared.build.clone(),
            prepared.project.clone(),
            prepared.build,
            prepared.project,
            prepared.digest,
        ],
        None,
        false,
    );
    assert!(fixture.store.require_paper_reading_source().is_err());
    let frozen = crate::agent::source_tools::OwnedSubmissionTools::new(fixture.record.clone());
    let registration = crate::agent::context_session::resource_tools::registration(
        fixture.store.clone(),
        fixture.record.clone(),
        OwnedToolRegistration::new(frozen.definitions(), Arc::new(frozen)),
    );
    let (_, executor) = registration.into_parts();
    let output = executor
        .authorize_and_execute(fixture.call.clone())
        .await
        .unwrap();
    assert!(!output.is_error, "{}", output.content);
    assert!(output.content.contains("Authorized source"));
    assert!(output.content.contains("evidenceItemId"));
    assert!(!output.content.contains("Unshared secret"));
    assert!(!output.content.contains("Ungranted page"));
    assert_eq!(fixture.proof()["snapshotId"], expected);
    assert_eq!(fixture.counts(), (1, 1));
    let first = fixture.proof();
    fixture.read().await.unwrap();
    assert_eq!(fixture.counts(), (1, 1));
    assert_eq!(fixture.proof(), first);
    fixture.service.shutdown().await.unwrap();
    assert_eq!(
        fixture
            .lifecycle()
            .lines()
            .filter(|line| line.starts_with("call "))
            .count(),
        5
    );
    assert!(fixture.lifecycle().ends_with("shutdown\n"));
    drop(executor);
    drop(fixture.store);
    let reopened = fixture::open(fixture.data.path());
    let db = reopened.connection().unwrap();
    assert_eq!(fixture::counts(&db), (1, 1));
    let id: String = db
        .query_row("select id from paper_context_snapshots", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(id, expected);
    let hidden:i64=db.query_row("select count(*) from paper_context_items where attachment_id<>'selected' or page_index<>0 or page_index is null",[],|row|row.get(0)).unwrap();
    assert_eq!(hidden, 0);
}

#[tokio::test]
async fn trusted_frozen_anchor_uses_actual_snapshot_geometry_without_rebuilding_parse() {
    let mut fixture = Fixture::new();
    let prepared = fixture.prepared();
    let expected = prepared.projection.snapshot_id.clone();
    fixture.configure(&[prepared.build, prepared.project], None, false);
    fixture.read().await.unwrap();
    fixture.snapshot_submission(&expected);
    // This changes current parse content, not the already frozen source. The
    // anchored branch must read Core geometry and immutable evidence rows.
    fixture.raw_parse("{}");
    let value = fixture.read().await.unwrap();
    assert!(value.to_string().contains("Authorized source"));
    assert!(value.to_string().contains("#/texts/0"));
    assert!(value.to_string().contains("parse-selected"));
    assert!(value.to_string().contains("regions"));
    assert_eq!(fixture.proof()["snapshotId"], expected);
    assert_eq!(fixture.counts(), (1, 2));
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

#[tokio::test]
async fn transport_failure_and_wrong_input_echo_publish_no_proof_or_snapshot() {
    for wrong_echo in [false, true] {
        let mut fixture = Fixture::new();
        let mut prepared = fixture.prepared();
        if wrong_echo {
            let research_workbench_native_sdk::catalog::CatalogOutput::PaperContext { outcome } =
                &mut prepared.build
            else {
                panic!("finite context output")
            };
            *outcome.input =
                research_workbench_native_sdk::paper_context_wire::PaperContextInputV1::Build {
                    cbor_base64: "not-the-actual-input".into(),
                };
        }
        fixture.configure(&[prepared.build], None, !wrong_echo);
        let error = fixture.read().await.unwrap_err();
        let output = error.into_output(&fixture.record, &fixture.call);
        let diagnostic: Value = serde_json::from_str(&output.content).unwrap();
        assert!(output.is_error);
        assert_eq!(
            diagnostic["requestId"],
            fixture.record.request_id.to_string()
        );
        assert_eq!(diagnostic["turnId"], fixture.record.turn_id.to_string());
        assert_eq!(diagnostic["callId"], fixture.call.call_id);
        assert_eq!(diagnostic["nativeFailure"]["packageKey"], fixture.key);
        assert!(
            diagnostic["nativeFailure"]["nativeRequestId"]
                .as_str()
                .is_some_and(|id| !id.is_empty())
        );
        if wrong_echo {
            assert_eq!(
                diagnostic["nativeFailure"]["code"],
                "plugin_host_response_mismatch"
            );
        }
        assert_eq!(fixture.counts(), (0, 0));
        let _ = fixture.service.shutdown().await;
    }
}

#[tokio::test]
async fn failed_original_proof_write_retains_durable_snapshot_and_fences_unknown_owner() {
    let mut fixture = Fixture::new();
    let prepared = fixture.prepared();
    let expected = prepared.projection.snapshot_id.clone();
    fixture.configure(&[prepared.build, prepared.project], None, false);
    fixture.store.connection().unwrap().execute_batch("create trigger reject_read_proof before insert on agent_text_read_scopes begin select raise(abort,'PRIVATE_READ_PROOF_DRIVER');end;").unwrap();
    let error = fixture.read().await.unwrap_err();
    let output = error.into_output(&fixture.record, &fixture.call);
    let value: Value = serde_json::from_str(&output.content).unwrap();
    assert!(output.is_error);
    assert_eq!(value["durableSnapshotId"], expected);
    assert_eq!(value["readProofConfirmed"], false);
    assert_eq!(value["nativeFailure"]["outcome"], "unknown");
    assert!(!output.content.contains("PRIVATE_READ_PROOF_DRIVER"));
    assert!(
        fixture.store.connection().is_err(),
        "unknown final proof must fence its owner"
    );
    fixture.service.shutdown().await.unwrap();
    let db =
        rusqlite::Connection::open(fixture.data.path().join("research-assistant.sqlite3")).unwrap();
    assert_eq!(fixture::counts(&db), (1, 0));
    assert_eq!(
        db.query_row("select id from paper_context_snapshots", [], |row| row
            .get::<_, String>(
            0
        ))
        .unwrap(),
        expected
    );
}
