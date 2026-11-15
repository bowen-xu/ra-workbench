//! Produce isolated native UI acceptance data through the production runner.
//! Only model responses are scripted; no delivery, proposal or checkpoint is seeded.
#[path = "native_acceptance/deletion.rs"]
mod deletion;
#[path = "native_acceptance/expectations.rs"]
mod expectations;
use super::*;
use crate::{agent::writing_delivery_commands as delivery, ai::JsonCredentialStore};
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};
use std::os::unix::fs::PermissionsExt;

const MAIN: &str = "\\documentclass{article}\n\\begin{document}\n\\section{Abstract}\nOriginal abstract.\n\n\\section{Method}\nUnchanged method context.\nUnchanged evidence context.\nUnchanged synthetic context.\n\n\\section{Conclusion}\nOriginal conclusion.\n\\end{document}\n";
const INTRODUCTION: &str = "\\section{Introduction}\nOriginal introduction.\n";

#[path = "native_acceptance/transport.rs"]
mod transport;
use transport::{FixtureCase, PatchTransport};

#[tokio::test]
#[ignore = "Creates retained synthetic data for native multi-file delivery acceptance; run with --exact --ignored --nocapture"]
async fn produce_native_multifile_delivery_fixture() {
    produce_fixture(FixtureCase::Standard, true).await;
}

#[tokio::test]
#[ignore = "Creates isolated special-file native acceptance data; requires an explicit fixture case"]
async fn produce_native_special_file_delivery_fixture() {
    let value = std::env::var("RESEARCH_ASSISTANT_NATIVE_SPECIAL_FILE_CASE")
        .expect("an explicit special-file case is required");
    produce_fixture(
        FixtureCase::special(&value).expect("unsupported special-file case"),
        true,
    )
    .await;
}

#[tokio::test]
async fn shell_changes_remain_reviewable_before_the_owned_turn_finishes() {
    produce_fixture(FixtureCase::MixedFiles, false).await;
}

