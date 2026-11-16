//! Controlled host faults over the same Core confirmation entry and canonical finite rules.
use super::*;
use crate::{
    ai::error::AiError, runtime_plugins::literature_paper_confirmation::ConfirmationResult,
};
use research_workbench_native_sdk::{
    catalog::CatalogOutput,
    paper_confirmation::{unit::UnitInputV1, wire::PaperConfirmationInputV1 as Input},
};
use std::fs;

fn output(input: Input) -> CatalogOutput {
    protocol::catalog(
        research_workbench_native_sdk::catalog::CatalogInput::PreparePaperConfirmation {
            input: Box::new(PaperConfirmationWireInputV1 {
                cbor_base64: encode_payload(&input).unwrap(),
            }),
        },
    )
}
pub(super) fn request() -> Request {
    Request::ConfirmOverview {
        draft: AiOverviewFieldConfirmationDraft {
            candidate_id: "legacy-candidate".into(),
            candidate_revision: 1,
            field_kinds: vec!["approach".into(), "conclusion".into()],
            scope_kind: "paper".into(),
            project_id: None,
            idempotency_key: "controlled-confirmation".into(),
        },
    }
}
fn inputs(f: &Fixture) -> Vec<Input> {
    let candidate = seed(f);
    let Request::ConfirmOverview { draft } = request() else {
        unreachable!()
    };
    let stored_review_state = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "select review_state from ai_candidates where id=?1",
            [&candidate.id],
            |row| row.get(0),
        )
        .unwrap();
    let qualified = protocol::qualify(&candidate, &draft).unwrap();
    let mut inputs = vec![
        Input::Request { request: request() },
        Input::OverviewCandidate {
            stored_review_state,
            candidate: Box::new(candidate.clone()),
            draft: draft.clone(),
        },
        Input::OverviewFields {
            overview: Box::new(qualified.overview.clone()),
            requested_fields: draft.field_kinds,
            confirmed_fields: vec![],
        },
    ];
    for field in &qualified.overview.fields {
        let input = Input::OverviewField {
            candidate: Box::new(candidate.clone()),
            overview: Box::new(qualified.overview.clone()),
            field: Box::new(field.clone()),
        };
        let PaperConfirmationDecisionV1::OverviewField { plan } =
            protocol::confirmation(input.clone()).unwrap()
        else {
            panic!("field plan")
        };
        let unit = plan.unit.unwrap();
        inputs.push(input);
        inputs.push(Input::Unit {
            scope_key: field.field.clone(),
            input: UnitInputV1::Validate {
                title: unit.title,
                body: unit.body,
                unit_kind: unit.unit_kind,
                provenance_kind: unit.provenance_kind,
                epistemic_role: unit.epistemic_role,
                anchor_count: plan.anchors.len(),
            },
        });
    }
    inputs
}
fn configure(f: &mut Fixture, outputs: &[CatalogOutput], gate: Option<usize>, fail: bool) {
    let directory = f.package.path().join("writing");
    let manifest = directory.join("manifest.json");
    let mut value: Value = serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    if !value["provides"]
        .as_array()
        .unwrap()
        .iter()
        .any(|cap| cap["id"] == "literature.paper-confirmation")
    {
        value["provides"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"literature.paper-confirmation","version":1}));
    }
    fs::write(manifest, serde_json::to_vec(&value).unwrap()).unwrap();
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/fixtures/native-resources-v1/literature");
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        let name = name.to_str().unwrap();
        if name.starts_with("paper-confirmation-")
            || matches!(
                name,
                "reading-unit-accepted-insert.v1.sql"
                    | "reading-unit-accepted-evidence.v1.sql"
                    | "nal-bootstrap-research-revision-insert.v1.sql"
            )
        {
            fs::copy(entry.path(), directory.join("data").join(name)).unwrap();
        }
    }
    f.configure(outputs, gate, fail);
}
fn spawn(f: &Fixture) -> tokio::task::JoinHandle<Result<ConfirmationResult, AiError>> {
    let store = f.store.clone();
    tokio::spawn(
        async move { crate::paper_confirmation_app::execute_selected(&store, request()).await },
    )
}
pub(super) fn counts(f: &Fixture) -> (i64, i64, i64, i64) {
    let db = f.store.connection().unwrap();
    assert!(db.is_autocommit());
    db.query_row("select (select count(*) from structured_reading_units), (select count(*) from ai_overview_confirmation_batches), (select count(*) from ai_overview_field_confirmations), (select count(*) from source_anchors where id<>'legacy-anchor')", [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).unwrap()
}

#[tokio::test]
async fn confirmation_wait_rolls_back_fields_releases_database_and_rechecks_source() {
    let mut f = Fixture::new();
    let outputs: Vec<_> = inputs(&f).into_iter().map(output).collect();
    configure(&mut f, &outputs, Some(5), false);
    let caller = spawn(&f);
    if !f.entered(5).await {
        f.release();
        let error = caller.await.ok().and_then(Result::err);
        panic!("field gate not entered: {error:?}");
    }
    let store = f.store.clone();
    tokio::time::timeout(WAIT, tokio::task::spawn_blocking(move || {
        let db = store.connection().unwrap();
        assert!(db.is_autocommit());
        assert_eq!(db.execute("update ai_evidence_items set quote_exact='Changed source Ω' where id='legacy-item'", []).unwrap(), 1);
    })).await.unwrap().unwrap();
    assert_eq!(counts(&f), (0, 0, 0, 0));
    f.release();
    let error = caller.await.unwrap().err().expect("changed source");
    assert!(
        error.to_string().contains("plugin_host_source_changed"),
        "{error}"
    );
    assert_eq!(counts(&f), (0, 0, 0, 0));
    assert_eq!(f.calls(), 6);
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
async fn confirmation_detached_caller_retains_operation_and_settles_both_fields() {
    let mut f = Fixture::new();
    let outputs: Vec<_> = inputs(&f).into_iter().map(output).collect();
    configure(&mut f, &outputs, Some(5), false);
    let caller = spawn(&f);
    if !f.entered(5).await {
        f.release();
        let error = caller.await.ok().and_then(Result::err);
        panic!("field gate not entered: {error:?}");
    }
    caller.abort();
    assert!(caller.await.err().expect("aborted caller").is_cancelled());
    assert_eq!(counts(&f), (0, 0, 0, 0));
    f.release();
    tokio::time::timeout(WAIT, async {
        loop {
            if counts(&f).2 == 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(counts(&f), (2, 1, 2, 1));
    f.service.shutdown().await.unwrap();
    assert!(f.service.host.lock().await.is_none());
}

#[tokio::test]
async fn confirmation_shutdown_during_second_field_keeps_entire_batch_unwritten() {
    let mut f = Fixture::new();
    let outputs: Vec<_> = inputs(&f).into_iter().map(output).collect();
    configure(&mut f, &outputs, Some(5), false);
    let caller = spawn(&f);
    let entered = f.entered(5).await;
    let shutdown = tokio::time::timeout(WAIT, f.service.shutdown()).await;
    f.release();
    assert!(entered);
    shutdown.unwrap().unwrap();
    let error = caller.await.unwrap().err().expect("stopping");
    assert!(
        error.to_string().contains("plugin_host_stopping"),
        "{error}"
    );
    assert_eq!(counts(&f), (0, 0, 0, 0));
}

#[tokio::test]
async fn confirmation_wrong_echo_stage_and_host_exit_leave_no_fields_and_allow_explicit_retry() {
    for mode in 0..5 {
        let mut f = Fixture::new();
        let inputs = inputs(&f);
        let outputs: Vec<_> = inputs.iter().cloned().map(output).collect();
        let mut wrong = outputs.clone();
        if let CatalogOutput::PaperConfirmation { outcome } = &mut wrong[5] {
            match mode {
                0 => outcome.input.cbor_base64 = encode_payload(&inputs[3]).unwrap(),
                1 => outcome.result_cbor_base64 = encode_payload(&Ok::<
                    _,
                    research_workbench_native_sdk::candidate_contracts::CandidateValidationError,
                >(
                    PaperConfirmationDecisionV1::UndoBatchKey {},
                ))
                .unwrap(),
                _ => {}
            }
        } else {
            panic!("confirmation output");
        }
        configure(&mut f, &wrong, None, false);
        if mode >= 2 {
            let path = f.package.path().join("writing/host");
            let host = fs::read_to_string(&path).unwrap();
            let changed = match mode {
                2 => host.replace(
                    "case \"$number\" in",
                    "if [ \"$number\" = 5 ]; then exit 42; fi\ncase \"$number\" in",
                ),
                3 => host.replace(
                    "\"$instance\" \"$request\"",
                    "\"wrong-instance\" \"$request\"",
                ),
                _ => host.replace(
                    "\"$instance\" \"$request\"",
                    "\"$instance\" \"wrong-request\"",
                ),
            };
            assert_ne!(host, changed);
            fs::write(path, changed).unwrap();
            f.bind();
        }
        let error = spawn(&f).await.unwrap().err().expect("host rejection");
        assert!(
            error.to_string().contains(if mode == 2 {
                "plugin_host_exited"
            } else if mode >= 3 {
                "plugin_host_protocol_invalid"
            } else {
                "plugin_host_response_mismatch"
            }),
            "{error}"
        );
        assert_eq!(counts(&f), (0, 0, 0, 0));
        assert!(f.service.host.lock().await.is_none());
        configure(&mut f, &outputs, None, false);
        assert!(spawn(&f).await.unwrap().is_ok());
        assert_eq!(counts(&f), (2, 1, 2, 1));
        f.service.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn confirmation_missing_capability_and_changed_fixed_resource_fail_before_host() {
    let mut f = Fixture::new();
    f.bind();
    let error = spawn(&f).await.unwrap().err().expect("missing capability");
    assert!(
        error
            .to_string()
            .contains("plugin_host_capability_unavailable"),
        "{error}"
    );
    let outputs: Vec<_> = inputs(&f).into_iter().map(output).collect();
    configure(&mut f, &outputs, None, false);
    fs::write(
        f.package
            .path()
            .join("writing/data/paper-confirmation-paper-exists.v1.sql"),
        "select 'changed'",
    )
    .unwrap();
    f.bind();
    let error = spawn(&f).await.unwrap().err().expect("changed resource");
    assert!(
        error
            .to_string()
            .contains("runtime_plugin_data_digest_mismatch"),
        "{error}"
    );
    let wire = serde_json::to_value(crate::ai::error::AiCommandError::from(error)).unwrap();
    assert_eq!(
        wire["nativeFailure"]["dataError"]["resourceId"],
        "literature.paper-confirmation-paper-exists"
    );
    assert_eq!(
        wire["nativeFailure"]["dataError"]["safeCause"],
        "runtime_plugin_data_digest_mismatch"
    );
    assert!(f.service.host.lock().await.is_none());
    assert_eq!(counts(&f), (0, 0, 0, 0));
    configure(&mut f, &outputs, None, false);
    assert!(spawn(&f).await.unwrap().is_ok());
    f.service.shutdown().await.unwrap();
}
