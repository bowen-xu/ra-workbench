//! Original apply compensation retains the real receipt when its editor owner leaves.
use super::*;
use serde_json::{Value, json};

#[path = "checkpoint/scenario.rs"]
mod scenario;
#[path = "checkpoint/terminal_discovery.rs"]
mod terminal_discovery;
use crate::agent::repository::proposal_applications::ApplicationTarget;
use scenario::*;

#[tokio::test]
async fn original_compensation_admits_rebuilt_owner_only_with_real_checkpoint_receipt() {
    let f = fixture_with_objects();
    let s = partial(&f).await;
    let original = s
        .service
        .read_proposal_application(
            s.scope.conversation,
            s.scope.manuscript,
            s.scope.proposal,
            s.scope.operation,
        )
        .await
        .unwrap();
    let restore = Uuid::new_v4();
    let proof = json!({"path":"main.tex","revision":1,"stateVersion":2});
    let boundary::Response::Edit {
        expected,
        text,
        restore_id,
        ..
    } = boundary::execute(&f.store, prepare_request(&s, restore, proof.clone(), AFTER))
        .await
        .unwrap()
    else {
        panic!("fresh compensation dispatch");
    };
    assert_eq!(restore_id, restore);
    assert_eq!(expected.session_id, s.owner.session_id);
    assert_eq!(expected.revision, s.owner.revision);
    assert_eq!(text, BEFORE);
    let current = s
        .service
        .read_proposal_application(
            s.scope.conversation,
            s.scope.manuscript,
            s.scope.proposal,
            s.scope.operation,
        )
        .await
        .unwrap();
    assert_eq!(current.files, original.files);
    // A lost prepare reply retains intent; retry cannot issue another edit.
    assert!(matches!(
        boundary::execute(&f.store, prepare_request(&s, restore, proof, AFTER))
            .await
            .unwrap(),
        boundary::Response::State { .. }
    ));
    let actual = EditorVersion {
        revision: Uuid::new_v4(),
        ..s.owner.clone()
    };
    f.store.connection().unwrap().execute_batch("create trigger fail_original_restore_ack before update of phase on agent_proposal_application_files when new.phase='restored' begin select raise(abort,'fixture failed confirmation'); end").unwrap();
    assert!(
        s.service
            .confirm_editor_restoration(s.scope, restore, observe(&actual, BEFORE))
            .await
            .is_err()
    );
    assert!(
        s.service
            .read_editor_restoration(s.scope, "main.tex".into())
            .await
            .unwrap()
            .1
            .unwrap()
            .result_revision
            .is_none()
    );
    // A later live edit does not turn a lost confirmation into another dispatch.
    assert!(matches!(
        boundary::execute(
            &f.store,
            prepare_request(
                &s,
                restore,
                json!({"path":"main.tex","revision":1,"stateVersion":2}),
                "later input"
            )
        )
        .await
        .unwrap(),
        boundary::Response::State { .. }
    ));
    f.store
        .connection()
        .unwrap()
        .execute_batch("drop trigger fail_original_restore_ack")
        .unwrap();
    let done = s
        .service
        .confirm_editor_restoration(s.scope, restore, observe(&actual, BEFORE))
        .await
        .unwrap();
    assert_eq!(done.status, ApplicationStatus::RecoveryRequired);
    assert_eq!(
        done.files
            .iter()
            .find(|f| f.path == "main.tex")
            .unwrap()
            .target,
        original
            .files
            .iter()
            .find(|f| f.path == "main.tex")
            .unwrap()
            .target
    );
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
}

