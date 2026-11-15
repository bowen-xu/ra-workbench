use super::super::super::apply_disk::deleted_chat::{delete_chat, full_fixture};
use super::*;
use crate::agent::writing_application_commands::discovery;

async fn page(
    f: &crate::agent::writing_submission::tests::Fixture,
) -> Result<Value, crate::agent::writing_workspace_commands::Error> {
    discovery::list(
        &f.store,
        discovery::ListRequest {
            manuscript_id: f.manuscript,
            after: None,
            limit: 64,
        },
    )
    .await
    .map(|page| serde_json::to_value(page).unwrap())
}
async fn terminal(
    f: &crate::agent::writing_submission::tests::Fixture,
    rolled_back: bool,
) -> (Scenario, Value) {
    let s = if rolled_back {
        partial(f).await
    } else {
        applying(f).await
    };
    let mut checkpoint = s.checkpoint.clone();
    if rolled_back {
        let receipt = &checkpoint["editorReceipt"];
        let owner = EditorVersion {
            session_id: Uuid::parse_str(receipt["sessionId"].as_str().unwrap()).unwrap(),
            revision: Uuid::parse_str(receipt["revision"].as_str().unwrap()).unwrap(),
        };
        let restore = Uuid::new_v4();
        s.service
            .prepare_editor_restoration(s.scope, restore, observe(&owner, AFTER))
            .await
            .unwrap();
        let restored = EditorVersion {
            revision: Uuid::new_v4(),
            ..owner
        };
        s.service
            .confirm_editor_restoration(s.scope, restore, observe(&restored, BEFORE))
            .await
            .unwrap();
        s.service
            .rollback_proposal_remainder(s.scope)
            .await
            .unwrap();
        checkpoint["content"] = json!(BEFORE);
        checkpoint["version"] = json!(3);
        checkpoint["editorReceipt"] = json!({"operationId":restore,"sessionId":restored.session_id,
            "revision":restored.revision,"text":BEFORE,"stateVersion":3});
        let mut source = checkpoint["operations"][0].clone();
        source["operationId"] = json!(restore);
        source["resultSha256"] = json!(format!("{:x}", Sha256::digest(BEFORE.as_bytes())));
        checkpoint["operations"]
            .as_array_mut()
            .unwrap()
            .push(source);
    } else {
        s.service
            .apply_disk_proposal_file(s.scope, "z.tex".into())
            .await
            .unwrap();
    }
    persist(f, &checkpoint);
    (s, checkpoint)
}

