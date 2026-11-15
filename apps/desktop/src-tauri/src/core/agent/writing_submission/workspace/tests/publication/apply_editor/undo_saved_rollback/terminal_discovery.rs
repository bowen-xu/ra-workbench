use super::*;
use crate::agent::writing_application_commands::undo::discovery;
use serde_json::{Value, json};

async fn page(f: &Fixture) -> Result<Value, crate::agent::writing_workspace_commands::Error> {
    discovery::list(
        &f.store,
        discovery::ListRequest {
            manuscript_id: f.manuscript,
            after: None,
            limit: 1,
        },
    )
    .await
    .map(|page| serde_json::to_value(page).unwrap())
}
fn persist(f: &Fixture, checkpoint: &Value) {
    f.store.connection().unwrap().execute(
        "insert into writing_document_checkpoints(manuscript_id,relative_path,revision,checkpoint_json,updated_at)
         values(?1,'main.tex',1,?2,'now') on conflict(manuscript_id,relative_path) do update set checkpoint_json=excluded.checkpoint_json",
        rusqlite::params![f.manuscript.to_string(),checkpoint.to_string()],
    ).unwrap();
}
async fn terminal(f: &Fixture, rolled_back: bool) -> (ApplicationScope, Uuid, Value) {
    let (service, original, undo, editor, hash) = partial(f).await;
    let (operation, actual, text) = if rolled_back {
        service
            .prepare_proposal_undo_rollback(
                original,
                undo,
                current(f, &editor, &hash, "Frozen manuscript"),
            )
            .await
            .unwrap();
        let restore = Uuid::new_v4();
        service
            .prepare_undo_editor_restoration(
                original,
                undo,
                restore,
                observation(&editor, "Frozen manuscript"),
            )
            .await
            .unwrap();
        let actual = EditorVersion {
            revision: Uuid::new_v4(),
            ..editor
        };
        service
            .confirm_undo_editor_restoration(
                original,
                undo,
                restore,
                observation(&actual, "proposal"),
            )
            .await
            .unwrap();
        service
            .rollback_disk_proposal_undo(original, undo, current(f, &actual, &hash, "proposal"))
            .await
            .unwrap();
        (restore, actual, "proposal")
    } else {
        service
            .resume_proposal_undo(
                original,
                undo,
                current(f, &editor, &hash, "Frozen manuscript"),
            )
            .await
            .unwrap();
        service
            .apply_disk_undo_file(original, undo, "z.tex".into())
            .await
            .unwrap();
        (undo, editor, "Frozen manuscript")
    };
    // Only durable host evidence is seeded. The terminal native operation above
    // uses the actual coordinators; discovery never writes these bytes or records.
    let request: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "select request_id from agent_change_sets where id=?1",
            [original.proposal.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    let source = |id: Uuid, text: &str| {
        json!({"operationId":id,"conversationId":f.conversation,
        "requestId":request,"resultSha256":format!("{:x}",Sha256::digest(text.as_bytes()))})
    };
    let mut operations = vec![
        source(original.operation, "proposal"),
        source(undo, "Frozen manuscript"),
    ];
    if rolled_back {
        operations.push(source(operation, text));
    }
    let checkpoint = json!({"schemaVersion":1,"manuscriptId":f.manuscript,"relativePath":"main.tex",
        "version":4,"content":format!("{text}\nlater input"),"reviewBaseline":text,"savedSha256":hash,"savedVersion":3,
        "operations":operations,"saveIntent":null,"editorReceipt":{"operationId":operation,"sessionId":actual.session_id,
            "revision":actual.revision,"text":text,"stateVersion":4}});
    (original, undo, checkpoint)
}

#[tokio::test]
async fn discovers_terminal_editor_save_wait_without_replaying_or_changing_the_checkpoint() {
    for rolled_back in [false, true] {
        let f = fixture_with_objects();
        let (original, undo, mut checkpoint) = terminal(&f, rolled_back).await;
        assert_eq!(page(&f).await.unwrap()["items"], json!([]));
        persist(&f, &checkpoint);
        f.store
            .connection()
            .unwrap()
            .execute(
                "update agent_conversations set archived=1 where id=?1",
                [f.conversation.to_string()],
            )
            .unwrap();
        let found = page(&f).await.unwrap();
        let database = f.root.path().join("terminal-undo.sqlite");
        f.store
            .connection()
            .unwrap()
            .execute("vacuum into ?1", [database.to_str().unwrap()])
            .unwrap();
        let reopened = crate::storage::AppStore::for_test(
            rusqlite::Connection::open(database).unwrap(),
            f.store.content_directory().to_path_buf(),
        );
        let restored = discovery::list(
            &reopened,
            discovery::ListRequest {
                manuscript_id: f.manuscript,
                after: None,
                limit: 1,
            },
        )
        .await
        .unwrap();
        assert_eq!(serde_json::to_value(restored).unwrap(), found);
        assert_eq!(found["items"][0]["undo"]["undoId"], json!(undo));
        assert_eq!(
            found["items"][0]["undo"]["originalScope"]["operationId"],
            json!(original.operation)
        );
        assert_eq!(
            found["items"][0]["undo"]["result"]["status"],
            if rolled_back { "rolledBack" } else { "applied" }
        );
        let json: String = f
            .store
            .connection()
            .unwrap()
            .query_row(
                "select checkpoint_json from writing_document_checkpoints",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(serde_json::from_str::<Value>(&json).unwrap(), checkpoint);
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
        checkpoint["savedVersion"] = checkpoint["version"].clone();
        persist(&f, &checkpoint);
        assert_eq!(page(&f).await.unwrap()["items"], json!([]));
        // A lost save completion remains pending until its version is confirmed.
        checkpoint["savedVersion"] = json!(3);
        checkpoint["saveIntent"] = json!({"operationId":Uuid::new_v4(),"version":4,
            "expectedSha256":checkpoint["savedSha256"],"targetSha256":format!("{:x}",Sha256::digest(checkpoint["content"].as_str().unwrap().as_bytes()))});
        persist(&f, &checkpoint);
        assert_eq!(
            page(&f).await.unwrap()["items"][0]["undo"]["undoId"],
            json!(undo)
        );
        checkpoint["savedVersion"] = json!(4);
        checkpoint["version"] = json!(5);
        checkpoint["saveIntent"]["version"] = json!(5);
        persist(&f, &checkpoint);
        // Ordinary edits after the saved operation have their own save lifecycle.
        assert_eq!(page(&f).await.unwrap()["items"], json!([]));
    }
}

#[tokio::test]
async fn terminal_save_discovery_rejects_forged_receipts_and_provenance() {
    for rolled_back in [false, true] {
        let f = fixture_with_objects();
        let (_, _, valid) = terminal(&f, rolled_back).await;
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
        let last = corrupt["operations"]
            .as_array_mut()
            .unwrap()
            .last_mut()
            .unwrap();
        last["resultSha256"] = json!(format!("{:x}", Sha256::digest(b"forged text")));
        persist(&f, &corrupt);
        assert!(page(&f).await.is_err());
        // Current text alone is never proof for a legacy terminal journal.
        let mut legacy = valid;
        legacy.as_object_mut().unwrap().remove("editorReceipt");
        persist(&f, &legacy);
        assert_eq!(page(&f).await.unwrap()["items"], json!([]));
    }
}
