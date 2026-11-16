use crate::{
    ai::{error::AiError, structured_generation_support::StructuredGenerationFacts},
    storage::AppStore,
};
use research_workbench_native_sdk::analysis_workflow::submission::CompletionTemplateV1;
use rusqlite::Connection;
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

fn receipt(
    store: &AppStore,
    resource: &str,
    template: Option<&str>,
    conversation: Uuid,
    request: Uuid,
) -> Result<(Value, CompletionTemplateV1), AiError> {
    let db = store.connection()?;
    StructuredGenerationFacts::new(&db, store).completion_receipt(
        resource,
        template,
        conversation,
        request,
    )
}

#[test]
fn generation_completion_requires_the_exact_submission_resource_receipt() {
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
    assert!(receipt(&store, &target.id, None, conversation, Uuid::new_v4()).is_err());
    assert!(receipt(&store, &target.id, None, Uuid::new_v4(), request).is_err());
    let (response, _) = receipt(&store, &target.id, None, conversation, request).unwrap();
    assert_eq!(response, content);
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
        receipt(&store, &target.id, None, conversation, request).is_err(),
        "an earlier successful write does not hide a later invalid draft"
    );
}

#[test]
fn portrait_receipt_observes_current_saved_template_and_rejects_its_failed_draft() {
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
    let (_, observed) = receipt(
        &store,
        &target.id,
        Some(&template.id),
        conversation,
        request,
    )
    .unwrap();
    let CompletionTemplateV1::Required {
        content: Some(observed),
    } = observed
    else {
        panic!("saved template observation")
    };
    assert_eq!(observed["fields"][0]["id"], "result");

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
        receipt(
            &store,
            &target.id,
            Some(&template.id),
            conversation,
            request
        )
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
        receipt(
            &store,
            &target.id,
            Some(&template.id),
            conversation,
            request
        )
        .is_err(),
        "a previous valid proposal cannot hide the current invalid draft"
    );
}
