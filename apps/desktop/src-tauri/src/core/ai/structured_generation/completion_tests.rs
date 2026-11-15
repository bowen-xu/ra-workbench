use crate::{
    ai::{generation_hooks::PreparedPaperWorkflow, generation_support::VisionGenerationResponse},
    storage::AppStore,
};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use uuid::Uuid;

fn database() -> (Connection, Uuid) {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    db.execute("insert into bibliographic_items(id,item_type,title,created_at,updated_at) values('paper','journalArticle','Paper','now','now')",[]).unwrap();
    let conversation = Uuid::new_v4();
    db.execute(
        "insert into agent_conversations(id,title) values(?1,'Internal task')",
        [conversation.to_string()],
    )
    .unwrap();
    (db, conversation)
}

fn reply() -> VisionGenerationResponse {
    VisionGenerationResponse {
        content: "The model claims it saved the object".into(),
        input_tokens: None,
        output_tokens: None,
        context_window_tokens: None,
        context_window_source: None,
        raw_response: json!({}),
    }
}

async fn workflow(
    store: &AppStore,
    conversation: Uuid,
    request: Uuid,
    kind: &str,
) -> Box<dyn PreparedPaperWorkflow> {
    use crate::ai::paper_reading_contracts::PaperAiReadingContextItem;
    use research_agent_core::submission::{ContextSourceKind, ContextText, SubmissionInput};
    let input = {
        let mut db = store.connection().unwrap();
        let sha = "a".repeat(64);
        db.execute("insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at) values(?1,'aa/paper.pdf','application/pdf',10,'now')", [&sha]).unwrap();
        db.execute("insert into attachments(id,item_id,content_sha256,display_name,created_at) values('att','paper',?1,'paper.pdf','now')", [&sha]).unwrap();
        db.execute("insert into docling_parse_runs(id,item_id,attachment_id,content_sha256,service_base_url,docling_status,parse_status,json_content,raw_response,created_at,finished_at) values('parse','paper','att',?1,'http://127.0.0.1:5001','success','success','{}','{}','now','now')", [&sha]).unwrap();
        let task_kind = if kind == "portrait" {
            "paper_profile_classification"
        } else {
            "paper_overview"
        };
        db.execute("insert into ai_runs(id,task_kind,status,provider_id,model_id,pipeline_version,prompt_version,output_schema_version,user_goal,scope_hash,input_item_count,max_input_tokens,max_output_tokens,timeout_ms,created_at,updated_at) values('00000000-0000-4000-8000-000000000101',?1,'generating','fixture-generation','fixture-v1','pipeline','prompt',1,'goal',?2,1,1000,1000,1000,'now','now')", params![task_kind, "f".repeat(64)]).unwrap();
        db.execute("insert into agent_legacy_ai_run_bindings(ai_run_id,conversation_id,request_id,task_kind) values('00000000-0000-4000-8000-000000000101',?1,?2,?3)", params![conversation.to_string(),request.to_string(),task_kind]).unwrap();
        let run = crate::ai::run_repository::get_run(&db, "00000000-0000-4000-8000-000000000101")
            .unwrap();
        crate::ai::generation_support::persist_test_docling_evidence(
            &mut db,
            &run,
            &[PaperAiReadingContextItem {
                id: "P1-T1".into(),
                paper_id: "paper".into(),
                attachment_id: "att".into(),
                content_sha256: sha,
                parse_run_id: "parse".into(),
                raw_ref: "#/texts/0".into(),
                kind: "text".into(),
                label: None,
                page_index: Some(0),
                regions: vec![],
                charspan: None,
                source_text_hash: Some(crate::ai::generation_support::hash_text("Frozen fact.")),
                text: Some("Frozen fact.".into()),
                table_json: None,
            }],
            &Default::default(),
        )
        .unwrap();
        let context = if kind == "portrait" {
            let template =
                crate::structured_resources::find_by_paper(&db, "paper", "readingTemplate")
                    .unwrap();
            vec![ContextText {
                source_id: "fixture-template".into(),
                kind: ContextSourceKind::Paper,
                resource_id: template.id,
                revision: "frozen-template".into(),
                anchor: Some(json!({"contextAccess":"readWrite"}).to_string()),
                text: "Synthetic frozen template grant".into(),
            }]
        } else {
            vec![]
        };
        SubmissionInput {
            text: String::new(),
            user_input: vec![],
            skill_instructions: vec![],
            context,
            instructions: vec![],
            instruction_configs: vec![],
            skills: vec![],
            model: research_agent_core::protocol::ModelIdentity {
                provider_id: "fixture".into(),
                model_id: "fixture".into(),
                revision: None,
            },
            reasoning_effort: None,
            service_tier: None,
            remote_compaction_v2: None,
        }
    };
    assert!(
        store.native_note_trace().is_some(),
        "RESEARCH_ASSISTANT_TEST_LITERATURE_PACKAGE must select the compiled Native fixture"
    );
    crate::ai::paper_reading_shared_transport::prepare_workflow_for_test(
        store,
        conversation,
        request,
        Some(&input),
    )
    .await
    .unwrap()
    .unwrap()
}