#[tokio::test]
async fn running_delivery_requires_the_original_editor_receipt_to_be_saved() {
    use crate::agent::{writing_delivery_commands as delivery, writing_workspace_commands::Scope};
    use research_agent_core::{
        conversation::ConversationId,
        protocol::{AgentEvent, EventPayload, ModelIdentity, TaskKind},
    };
    let f = full_fixture();
    let (s, mut checkpoint) = terminal(&f, false).await;
    let binding = crate::agent::repository::change_sets::read_in(
        &f.store.connection().unwrap(),
        f.conversation,
        f.manuscript,
        s.scope.proposal,
    )
    .unwrap()
    .binding;
    let started = AgentEvent::new(
        ConversationId::from_uuid(f.conversation),
        1,
        0,
        binding.request_id,
        EventPayload::TurnStarted {
            task_kind: TaskKind::Regular,
            turn_id: binding.turn_id,
            context_snapshot_id: binding.context_snapshot_id,
            model: ModelIdentity {
                provider_id: "fixture".into(),
                model_id: "fixture".into(),
                revision: None,
            },
        },
    )
    .unwrap();
    f.store.connection().unwrap().execute("insert into agent_items(conversation_id,ordinal,schema_version,payload_json) values(?1,0,1,?2)", rusqlite::params![f.conversation.to_string(), serde_json::to_string(&started).unwrap()]).unwrap();
    let request = delivery::Acknowledge {
        scope: Scope {
            conversation_id: f.conversation,
            manuscript_id: f.manuscript,
            request_id: binding.request_id,
            context_snapshot_id: binding.context_snapshot_id,
        },
        turn_id: binding.turn_id,
    };
    let delivery_id = Uuid::new_v4();
    assert!(
        delivery::verify_running(&f.store, request.clone(), delivery_id)
            .await
            .is_err()
    );
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row(
                "select count(*) from writing_agent_delivery_proofs",
                [],
                |row| row.get::<_, u64>(0)
            )
            .unwrap(),
        0
    );
    checkpoint["savedVersion"] = json!(1);
    persist(&f, &checkpoint);
    f.store.connection().unwrap().execute_batch("create trigger fail_delivery_proof before insert on writing_agent_delivery_proofs begin select raise(abort,'synthetic failure'); end;").unwrap();
    assert!(
        delivery::verify_running(&f.store, request.clone(), delivery_id)
            .await
            .is_err()
    );
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row(
                "select count(*) from writing_agent_delivery_proofs",
                [],
                |row| row.get::<_, u64>(0)
            )
            .unwrap(),
        0
    );
    f.store
        .connection()
        .unwrap()
        .execute_batch("drop trigger fail_delivery_proof;")
        .unwrap();
    assert_eq!(
        delivery::verify_running(&f.store, request.clone(), delivery_id)
            .await
            .unwrap(),
        delivery::RunningProof::Applied
    );
    let evidence: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "select proof_json from writing_agent_delivery_proofs where delivery_id=?1",
            [delivery_id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    let parsed: Value = serde_json::from_str(&evidence).unwrap();
    assert_eq!(parsed["operationId"], json!(s.scope.operation));
    assert_eq!(
        parsed["editors"][0]["resultSha256"],
        json!(format!("{:x}", Sha256::digest(AFTER.as_bytes())))
    );
    assert!(!evidence.contains(AFTER));
    assert!(
        f.store
            .connection()
            .unwrap()
            .execute(
                "update writing_agent_delivery_proofs set proof_json='{}'",
                []
            )
            .is_err()
    );
    assert!(
        f.store
            .connection()
            .unwrap()
            .execute("delete from writing_agent_delivery_proofs", [])
            .is_err()
    );
    let database = f.root.path().join("running-delivery-proof.sqlite");
    f.store
        .connection()
        .unwrap()
        .execute("vacuum into ?1", [database.to_str().unwrap()])
        .unwrap();
    let reopened = Connection::open(database).unwrap();
    assert_eq!(
        reopened
            .query_row(
                "select proof_json from writing_agent_delivery_proofs where delivery_id=?1",
                [delivery_id.to_string()],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        evidence
    );
    checkpoint["editorReceipt"] = Value::Null;
    persist(&f, &checkpoint);
    assert!(
        delivery::verify_running(&f.store, request, delivery_id)
            .await
            .is_err()
    );
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row(
                "select proof_json from writing_agent_delivery_proofs where delivery_id=?1",
                [delivery_id.to_string()],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        evidence
    );
}

#[tokio::test]
async fn original_terminal_discovery_waits_for_exact_saved_receipt_across_deleted_chat_and_reopen()
{
    for rolled_back in [false, true] {
        let f = full_fixture();
        let (s, mut checkpoint) = terminal(&f, rolled_back).await;
        let found = page(&f).await.unwrap();
        assert_eq!(
            found["items"][0]["application"]["scope"]["operationId"],
            json!(s.scope.operation)
        );
        assert_eq!(
            found["items"][0]["application"]["status"],
            if rolled_back { "rolledBack" } else { "applied" }
        );
        delete_chat(&f).await;
        assert_eq!(page(&f).await.unwrap(), found);
        let database = f.root.path().join("terminal-application.sqlite");
        f.store
            .connection()
            .unwrap()
            .execute("vacuum into ?1", [database.to_str().unwrap()])
            .unwrap();
        let reopened = AppStore::for_test(
            Connection::open(database).unwrap(),
            f.store.content_directory().to_path_buf(),
        );
        let restored = discovery::list(
            &reopened,
            discovery::ListRequest {
                manuscript_id: f.manuscript,
                after: None,
                limit: 64,
            },
        )
        .await
        .unwrap();
        assert_eq!(serde_json::to_value(restored).unwrap(), found);
        let saved: String = f
            .store
            .connection()
            .unwrap()
            .query_row(
                "select checkpoint_json from writing_document_checkpoints",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(serde_json::from_str::<Value>(&saved).unwrap(), checkpoint);
        assert_eq!(
            std::fs::read(
                f.store
                    .manuscript_directory()
                    .join(f.manuscript.to_string())
                    .join("source/main.tex")
            )
            .unwrap(),
            b"Frozen manuscript"
        );
        let receipt_version = checkpoint["editorReceipt"]["stateVersion"]
            .as_u64()
            .unwrap();
        checkpoint["savedVersion"] = json!(receipt_version);
        // Later ordinary unsaved input must not reopen an already saved application.
        checkpoint["version"] = json!(receipt_version + 1);
        checkpoint["content"] = json!(format!(
            "{}later input",
            checkpoint["content"].as_str().unwrap()
        ));
        persist(&f, &checkpoint);
        assert_eq!(page(&f).await.unwrap()["items"], json!([]));
        checkpoint["savedVersion"] = json!(receipt_version - 1);
        checkpoint["saveIntent"] = json!({"operationId":Uuid::new_v4(),"version":receipt_version + 1,
            "expectedSha256":f.hash,"targetSha256":format!("{:x}",Sha256::digest(checkpoint["content"].as_str().unwrap().as_bytes()))});
        persist(&f, &checkpoint);
        assert_eq!(page(&f).await.unwrap(), found);
        checkpoint["savedVersion"] = json!(receipt_version);
        persist(&f, &checkpoint);
        assert_eq!(page(&f).await.unwrap()["items"], json!([]));
    }
}

#[tokio::test]
async fn original_terminal_discovery_rejects_forged_identity_and_does_not_infer_receipts_from_text()
{
    for rolled_back in [false, true] {
        let f = fixture_with_objects();
        let (_, valid) = terminal(&f, rolled_back).await;
        for field in ["sessionId", "revision"] {
            let mut corrupt = valid.clone();
            corrupt["editorReceipt"][field] = json!(Uuid::new_v4());
            persist(&f, &corrupt);
            assert!(page(&f).await.is_err());
        }
        for field in ["requestId", "conversationId"] {
            let mut corrupt = valid.clone();
            corrupt["operations"][0][field] = json!(Uuid::new_v4());
            persist(&f, &corrupt);
            assert!(page(&f).await.is_err());
        }
        let mut corrupt = valid.clone();
        corrupt["editorReceipt"]["text"] = json!("forged text");
        persist(&f, &corrupt);
        assert!(page(&f).await.is_err());
        let mut foreign = valid.clone();
        let other = Uuid::new_v4();
        foreign["editorReceipt"]["operationId"] = json!(other);
        foreign["operations"]
            .as_array_mut()
            .unwrap()
            .last_mut()
            .unwrap()["operationId"] = json!(other);
        persist(&f, &foreign);
        assert_eq!(page(&f).await.unwrap()["items"], json!([]));
        let mut legacy = valid;
        legacy.as_object_mut().unwrap().remove("editorReceipt");
        persist(&f, &legacy);
        assert_eq!(page(&f).await.unwrap()["items"], json!([]));
    }
}