async fn produce_fixture(case: FixtureCase, retain: bool) {
    let root = tempfile::Builder::new()
        .prefix("research-assistant-writing-diff-agent-")
        .tempdir_in("/private/tmp")
        .unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let data = root.path().to_path_buf();
    let mut db = Connection::open(data.join("research-assistant.sqlite3")).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let conversation = Uuid::new_v4();
    let manuscript = Uuid::new_v4();
    let project = Uuid::new_v4();
    crate::agent::repository::create_conversation(
        &mut db,
        ConversationId::from_uuid(conversation),
        "Native Agent acceptance",
    )
    .unwrap();
    db.execute("insert into research_projects(id,title,summary,created_at) values(?1,'Native Agent acceptance','Synthetic only','2026-10-07T00:00:00Z')", [project.to_string()]).unwrap();
    db.execute("insert into manuscripts(id,project_id,title,root_path,status,created_at,updated_at) values(?1,?2,'Native Agent acceptance','main.tex','active','2026-10-07T00:00:00Z','2026-10-07T00:00:00Z')", params![manuscript.to_string(),project.to_string()]).unwrap();
    db.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values(?1,'manuscript',?2)", params![conversation.to_string(),manuscript.to_string()]).unwrap();
    db.execute("insert into llm_provider_configs(provider_id,enabled,base_url,timeout_ms,updated_at) values('deepseek',1,'',60000,'2026-10-07T00:00:00Z') on conflict(provider_id) do update set enabled=1,base_url=''", []).unwrap();
    db.execute("insert into llm_model_routes(route_kind,route_id,model_kind,provider_id,model_id,updated_at) values('default','text','text','deepseek','fixture-model','2026-10-07T00:00:00Z') on conflict(route_kind,route_id) do update set provider_id='deepseek',model_id='fixture-model'", []).unwrap();
    let database = crate::data_interface::Database::open(db, &data.join("data-log")).unwrap();
    let store = AppStore::for_test_database(database, data.join("objects"));
    std::fs::create_dir_all(store.content_directory()).unwrap();
    let source = store
        .manuscript_directory()
        .join(manuscript.to_string())
        .join("source");
    std::fs::create_dir_all(source.join("sections")).unwrap();
    let hash = |text: &str| format!("{:x}", Sha256::digest(text.as_bytes()));
    for (path, text) in [
        ("main.tex", MAIN),
        ("sections/introduction.tex", INTRODUCTION),
    ] {
        std::fs::write(source.join(path), text).unwrap();
        store.connection().unwrap().execute("insert into manuscript_files(manuscript_id,relative_path,file_kind,byte_size,sha256,modified_at) values(?1,?2,'tex',?3,?4,'2026-10-07T00:00:00Z')", params![manuscript.to_string(),path,text.len(),hash(text)]).unwrap();
    }
    case.seed_binary(&source, &store, manuscript);
    let credentials =
        JsonCredentialStore::from_explicit_development_path(data.join("credentials.json"));
    credentials.set("deepseek", "synthetic-fixture").unwrap();
    std::fs::write(
        data.join("empty-credentials.json"),
        r#"{"schemaVersion":1,"providers":{},"mcp":{}}"#,
    )
    .unwrap();
    std::fs::set_permissions(
        data.join("empty-credentials.json"),
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let models = AgentModelResolver::from_app_store(&store, credentials);
    let service = WritingSubmissionService::from_app_store(&store);
    let prepared = service
        .prepare(
            &models,
            WritingSubmissionRequest {
                resources: vec![],
                user_input: vec![],
                conversation_id: conversation,
                request_id: Uuid::new_v4(),
                text:
                    "Revise the abstract, conclusion and introduction in this synthetic manuscript."
                        .into(),
                model: None,
                session_config: None,
                turn_config: None,
                retrieval: None,
                context: crate::agent::writing_submission::WritingContextRequest {
                    manuscript_id: manuscript,
                    files: vec![crate::agent::WritingContextFile {
                        relative_path: "main.tex".into(),
                        expected_disk_sha256: hash(MAIN),
                        buffer: None,
                        selection: None,
                    }],
                },
            },
        )
        .await
        .unwrap();
    let record = prepared.record.clone();
    // This retained regression fixture restores a frozen pre-migration model
    // sequence. New text-patch turns are covered by tests/text_patch.rs.
    store.connection().unwrap().execute("insert into agent_text_tool_contracts(conversation_id,request_id,version) values(?1,?2,1)",params![conversation.to_string(),record.request_id.to_string()]).unwrap();
    let receipt = WritingSubmissionReceipt {
        conversation_id: conversation,
        request_id: record.request_id,
        manuscript_id: manuscript,
    };
    let workspace = service
        .prepare_workspace(
            receipt,
            record.context_snapshot_id,
            crate::agent::writing_workspace::WritingWorkspaceRequest {
                manuscript_id: manuscript,
                buffers: vec![],
            },
        )
        .await
        .unwrap();
    drop(service.approve_workspace(workspace).await.unwrap());
    let sessions = Arc::new(AgentSessionService::new(Arc::new(
        SqliteConversationStore::new(store.shared_agent_connection(), "native-acceptance".into())
            .unwrap(),
    )));
    let mut execution_profile = profile(record.input.model.clone());
    if case != FixtureCase::Standard {
        // Patch, approved shell, bounded real process polls, review and finish.
        execution_profile.limits.model_calls = Some(10);
        execution_profile.limits.tool_calls = Some(9);
        // Six frozen file inventories plus the real shell result remain in history.
        execution_profile.max_input_bytes = 64 * 1024;
    }
    service
        .validate_start(
            &prepared,
            &confirmation(&record, manuscript),
            &execution_profile,
        )
        .await
        .unwrap();
    let transport = Arc::new(PatchTransport::new(case));
    let admission = service
        .start_transport_with_manuscript(
            sessions,
            record.clone(),
            execution_profile,
            transport.clone(),
            manuscript,
        )
        .await
        .unwrap();
    let SubmissionAdmission::Started(owned) = admission else {
        panic!("expected native start")
    };
    let completion = owned.task.wait().await;
    if completion != TaskCompletion::Completed {
        let db = store.connection().unwrap();
        let mut statement = db.prepare(
            "select json_extract(payload_json,'$.payload.type'),json_extract(payload_json,'$.payload.error') from agent_items where conversation_id=?1 order by ordinal",
        ).unwrap();
        let diagnostics: Vec<(Option<String>, Option<String>)> = statement
            .query_map([conversation.to_string()], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect();
        panic!(
            "synthetic fixture runner did not complete: {completion:?}; calls: {}; event types/errors: {diagnostics:?}",
            transport.calls.load(Ordering::SeqCst)
        );
    }
    assert!((3..=10).contains(&transport.calls.load(Ordering::SeqCst)));
    if case == FixtureCase::Standard {
        assert_eq!(transport.calls.load(Ordering::SeqCst), 3);
    }
    // Production delivery discovery must work without any selected chat or seeded events.
    let page = delivery::list(
        &store,
        delivery::Request {
            manuscript_id: manuscript,
            after: None,
            limit: 64,
        },
    )
    .await
    .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].turn_id, record.turn_id);
    assert_eq!(page.items[0].scope.request_id, record.request_id);
    assert_eq!(
        std::fs::read_to_string(source.join("main.tex")).unwrap(),
        MAIN
    );
    assert_eq!(
        std::fs::read_to_string(source.join("sections/introduction.tex")).unwrap(),
        INTRODUCTION
    );
    let review = service
        .review_workspace(receipt, record.context_snapshot_id)
        .await
        .unwrap();
    assert_eq!(review.changes.len(), case.change_count());
    expectations::assert_inventory(case, &review.changes);
    case.assert_original_binary(&source);
    assert_eq!(
        store
            .connection()
            .unwrap()
            .query_row(
                "select count(*) from agent_proposal_applications",
                [],
                |row| { row.get::<_, i64>(0) }
            )
            .unwrap(),
        0
    );
    assert_eq!(
        store
            .connection()
            .unwrap()
            .query_row(
                "select count(*) from writing_document_checkpoints",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    let manifest = serde_json::json!({
        "fixtureCase":case.name(), "reviewInventory":review.changes,
        "dataDirectory":data, "manuscriptId":manuscript, "conversationId":conversation,
        "requestId":record.request_id, "turnId":record.turn_id,
        "mainBefore":MAIN,
        "mainAfter":MAIN.replace("Original abstract.", "Agent revised abstract.").replace("Original conclusion.", "Agent revised conclusion."),
        "introductionBefore":INTRODUCTION,
        "introductionAfter":INTRODUCTION.replace("Original introduction.", "Agent revised introduction.")
    });
    std::fs::write(
        data.join("native-acceptance.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    drop(service);
    drop(store);
    if retain {
        let retained = root.keep();
        println!("Native acceptance data: {}", retained.display());
    }
}
