use super::*;
use crate::agent::{
    writing_proposal_commands as commands,
    writing_workspace_commands::{Confirmation, Scope},
};

#[tokio::test]
async fn proposal_boundary_round_trips_exact_review_and_rejects_stale_or_unscoped_requests() {
    let f = fixture_with_objects();
    let review = changed(&f).await;
    let b = &review.binding;
    let binding = Confirmation {
        scope: Scope {
            conversation_id: b.conversation_id,
            request_id: b.request_id,
            manuscript_id: b.manuscript_id,
            context_snapshot_id: b.context_snapshot_id,
        },
        turn_id: b.turn_id,
        lease_id: b.lease_id,
        manifest_hash: b.manifest_hash.clone(),
    };
    let changes: Vec<_> = review.changes.iter().map(|c| serde_json::json!({"path":c.path,"before":c.before.clone().map(crate::agent::writing_review_commands::Version::from),"after":c.after.clone().map(crate::agent::writing_review_commands::Version::from)})).collect();
    let request = serde_json::json!({"binding":binding,"changes":changes});
    let mut stale = request.clone();
    stale["changes"][0]["after"]["sha256"] = serde_json::json!("a".repeat(64));
    assert!(
        commands::publish(&f.store, serde_json::from_value(stale).unwrap())
            .await
            .is_err()
    );
    let mut injected = request.clone();
    injected["changes"][0]["source"] = serde_json::json!({"revision":"fake"});
    assert!(serde_json::from_value::<commands::PublishRequest>(injected).is_err());
    let saved = serde_json::to_value(
        commands::publish(&f.store, serde_json::from_value(request.clone()).unwrap())
            .await
            .unwrap(),
    )
    .unwrap();
    let latest_scope: Scope = serde_json::from_value(request["binding"]["scope"].clone()).unwrap();
    assert_eq!(
        serde_json::to_value(commands::latest(&f.store, latest_scope).await.unwrap()).unwrap(),
        saved
    );
    assert_eq!(saved["schemaVersion"], 1);
    assert_eq!(saved["status"], "pending");
    assert_eq!(saved["review"]["binding"], request["binding"]);
    assert!(saved["review"]["changes"][0].get("source").is_none());
    let scope = commands::ProposalRequest {
        conversation_id: f.conversation,
        manuscript_id: f.manuscript,
        proposal_id: Uuid::parse_str(saved["proposalId"].as_str().unwrap()).unwrap(),
    };
    assert_eq!(
        serde_json::to_value(commands::read(&f.store, scope.clone()).await.unwrap()).unwrap(),
        saved
    );
    let content = serde_json::to_value(
        commands::read_file(
            &f.store,
            commands::ProposalFileRequest {
                proposal: scope.clone(),
                path: "main.tex".into(),
            },
        )
        .await
        .unwrap(),
    )
    .unwrap();
    assert_eq!(content["proposalId"], saved["proposalId"]);
    assert_eq!(content["file"]["binding"], saved["review"]["binding"]);
    assert_eq!(
        content["file"]["after"],
        serde_json::json!({"kind":"text","text":"Revised manuscript"})
    );
    let mut wrong = scope.clone();
    wrong.conversation_id = Uuid::new_v4();
    assert!(commands::read(&f.store, wrong).await.is_err());
    assert!(
        commands::read_file(
            &f.store,
            commands::ProposalFileRequest {
                proposal: scope.clone(),
                path: "../main.tex".into()
            }
        )
        .await
        .is_err()
    );
    let mut nil = scope.clone();
    nil.proposal_id = Uuid::nil();
    assert!(commands::read(&f.store, nil).await.is_err());
    assert_eq!(
        serde_json::to_value(
            commands::publish(&f.store, serde_json::from_value(request).unwrap())
                .await
                .unwrap()
        )
        .unwrap()["proposalId"],
        saved["proposalId"]
    );
    let rejected =
        serde_json::to_value(commands::reject(&f.store, scope.clone()).await.unwrap()).unwrap();
    assert_eq!(rejected["status"], "rejected");
    assert_eq!(rejected["review"], saved["review"]);
    assert_eq!(
        serde_json::to_value(commands::read(&f.store, scope).await.unwrap()).unwrap(),
        rejected
    );
}
