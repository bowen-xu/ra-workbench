//! Page publication retains the original guard and accepted-operation lifecycle.
use super::*;
use crate::ai::{generation_support as runs, task_evidence_support::TaskEvidenceFacts};

pub(super) fn fixture(kind: &str) -> (Fixture, String, Vec<CatalogOutput>) {
    let mut f = Fixture::new();
    let (credentials, input) = actual_package::analysis_overview::seeded_task(&f);
    let mut draft =
        research_literature_library_native::analysis_workflow::overview_task::task(&input);
    draft.task_kind = kind.into();
    draft.paper_template_ref = None;
    draft.overview_profile_selection_revision = None;
    draft.suggestion_candidate_kind = Some("concept_suggestion_v1".into());
    if kind == "reference_extraction" {
        draft.suggestion_candidate_kind = None;
        f.store.connection().unwrap().execute("insert into source_anchors(id,item_id,attachment_id,content_sha256,page_index,page_label,rects_json,quote_exact,quote_prefix,quote_suffix,created_at) values('reference-anchor','paper','selected',?1,0,'1','[]','Synthetic reference.','Before. ',' After.','now')",["a".repeat(64)]).unwrap();
        draft.scope_items = vec![
            research_workbench_native_sdk::generation::AiScopeItemDraft {
                item_kind: "anchor".into(),
                object_id: "reference-anchor".into(),
                parent_object_id: None,
                content_sha256: None,
                object_revision: None,
            },
        ];
    }
    let run = {
        let mut db = f.store.connection().unwrap();
        let preview = actual_package::prepared_preview( &db, &credentials, &draft).unwrap();
        draft.confirmed_scope_hash = Some(preview.scope_hash);
        draft.cloud_consent_version = Some("ai-cloud-consent-v1".into());
        let mut identities =
            runs::RunCreateIdentities::new(&draft, uuid::Uuid::new_v4(), None).unwrap();
        runs::insert_prepared_run_guarded(
            &mut db,
            &draft,
            None,
            &mut identities,
            |db| actual_package::prepared_preview( db, &credentials, &draft),
            |_, _, _| Err(crate::ai::error::AiError::TemporaryUnavailable),
        )
        .unwrap()
    };
    let inputs = if kind == "reference_extraction" {
        let db = f.store.connection().unwrap();
        let source = TaskEvidenceFacts::new(&db)
            .anchor("reference-anchor")
            .unwrap();
        let result =
            research_literature_library_native::analysis_workflow::task_preview::selection::preview(
                source.clone(),
            );
        vec![(
            WorkflowInputV1::SelectionEvidence { source },
            WorkflowDecisionV1::SelectionEvidence { result },
        )]
    } else {
        let db = f.store.connection().unwrap();
        let facts = TaskEvidenceFacts::new(&db);
        let attachment = facts.attachment("selected").unwrap().unwrap();
        let pages = facts.page_texts("selected").unwrap();
        let policy = research_literature_library_native::analysis_workflow::task_preview::pages::select_pages(&attachment,"selected",&pages).unwrap();
        vec![
            (WorkflowInputV1::PageAttachment { attachment:Some(attachment.clone()) }, WorkflowDecisionV1::Validated { rejection:None }),
            (WorkflowInputV1::PageSources { attachment_id:"selected".into(),attachment,pages }, WorkflowDecisionV1::PageSources { result:Ok(research_literature_library_native::analysis_workflow::task_preview::pages::prepare_rows(&policy)) }),
        ]
    };
    let outputs = inputs
        .into_iter()
        .map(|(input, decision)| CatalogOutput::AnalysisWorkflow {
            outcome: Box::new(WorkflowWireOutcomeV1 {
                input: WorkflowWireInputV1 {
                    cbor_base64: encode_payload(&input).unwrap(),
                },
                decision_cbor_base64: encode_payload(&decision).unwrap(),
            }),
        })
        .collect::<Vec<_>>();
    configure(&mut f, outputs[0].clone());
    (f, run.id, outputs)
}

#[tokio::test]
async fn page_evidence_wait_releases_guard_and_rejects_changed_page_before_publication() {
    let (mut f, run, outputs) = fixture("paper_suggestions");
    f.configure(&outputs, Some(1), false);
    let owner = f.service.clone();
    let store = f.store.clone();
    let key = f.key.clone();
    let call = tokio::spawn(async move { owner.persist_page_task_evidence(store, key, run).await });
    assert!(f.entered(1).await);
    let store = f.store.clone();
    let mut writer = tokio::task::spawn_blocking(move || {
        let db = store.connection().unwrap();
        assert!(db.is_autocommit());
        db.execute(
            "update attachment_page_texts set page_text='changed' where attachment_id='selected'",
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
    assert!(
        available.is_ok(),
        "Core DB guard must be released during Native wait"
    );
    let crate::ai::error::AiError::NativeWorkflow { failure, .. } = error else {
        panic!("expected the original Native source-change diagnostic");
    };
    assert_eq!(failure.code, "plugin_host_source_changed");
    assert!(failure.instance_id.is_some());
    assert!(failure.request_id.is_some());
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row("select count(*) from ai_evidence_bundles", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
async fn accepted_page_evidence_keeps_lease_after_caller_loss_and_drains_on_shutdown() {
    for close in [false, true] {
        let (mut f, run, outputs) = fixture("paper_suggestions");
        f.configure(&outputs, Some(1), false);
        let owner = f.service.clone();
        let store = f.store.clone();
        let key = f.key.clone();
        let call =
            tokio::spawn(async move { owner.persist_page_task_evidence(store, key, run).await });
        assert!(f.entered(1).await);
        call.abort();
        assert!(call.await.unwrap_err().is_cancelled());
        assert!(f.service.doi_operations.try_write().is_err());
        if close {
            let stopped = f.service.stopped.notified();
            tokio::pin!(stopped);
            stopped.as_mut().enable();
            let owner = f.service.clone();
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
        assert_eq!(
            db.query_row("select count(*) from ai_evidence_bundles", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            if close { 0 } else { 1 }
        );
    }
}
