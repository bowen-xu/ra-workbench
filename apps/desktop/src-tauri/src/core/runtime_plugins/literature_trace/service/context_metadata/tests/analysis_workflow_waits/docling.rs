use super::*;
use research_workbench_native_sdk::analysis_workflow::docling::*;

fn fixture() -> (
    Fixture,
    crate::ai::generation_support::AiRun,
    Vec<crate::ai::paper_reading_contracts::PaperAiReadingContextItem>,
) {
    let f = Fixture::new();
    let (credentials, input) = actual_package::analysis_overview::seeded_task(&f);
    let mut task = fixtures::task(&input);
    let (run, items) = {
        let mut db = f.store.connection().unwrap();
        let preview = fixtures::preview(&db, &credentials, &task).unwrap();
        task.confirmed_scope_hash = Some(preview.scope_hash);
        task.cloud_consent_version = Some("ai-cloud-consent-v1".into());
        let mut identities = crate::ai::generation_support::RunCreateIdentities::new(
            &task,
            uuid::Uuid::new_v4(),
            None,
        )
        .unwrap();
        let run = crate::ai::generation_support::insert_prepared_run_guarded(
            &mut db,
            &task,
            None,
            &mut identities,
            |db| fixtures::preview(db, &credentials, &task),
            |_, _, _| {
                Err(crate::ai::error::AiError::CapabilityUnavailable(
                    "literature_library.paper_generation_source",
                ))
            },
        )
        .unwrap();
        let items = fixtures::context().context_items;
        (run, items)
    };
    (f, run, items)
}

#[tokio::test]
async fn docling_wait_releases_the_original_guard_and_rejects_changed_title() {
    use research_workbench_native_sdk::analysis_workflow::docling::*;
    let (mut f, run, items) = fixture();
    let input = WorkflowInputV1::DoclingRow {
        input_cbor_base64: encode_payload(&DoclingRowInputV1 {
            run_id: run.id.clone(),
            item: items[0].clone(),
        })
        .unwrap(),
    };
    let row = fixtures::docling_row(&run.id, &items[0]).unwrap().unwrap();
    let output = CatalogOutput::AnalysisWorkflow {
        outcome: Box::new(WorkflowWireOutcomeV1 {
            input: WorkflowWireInputV1 {
                cbor_base64: encode_payload(&input).unwrap(),
            },
            decision_cbor_base64: encode_payload(&WorkflowDecisionV1::DoclingRow {
                decision: DoclingRowDecisionV1::Ready { row },
            })
            .unwrap(),
        }),
    };
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/fixtures/native-resources-v1/literature");
    for suffix in ["existing", "title", "bundle", "anchor", "item", "context"] {
        let file = format!("analysis-docling-evidence-{suffix}.v1.sql");
        fs::copy(
            source.join(&file),
            f.package.path().join("writing/data").join(file),
        )
        .unwrap();
    }
    configure(&mut f, output.clone());
    f.configure(&[output], Some(0), false);
    let owner = Arc::clone(&f.service);
    let store = f.store.clone();
    let key = f.key.clone();
    let call = tokio::spawn(async move {
        owner
            .persist_analysis_docling_evidence(
                store,
                key,
                run,
                items,
                std::collections::BTreeMap::new(),
            )
            .await
    });
    let entered = f.entered(0).await;
    let store = f.store.clone();
    let mut writer = tokio::task::spawn_blocking(move || {
        let db = store.connection().unwrap();
        assert!(db.is_autocommit());
        db.execute(
            "update bibliographic_items set title='Changed title' where id='paper'",
            [],
        )
        .unwrap();
    });
    let available = tokio::time::timeout(WAIT, &mut writer).await;
    f.release();
    let error = tokio::time::timeout(WAIT, call)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    let released = match available {
        Ok(result) => result.is_ok(),
        Err(_) => {
            writer.await.unwrap();
            false
        }
    };
    f.service.shutdown().await.unwrap();
    assert!(entered);
    assert!(
        released,
        "Core owns the database throughout the Native wait"
    );
    let crate::ai::error::AiError::NativeWorkflow { failure, .. } = error else {
        panic!("expected changed-source diagnostic");
    };
    assert_eq!(failure.code, "plugin_host_source_changed");
    assert!(failure.instance_id.is_some());
    assert!(failure.request_id.is_some());
    let db = f.store.connection().unwrap();
    assert!(db.is_autocommit());
    assert_eq!(
        db.query_row("select count(*) from ai_evidence_bundles", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn accepted_docling_evidence_retains_ownership_after_caller_loss_and_drains_on_shutdown() {
    for close in [false, true] {
        let (mut f, run, items) = fixture();
        let outputs = items
            .iter()
            .map(|item| {
                let input = WorkflowInputV1::DoclingRow {
                    input_cbor_base64: encode_payload(&DoclingRowInputV1 {
                        run_id: run.id.clone(),
                        item: item.clone(),
                    })
                    .unwrap(),
                };
                let decision = match fixtures::docling_row(&run.id, item).unwrap() {
                    Some(row) => DoclingRowDecisionV1::Ready { row },
                    None => DoclingRowDecisionV1::Skipped {},
                };
                CatalogOutput::AnalysisWorkflow {
                    outcome: Box::new(WorkflowWireOutcomeV1 {
                        input: WorkflowWireInputV1 {
                            cbor_base64: encode_payload(&input).unwrap(),
                        },
                        decision_cbor_base64: encode_payload(&WorkflowDecisionV1::DoclingRow {
                            decision,
                        })
                        .unwrap(),
                    }),
                }
            })
            .collect::<Vec<_>>();
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tests/fixtures/native-resources-v1/literature");
        for suffix in ["existing", "title", "bundle", "anchor", "item", "context"] {
            let file = format!("analysis-docling-evidence-{suffix}.v1.sql");
            fs::copy(
                source.join(&file),
                f.package.path().join("writing/data").join(file),
            )
            .unwrap();
        }
        configure(&mut f, outputs[0].clone());
        f.configure(&outputs, Some(0), false);
        let owner = Arc::clone(&f.service);
        let store = f.store.clone();
        let key = f.key.clone();
        let caller = tokio::spawn(async move {
            owner
                .persist_analysis_docling_evidence(store, key, run, items, Default::default())
                .await
        });
        assert!(f.entered(0).await);
        caller.abort();
        assert!(caller.await.unwrap_err().is_cancelled());
        assert!(
            f.service.doi_operations.try_write().is_err(),
            "the accepted operation retains its original service lease"
        );
        if close {
            let stopped = f.service.stopped.notified();
            tokio::pin!(stopped);
            stopped.as_mut().enable();
            let owner = Arc::clone(&f.service);
            let shutdown = tokio::spawn(async move { owner.shutdown().await });
            tokio::time::timeout(WAIT, &mut stopped).await.unwrap();
            f.release();
            tokio::time::timeout(WAIT, shutdown)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert!(f.service.host.lock().await.is_none());
        } else {
            f.release();
            let drained = tokio::time::timeout(WAIT, f.service.doi_operations.write())
                .await
                .unwrap();
            drop(drained);
            f.service.shutdown().await.unwrap();
        }
        let db = f.store.connection().unwrap();
        assert!(db.is_autocommit());
        let count: i64 = db
            .query_row("select count(*) from ai_evidence_bundles", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, if close { 0 } else { 1 });
    }
}