fn rollback_request(
    s: &Scenario,
    action: &str,
    text: &str,
    state_version: u64,
) -> crate::agent::writing_application_commands::Request {
    serde_json::from_value(json!({"action":action,"scope":{"conversationId":s.scope.conversation,"manuscriptId":s.scope.manuscript,"proposalId":s.scope.proposal,"operationId":s.scope.operation},
        "buffers":[{"path":"main.tex","sessionId":s.owner.session_id,"revision":s.owner.revision,"text":text,"expectedDiskSha256":s.checkpoint["savedSha256"]}],
        "checkpointVersions":[{"path":"main.tex","revision":1,"stateVersion":state_version}]})).unwrap()
}
#[tokio::test]
async fn original_compensation_rechecks_all_targets_after_another_owner_rebuild() {
    use crate::agent::writing_application_commands as commands;
    let f = fixture_with_objects();
    let mut s = partial(&f).await;
    commands::execute(&f.store, rollback_request(&s, "prepareRollback", AFTER, 2))
        .await
        .unwrap();
    let original = s
        .service
        .read_proposal_application(
            s.scope.conversation,
            s.scope.manuscript,
            s.scope.proposal,
            s.scope.operation,
        )
        .await
        .unwrap();
    let restore = Uuid::new_v4();
    boundary::execute(
        &f.store,
        prepare_request(
            &s,
            restore,
            json!({"path":"main.tex","revision":1,"stateVersion":2}),
            AFTER,
        ),
    )
    .await
    .unwrap();
    let actual = EditorVersion {
        revision: Uuid::new_v4(),
        ..s.owner.clone()
    };
    s.service
        .confirm_editor_restoration(s.scope, restore, observe(&actual, BEFORE))
        .await
        .unwrap();
    s.checkpoint["version"] = json!(3);
    s.checkpoint["content"] = json!(BEFORE);
    s.checkpoint["reviewBaseline"] = json!(BEFORE);
    s.checkpoint["editorReceipt"] = json!({"operationId":restore,"sessionId":actual.session_id,"revision":actual.revision,"text":BEFORE,"stateVersion":3});
    let request = s.checkpoint["operations"][0]["requestId"].clone();
    s.checkpoint["operations"].as_array_mut().unwrap().push(json!({"operationId":restore,"conversationId":s.scope.conversation,"requestId":request,"resultSha256":format!("{:x}",Sha256::digest(BEFORE.as_bytes()))}));
    persist(&f, &s.checkpoint);
    s.owner = EditorVersion {
        session_id: Uuid::new_v4(),
        revision: Uuid::new_v4(),
    };
    let source = f
        .store
        .manuscript_directory()
        .join(f.manuscript.to_string())
        .join("source/a.tex");
    std::fs::write(&source, "later user edit").unwrap();
    assert!(
        commands::execute(
            &f.store,
            rollback_request(&s, "rollbackRemainder", BEFORE, 3)
        )
        .await
        .is_err()
    );
    assert_eq!(std::fs::read_to_string(&source).unwrap(), "later user edit");
    std::fs::write(&source, "added disk").unwrap();
    let done = serde_json::to_value(
        commands::execute(
            &f.store,
            rollback_request(&s, "rollbackRemainder", BEFORE, 3),
        )
        .await
        .unwrap(),
    )
    .unwrap();
    assert_eq!(done["status"], "rolledBack");
    assert!(!source.exists());
    let final_state = s
        .service
        .read_proposal_application(
            s.scope.conversation,
            s.scope.manuscript,
            s.scope.proposal,
            s.scope.operation,
        )
        .await
        .unwrap();
    assert_eq!(
        final_state
            .files
            .iter()
            .find(|f| f.path == "main.tex")
            .unwrap()
            .target,
        original
            .files
            .iter()
            .find(|f| f.path == "main.tex")
            .unwrap()
            .target
    );
    assert_eq!(
        s.service
            .read_editor_restoration(s.scope, "main.tex".into())
            .await
            .unwrap()
            .1
            .unwrap()
            .result_revision,
        Some(actual.revision)
    );
    // Terminal retry is a read of the completed operation, not a new write.
    std::fs::write(&source, "new independent file").unwrap();
    assert_eq!(
        serde_json::to_value(
            commands::execute(
                &f.store,
                rollback_request(&s, "rollbackRemainder", BEFORE, 3)
            )
            .await
            .unwrap()
        )
        .unwrap(),
        done
    );
    assert_eq!(
        std::fs::read_to_string(source).unwrap(),
        "new independent file"
    );
}
#[tokio::test]
async fn original_compensation_refuses_forged_or_stale_evidence_before_any_intent() {
    use crate::agent::writing_application_commands as commands;
    let f = fixture_with_objects();
    let mut s = partial(&f).await;
    let original = s
        .service
        .read_proposal_application(
            s.scope.conversation,
            s.scope.manuscript,
            s.scope.proposal,
            s.scope.operation,
        )
        .await
        .unwrap();
    let reliable = s.checkpoint.clone();
    for fault in [
        "source",
        "receipt",
        "missingReceipt",
        "text",
        "version",
        "intent",
        "disk",
    ] {
        s.checkpoint = reliable.clone();
        let mut text = AFTER;
        match fault {
            "source" => s.checkpoint["operations"][0]["requestId"] = json!(Uuid::new_v4()),
            "receipt" => s.checkpoint["editorReceipt"]["sessionId"] = json!(Uuid::new_v4()),
            "missingReceipt" => s.checkpoint["editorReceipt"] = Value::Null,
            "text" => {
                s.checkpoint["content"] = json!("later input");
                text = "later input";
            }
            "version" => s.checkpoint["version"] = json!(3),
            "intent" => {
                s.checkpoint["saveIntent"] = json!({"operationId":Uuid::new_v4(),"version":2,"expectedSha256":f.hash,"targetSha256":f.hash})
            }
            "disk" => {
                std::fs::write(
                    f.store
                        .manuscript_directory()
                        .join(f.manuscript.to_string())
                        .join("source/a.tex"),
                    "later user edit",
                )
                .unwrap();
            }
            _ => unreachable!(),
        }
        persist(&f, &s.checkpoint);
        assert!(
            commands::execute(&f.store, rollback_request(&s, "prepareRollback", text, 2))
                .await
                .is_err(),
            "{fault}"
        );
        assert_eq!(
            s.service
                .read_proposal_application(
                    s.scope.conversation,
                    s.scope.manuscript,
                    s.scope.proposal,
                    s.scope.operation
                )
                .await
                .unwrap(),
            original
        );
        assert!(
            s.service
                .read_editor_restoration(s.scope, "main.tex".into())
                .await
                .unwrap()
                .1
                .is_none()
        );
        if fault != "disk" {
            assert!(
                boundary::execute(
                    &f.store,
                    prepare_request(
                        &s,
                        Uuid::new_v4(),
                        json!({"path":"main.tex","revision":1,"stateVersion":2}),
                        text
                    )
                )
                .await
                .is_err(),
                "{fault}"
            );
        }
    }
    // Database admission also rejects evidence that loses the original source.
    s.checkpoint = reliable;
    s.checkpoint["operations"] = json!([]);
    persist(&f, &s.checkpoint);
    let db = f.store.connection().unwrap();
    assert!(db.execute("insert into agent_editor_restorations(application_id,relative_path,restore_id,editor_session,expected_revision) values(?1,'main.tex',?2,?3,?4)",rusqlite::params![s.scope.operation.to_string(),Uuid::new_v4().to_string(),s.owner.session_id.to_string(),s.owner.revision.to_string()]).is_err());
}

