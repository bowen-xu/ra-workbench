use super::*;
use sha2::{Digest, Sha256};

async fn refused_without_workspace_effects(h: &Harness, id: &str) {
    let before = std::fs::read(h.work.join("main.tex")).unwrap();
    let reads = h.read_count();
    let (error, read) = h.read(&format!("{id}-read")).await;
    assert!(error, "{read}");
    assert!(read.get("text").is_none(), "{read}");
    let (error, patch) = h
        .execute(h.patch(id, "Frozen manuscript", "must-not-write"))
        .await;
    assert!(error, "{patch}");
    assert_eq!(patch["targets"][0]["status"], "failed");
    assert_eq!(std::fs::read(h.work.join("main.tex")).unwrap(), before);
    assert_eq!(
        h.read_count(),
        reads,
        "rejected reads cannot gain a read receipt"
    );
    // The outer tool call may retain a failed receipt; the file backend must
    // never begin a target mutation or record an applied/unknown file write.
    let target: i64 = h.fixture.store.connection().unwrap().query_row(
        "select count(*) from agent_text_patch_calls where conversation_id=?1 and request_id=?2",
        rusqlite::params![h.record.conversation_id.to_string(), format!("{}:{id}:file:0", h.record.request_id)],
        |row| row.get(0),
    ).unwrap();
    assert_eq!(target, 0);
    assert_eq!(h.original(), "Frozen manuscript");
}

#[tokio::test]
async fn active_then_uncertain_process_intent_refuses_shared_file_reads_and_patches() {
    let h = Harness::new().await;
    assert!(!h.read("before-exec").await.0);
    let running = h
        .execute(h.call(
            "active",
            crate::agent::shell_tools::EXEC,
            json!({"command":"read value; printf '%s' \"$value\" > main.tex", "yield_ms":0}),
        ))
        .await;
    assert_eq!(running.1["state"], "running", "{:?}", running.1);
    assert!(matches!(
        h.tools.finalization_call().await,
        Err(RunError::ToolOutcomeUnknown)
    ));
    refused_without_workspace_effects(&h, "active-write").await;
    h.sessions
        .processes()
        .finish_turn(h.record.conversation_id, h.record.turn_id)
        .await
        .unwrap();
    assert!(!h.read("after-actual-drain").await.0);
    let owner = h.source.workspace(&h.record).await.unwrap();
    owner
        .host
        .with_owned_lease_mut(|lease| {
            // A dropped real intent without a process outcome is intentionally
            // unresolved. It cannot be cleared using a registry's idle state.
            let execution = lease.begin_execution().unwrap();
            drop(execution);
        })
        .await
        .unwrap();
    refused_without_workspace_effects(&h, "uncertain-write").await;
    assert!(h.tools.release_for_finalization().await.is_err());
    h.sessions.processes().shutdown().await.unwrap();
}

#[tokio::test]
async fn original_record_and_current_binding_must_both_match_shared_file_owner() {
    let h = Harness::new().await;
    assert!(!h.read("original").await.0);
    let mut wrong = h.record.clone();
    wrong.input.text.push_str(" different frozen input");
    assert!(h.source.workspace(&wrong).await.is_err());
    let registration = crate::agent::text_tools::registration_with_workspace(
        h.fixture.store.clone(),
        wrong,
        OwnedToolRegistration::new(
            vec![],
            Arc::new(crate::agent::composed_tools::RejectingToolExecutor),
        ),
        Some(h.source.clone()),
    );
    let output = registration
        .into_parts()
        .1
        .authorize_and_execute(h.call(
            "wrong-record",
            "research_read_file",
            json!({"path":h.path()}),
        ))
        .await
        .unwrap();
    assert!(output.is_error, "{}", output.content);
    // A self-consistent persisted manifest with the same C/T/lease still cannot
    // replace the one captured by this owner (including its environment/hash).
    let owner = h.source.workspace(&h.record).await.unwrap();
    let original = owner
        .host
        .inspect_owned(|lease| Ok(lease.manifest().clone()))
        .await
        .unwrap();
    let saved: (String, String) = h.fixture.store.connection().unwrap().query_row(
        "select manifest_hash,manifest_json from agent_execution_workspaces where conversation_id=?1 and request_id=?2",
        rusqlite::params![h.record.conversation_id.to_string(),h.record.request_id.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    let mut changed = original.clone();
    changed.identity.environment_id.push_str("-changed");
    let encoded = serde_json::to_string(&changed).unwrap();
    let hash = format!("{:x}", Sha256::digest(encoded.as_bytes()));
    h.fixture.store.connection().unwrap().execute(
        "update agent_execution_workspaces set manifest_hash=?3,manifest_json=?4 where conversation_id=?1 and request_id=?2",
        rusqlite::params![h.record.conversation_id.to_string(),h.record.request_id.to_string(),hash,encoded],
    ).unwrap();
    refused_without_workspace_effects(&h, "changed-manifest").await;
    h.fixture.store.connection().unwrap().execute(
        "update agent_execution_workspaces set manifest_hash=?3,manifest_json=?4 where conversation_id=?1 and request_id=?2",
        rusqlite::params![h.record.conversation_id.to_string(),h.record.request_id.to_string(),saved.0,saved.1],
    ).unwrap();
    assert!(!h.read("original-binding-restored").await.0);
    {
        let mut db = h.fixture.store.connection().unwrap();
        let tx = db.transaction().unwrap();
        execution_workspaces::revoke_in(&tx, h.record.conversation_id, h.record.request_id)
            .unwrap();
        tx.commit().unwrap();
    }
    refused_without_workspace_effects(&h, "revoked-write").await;
    h.sessions.processes().shutdown().await.unwrap();
}
