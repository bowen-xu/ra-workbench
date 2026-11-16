//! Wrong pure outcomes retain the existing host's cleanup and request identity.
use super::*;
use research_workbench_native_sdk::{
    analysis_workflow::wire::*, catalog::CatalogOutput, paper_context_wire::encode_payload,
};
use std::fs;

fn request() -> WorkflowInputV1 {
    WorkflowInputV1::ValidateStart {
        revision: 1,
        has_template: false,
    }
}
fn response() -> CatalogOutput {
    CatalogOutput::AnalysisWorkflow {
        outcome: Box::new(WorkflowWireOutcomeV1 {
            input: WorkflowWireInputV1 {
                cbor_base64: encode_payload(&request()).unwrap(),
            },
            decision_cbor_base64: encode_payload(&WorkflowDecisionV1::Validated {
                rejection: None,
            })
            .unwrap(),
        }),
    }
}
fn configure(f: &mut Fixture, output: CatalogOutput) {
    let path = f.package.path().join("writing/manifest.json");
    let mut manifest: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    manifest["provides"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":CAPABILITY_ID,"version":CAPABILITY_VERSION}));
    fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    f.configure(&[output], None, false);
}

mod acceptance;
mod citation;
#[path = "analysis_workflow_waits/docling.rs"]
mod docling;
pub(super) mod fixtures;
mod matrix;
mod pages;
mod revision;
mod selection;

#[tokio::test]
async fn template_wait_releases_the_original_guard_and_rejects_changed_selection() {
    use research_workbench_native_sdk::analysis_workflow::template::*;
    let mut f = Fixture::new();
    let reference = actual_package::analysis_template::adopted(&f.store);
    let (selection_revision, binding) =
        crate::ai::template_facts::TemplateFacts::new(&f.store.connection().unwrap())
            .reading_template("paper")
            .unwrap();
    let input = WorkflowInputV1::Template {
        input: Box::new(TemplateInputV1::ResourceBinding {
            reference: reference.clone(),
            selection_revision,
            binding,
        }),
    };
    let output = CatalogOutput::AnalysisWorkflow {
        outcome: Box::new(WorkflowWireOutcomeV1 {
            input: WorkflowWireInputV1 {
                cbor_base64: encode_payload(&input).unwrap(),
            },
            decision_cbor_base64: encode_payload(&WorkflowDecisionV1::Template {
                decision: TemplateDecisionV1::Verified {},
            })
            .unwrap(),
        }),
    };
    // Add the same finite capability, then retain this response at its first wait.
    configure(&mut f, output.clone());
    f.configure(&[output], Some(0), false);
    let owner = Arc::clone(&f.service);
    let store = f.store.clone();
    let key = f.key.clone();
    let call = tokio::spawn(async move {
        owner
            .resolve_analysis_template(store, &key, "paper", &reference)
            .await
    });
    let entered = f.entered(0).await;
    let store = f.store.clone();
    let mut writer = tokio::task::spawn_blocking(move || {
        let db = store.connection().unwrap();
        assert!(db.is_autocommit());
        db.execute("update structured_resources set revision=revision+1 where paper_id='paper' and kind='structuredReading'", []).unwrap();
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
        "the database owner must be available throughout the Native wait"
    );
    let crate::ai::error::AiError::NativeWorkflow { failure, .. } = error else {
        panic!("expected changed-source diagnostic");
    };
    assert_eq!(failure.code, "plugin_host_source_changed");
    assert!(failure.instance_id.is_some());
    assert!(failure.request_id.is_some());
    assert!(f.store.connection().unwrap().is_autocommit());
}

#[tokio::test]
async fn workflow_capability_rejection_happens_before_host_admission() {
    let mut f = Fixture::new();
    f.bind();
    let failure = f
        .service
        .analysis_workflow_decision(&f.key, request())
        .await
        .unwrap_err();
    assert_eq!(failure.code, "plugin_host_capability_unavailable");
    assert_eq!(failure.package_key.as_deref(), Some(f.key.as_str()));
    assert!(f.service.host.lock().await.is_none());
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
async fn workflow_rejects_wrong_echo_output_kind_and_leaf_with_real_request_identity() {
    for mode in 0..4 {
        let mut f = Fixture::new();
        let mut output = response();
        if let CatalogOutput::AnalysisWorkflow { outcome } = &mut output {
            match mode {
                0 => {
                    outcome.input.cbor_base64 = encode_payload(&WorkflowInputV1::ValidateStart {
                        revision: 2,
                        has_template: false,
                    })
                    .unwrap()
                }
                1 => {
                    outcome.decision_cbor_base64 = encode_payload(&WorkflowDecisionV1::Status {
                        status: "overview_running".into(),
                    })
                    .unwrap()
                }
                2 => outcome.decision_cbor_base64 = "invalid base64".into(),
                _ => output = f.response(),
            }
        }
        configure(&mut f, output);
        let failure = f
            .service
            .analysis_workflow_decision(&f.key, request())
            .await
            .unwrap_err();
        assert_eq!(failure.code, "plugin_host_response_mismatch");
        assert!(failure.instance_id.is_some());
        assert!(failure.request_id.is_some());
        assert!(f.service.host.lock().await.is_none());
        f.service.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn workflow_rejects_malformed_prompt_leaf_before_reusing_the_host() {
    use research_workbench_native_sdk::analysis_workflow::prompt::TaskPromptInputV1;
    let input = TaskPromptInputV1 {
        task_template: "{{ task }}\n{{ contract }}\n{{ preferences }}".into(),
        reading_contract: "{{ schema }}".into(),
        sources: vec![],
        kind: "structuredReading".into(),
        profile: None,
        preferences: "".into(),
    };
    let request = WorkflowInputV1::RenderPrompt {
        input_cbor_base64: encode_payload(&input).unwrap(),
    };
    let output = CatalogOutput::AnalysisWorkflow {
        outcome: Box::new(WorkflowWireOutcomeV1 {
            input: WorkflowWireInputV1 {
                cbor_base64: encode_payload(&request).unwrap(),
            },
            decision_cbor_base64: encode_payload(&WorkflowDecisionV1::Prompt {
                result_cbor_base64: "invalid payload".into(),
            })
            .unwrap(),
        }),
    };
    let mut f = Fixture::new();
    configure(&mut f, output);
    let failure = f
        .service
        .render_analysis_task_prompt(&f.key, input)
        .await
        .unwrap_err();
    assert_eq!(failure.code, "plugin_host_response_mismatch");
    assert!(failure.instance_id.is_some());
    assert!(failure.request_id.is_some());
    assert!(f.service.host.lock().await.is_none());
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
async fn workflow_rejects_rebound_submission_scope_before_reusing_the_host() {
    use research_workbench_native_sdk::analysis_workflow::submission::SubmissionScopeV1;
    for mode in 0..3 {
        let request = WorkflowInputV1::SubmissionScope {
            task_kind: "paper_overview".into(),
            paper_ids: vec!["paper".into()],
            attachment_ids: vec!["selected".into()],
        };
        let mut scope = SubmissionScopeV1 {
            paper_id: "paper".into(),
            kind: "structuredReading".into(),
            attachment_ids: vec!["selected".into()],
            system_text: "synthetic contract".into(),
        };
        match mode {
            0 => scope.paper_id = "different".into(),
            1 => scope.attachment_ids = vec!["different".into()],
            _ => scope.kind = "arbitrary".into(),
        }
        let output = CatalogOutput::AnalysisWorkflow {
            outcome: Box::new(WorkflowWireOutcomeV1 {
                input: WorkflowWireInputV1 {
                    cbor_base64: encode_payload(&request).unwrap(),
                },
                decision_cbor_base64: encode_payload(&WorkflowDecisionV1::SubmissionScope {
                    result: Ok(scope),
                })
                .unwrap(),
            }),
        };
        let mut f = Fixture::new();
        configure(&mut f, output);
        let failure = f
            .service
            .analysis_workflow_decision(&f.key, request)
            .await
            .unwrap_err();
        assert_eq!(failure.code, "plugin_host_response_mismatch");
        assert!(failure.instance_id.is_some());
        assert!(failure.request_id.is_some());
        assert!(f.service.host.lock().await.is_none());
        f.service.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn creation_releases_the_original_transaction_and_rejects_changed_admission_facts() {
    use research_workbench_native_sdk::analysis_workflow::PaperAnalysisWorkflowDraft;
    let mut f = Fixture::new();
    let provider: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "select provider_id from ai_provider_settings order by provider_id limit 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    f.store.connection().unwrap().execute("insert into paper_context_snapshots(id,paper_id,context_hash,context_schema_version,created_at) values('admission-snapshot','paper',?1,1,'old')",["d".repeat(64)]).unwrap();
    let draft = PaperAnalysisWorkflowDraft {
        paper_id: "paper".into(),
        context_snapshot_id: "admission-snapshot".into(),
        provider_id: provider.clone(),
        model_id: "model".into(),
        reasoning_effort: None,
        prompt_set_version: "v3".into(),
    };
    let response = |input: WorkflowInputV1| CatalogOutput::AnalysisWorkflow {
        outcome: Box::new(WorkflowWireOutcomeV1 {
            input: WorkflowWireInputV1 {
                cbor_base64: encode_payload(&input).unwrap(),
            },
            decision_cbor_base64: encode_payload(&WorkflowDecisionV1::Validated {
                rejection: None,
            })
            .unwrap(),
        }),
    };
    let directory = f.package.path().join("writing");
    let manifest = directory.join("manifest.json");
    let mut value: Value = serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    value["provides"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":CAPABILITY_ID,"version":CAPABILITY_VERSION}));
    fs::write(manifest, serde_json::to_vec(&value).unwrap()).unwrap();
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/fixtures/native-resources-v1/literature");
    for file in [
        "analysis-history-latest.v1.sql",
        "analysis-workflow-generation-active.v1.sql",
        "analysis-workflow-generation-pointer.v1.sql",
        "analysis-workflow-generation-title.v1.sql",
        "analysis-workflow-generation-set-pointer.v1.sql",
        "analysis-workflow-supersede.v1.sql",
        "analysis-workflow-create.v1.sql",
        "analysis-workflow-create-turn.v1.sql",
    ] {
        fs::copy(source.join(file), directory.join("data").join(file)).unwrap();
    }
    f.configure(
        &[
            response(WorkflowInputV1::ValidateDraft {
                draft: draft.clone(),
            }),
            response(WorkflowInputV1::ValidateGeneration {
                portrait_active: false,
                analysis_active: false,
            }),
        ],
        Some(1),
        false,
    );
    let owner = Arc::clone(&f.service);
    let store = f.store.clone();
    let key = f.key.clone();
    let call = tokio::spawn(async move { owner.create_analysis_workflow(store, key, draft).await });
    let entered = f.entered(1).await;
    let store = f.store.clone();
    let mut writer = tokio::task::spawn_blocking(move || {
        let db = store.connection().unwrap();
        assert!(db.is_autocommit());
        db.execute("insert into paper_analysis_workflows(id,paper_id,context_snapshot_id,status,provider_id,model_id,prompt_set_version,created_at,updated_at) values('concurrent-workflow','paper','admission-snapshot','preparing',?1,'model','v3','now','now')",[provider]).unwrap();
    });
    let available = tokio::time::timeout(WAIT, &mut writer).await;
    f.release();
    let result = tokio::time::timeout(WAIT, call).await.unwrap().unwrap();
    let released = match available {
        Ok(result) => result.is_ok(),
        Err(_) => {
            writer.await.unwrap();
            false
        }
    };
    let failure = result.unwrap_err();
    f.service.shutdown().await.unwrap();
    assert!(entered);
    assert!(
        released,
        "the original guard must be released before the Native wait"
    );
    assert_eq!(failure["code"], "plugin_host_source_changed");
    assert_eq!(failure["nativeFailure"]["outcome"], "rolledBack");
    let db = f.store.connection().unwrap();
    let count:(i64,i64,i64)=db.query_row("select (select count(*) from paper_analysis_workflows),(select count(*) from agent_paper_analysis_bindings),(select count(*) from paper_generation_conversations)",[],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
    assert_eq!(count, (1, 0, 0));
    assert!(db.is_autocommit());
}