#[tokio::test]
async fn original_compensation_requires_proof_for_a_new_owner_and_revision_for_the_same_owner() {
    let f = fixture_with_objects();
    let mut s = partial(&f).await;
    let before = s
        .service
        .read_proposal_application(
            s.scope.conversation,
            s.scope.manuscript,
            s.scope.proposal,
            s.scope.operation,
        )
        .await
        .unwrap();
    let wire = prepare_request(&s, Uuid::new_v4(), Value::Null, AFTER);
    assert!(boundary::execute(&f.store, wire).await.is_err());
    let mut request = rollback_request(&s, "prepareRollback", AFTER, 2);
    let crate::agent::writing_application_commands::Request::PrepareRollback {
        checkpoint_versions,
        ..
    } = &mut request
    else {
        panic!("rollback admission");
    };
    checkpoint_versions.clear();
    assert!(
        crate::agent::writing_application_commands::execute(&f.store, request)
            .await
            .is_err()
    );
    let file = before
        .files
        .iter()
        .find(|file| file.path == "main.tex")
        .unwrap();
    let ApplicationTarget::Editor { expected, .. } = &file.target else {
        panic!("editor target");
    };
    s.owner.session_id = expected.session_id;
    // Reliable content does not authorize a changed revision on the original owner.
    assert!(
        boundary::execute(
            &f.store,
            prepare_request(
                &s,
                Uuid::new_v4(),
                json!({"path":"main.tex","revision":1,"stateVersion":2}),
                AFTER
            )
        )
        .await
        .is_err()
    );
    assert!(
        s.service
            .read_editor_restoration(s.scope, "main.tex".into())
            .await
            .unwrap()
            .1
            .is_none()
    );
    assert_eq!(
        s.service
            .read_proposal_application(
                s.scope.conversation,
                s.scope.manuscript,
                s.scope.proposal,
                s.scope.operation
            )
            .await
            .unwrap(),
        before
    );
}