#[tokio::test]
async fn generation_completion_requires_the_exact_submission_resource_receipt() {
    let (mut db, conversation) = database();
    let target =
        crate::structured_resources::find_by_paper(&db, "paper", "structuredReading").unwrap();
    let request = Uuid::new_v4();
    let content = json!({"kind":"paper_overview_unit_v1","schemaVersion":1,"profileId":"generic_overview_v1","fields":[
        {"field":"research_question","status":"filled","statements":[{"id":"question-1","text":"待核实的问题。","epistemicRole":"uncertain","evidenceItemIds":[]}]},
        {"field":"approach","status":"not_found","statements":[]},{"field":"key_result","status":"not_found","statements":[]},{"field":"conclusion","status":"not_found","statements":[]},{"field":"limitation","status":"not_found","statements":[]},{"field":"open_question","status":"not_found","statements":[]}
    ]});
    let saved = crate::hbs_storage::test_sources::write(
        &mut db,
        crate::structured_resources::WriteDraft {
            resource_id: target.id.clone(),
            expected_revision: 0,
            content: content.to_string(),
            request_id: "write-operation".into(),
            conversation_id: Some(conversation.to_string()),
            expected_draft_revision: None,
            source_snapshot_id: None,
            submission_id: Some(request.to_string()),
            editor_kind: "agent".into(),
        },
    )
    .unwrap();
    assert_eq!(saved.status, "applied");
    let root = tempfile::tempdir().unwrap();
    let store = AppStore::for_test(db, root.path().join("objects"));
    let workflow = workflow(&store, conversation, request, "structuredReading").await;
    assert!(
        workflow
            .complete(&store, conversation, Uuid::new_v4(), reply())
            .await
            .is_err()
    );
    assert!(
        workflow
            .complete(&store, Uuid::new_v4(), request, reply())
            .await
            .is_err()
    );
    let response = workflow
        .complete(&store, conversation, request, reply())
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&response.content).unwrap(),
        content
    );
    {
        let mut db = store.connection().unwrap();
        let result = crate::hbs_storage::test_sources::write(
            &mut db,
            crate::structured_resources::WriteDraft {
                resource_id: target.id.clone(),
                expected_revision: saved.resource.revision,
                content: "{invalid follow-up".into(),
                request_id: "later-invalid".into(),
                conversation_id: Some(conversation.to_string()),
                expected_draft_revision: None,
                source_snapshot_id: None,
                submission_id: Some(request.to_string()),
                editor_kind: "agent".into(),
            },
        )
        .unwrap();
        assert_eq!(result.status, "invalid");
    }
    assert!(
        workflow
            .complete(&store, conversation, request, reply())
            .await
            .is_err(),
        "an earlier successful write does not hide a later invalid draft"
    );
    store
        .native_note_trace()
        .unwrap()
        .service
        .shutdown()
        .await
        .unwrap();
}

#[tokio::test]
async fn portrait_completion_requires_current_matching_saved_template_and_no_failed_draft() {
    use crate::structured_resources::{WriteDraft, find_by_paper};
    let (mut db, conversation) = database();
    let request = Uuid::new_v4();
    let target = find_by_paper(&db, "paper", "portrait").unwrap();
    let template = find_by_paper(&db, "paper", "readingTemplate").unwrap();
    let fields =
        json!([{"id":"question","title":"问题","question":"回应什么问题？","claimIds":[]}]);
    let portrait = json!({"kind":"paper_portrait_v1","schemaVersion":1,"coverage":"仅提供的摘录。","claims":[
        {"id":"genre","dimension":"genre","label":"不确定","description":"证据不足。","certainty":"uncertain","evidence":[]},
        {"id":"mode","dimension":"mode","label":"不确定","description":"证据不足。","certainty":"uncertain","evidence":[]},
        {"id":"paradigm","dimension":"paradigm","label":"不确定","description":"证据不足。","certainty":"uncertain","evidence":[]}
    ],"template":fields});
    let input = |id: &str, body: Value, operation: &str| WriteDraft {
        resource_id: id.into(),
        expected_revision: 0,
        content: body.to_string(),
        request_id: operation.into(),
        conversation_id: Some(conversation.to_string()),
        expected_draft_revision: None,
        source_snapshot_id: None,
        submission_id: Some(request.to_string()),
        editor_kind: "agent".into(),
    };
    assert_eq!(
        crate::hbs_storage::test_sources::write(&mut db, input(&target.id, portrait, "portrait"))
            .unwrap()
            .status,
        "applied"
    );
    let wrong = json!({"kind":"reading_template_v1","schemaVersion":1,"name":"本文模板","fields":[{"id":"result","title":"结果","question":"得出了什么结果？"}]});
    assert_eq!(
        crate::hbs_storage::test_sources::write(
            &mut db,
            input(&template.id, wrong, "wrong-proposal")
        )
        .unwrap()
        .status,
        "applied"
    );
    let root = tempfile::tempdir().unwrap();
    let store = AppStore::for_test(db, root.path().join("objects"));
    let workflow = workflow(&store, conversation, request, "portrait").await;
    assert!(
        workflow
            .complete(&store, conversation, request, reply())
            .await
            .is_err()
    );
    {
        let mut db = store.connection().unwrap();
        let correct = json!({"kind":"reading_template_v1","schemaVersion":1,"name":"本文模板","fields":[{"id":"question","title":"问题","question":"回应什么问题？"}]});
        let mut correction = input(&template.id, correct, "correct-template");
        correction.expected_revision = 1;
        assert_eq!(
            crate::hbs_storage::test_sources::write(&mut db, correction)
                .unwrap()
                .status,
            "applied"
        );
    }
    assert!(
        workflow
            .complete(&store, conversation, request, reply())
            .await
            .is_ok()
    );
    {
        let mut db = store.connection().unwrap();
        let mut invalid = input(&template.id, json!({}), "invalid-proposal");
        invalid.content = "{invalid".into();
        invalid.expected_revision = 2;
        assert_eq!(
            crate::hbs_storage::test_sources::write(&mut db, invalid)
                .unwrap()
                .status,
            "invalid"
        );
    }
    assert!(
        workflow
            .complete(&store, conversation, request, reply())
            .await
            .is_err(),
        "a previous valid proposal cannot hide the current invalid draft"
    );
    store
        .native_note_trace()
        .unwrap()
        .service
        .shutdown()
        .await
        .unwrap();
}
