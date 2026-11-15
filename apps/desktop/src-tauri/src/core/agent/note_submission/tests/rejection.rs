use super::*;
use crate::agent::{
    note_rejection::{self, Request},
    note_review::Scope,
};
use serde_json::json;

#[tokio::test]
async fn rejection_is_durable_idempotent_and_does_not_modify_the_note_or_journal() {
    let f = fixture(true);
    let (scope, proposal) = super::review::seeded(&f, true).await;
    assert!(
        note_rejection::status(&f.store, scope)
            .await
            .unwrap()
            .is_none()
    );
    let before = counts(&f.store);
    // A rejected old suggestion must not require the current body to match.
    f.store
        .connection()
        .unwrap()
        .execute("update note_pages set revision=revision+1", [])
        .unwrap();
    let request_id = Uuid::new_v4();
    let receipt = note_rejection::reject(&f.store, Request { scope, request_id })
        .await
        .unwrap();
    assert_eq!(receipt.decision_id, request_id.to_string());
    assert_eq!(
        receipt.proposal_id,
        serde_json::from_str::<serde_json::Value>(&proposal).unwrap()["proposalId"]
    );
    let reopened = AppStore::for_test(
        Connection::open(f.root.path().join("fixture.sqlite")).unwrap(),
        f.root.path().join("data/objects"),
    );
    for retry_id in [request_id, Uuid::new_v4()] {
        let retried = note_rejection::reject(
            &reopened,
            Request {
                scope,
                request_id: retry_id,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            serde_json::to_value(retried).unwrap(),
            serde_json::to_value(&receipt).unwrap()
        );
    }
    assert_eq!(
        serde_json::to_value(
            note_rejection::status(&reopened, scope)
                .await
                .unwrap()
                .unwrap()
        )
        .unwrap(),
        serde_json::to_value(receipt).unwrap()
    );
    assert_eq!(counts(&reopened), before);
    let db = reopened.connection().unwrap();
    let count: i64 = db
        .query_row("select count(*) from agent_note_rejections", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 1);
    assert!(
        db.execute(
            "update agent_note_rejections set proposal_id=proposal_id",
            []
        )
        .is_err()
    );
}
fn counts(store: &AppStore) -> (i64, i64, String) {
    let db = store.connection().unwrap();
    db.query_row("select (select count(*) from agent_items),(select count(*) from note_revisions),(select group_concat(content_json) from note_blocks)",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap()
}

#[tokio::test]
async fn rejects_unverified_or_revoked_decisions_without_inserting_a_row() {
    for case in [
        "incomplete",
        "forged",
        "archived",
        "wrong_page",
        "nil_request",
    ] {
        let f = fixture(true);
        let (mut scope, _) = super::review::seeded(&f, case != "incomplete").await;
        let mut request_id = Uuid::new_v4();
        match case {
            "forged" => {
                f.store.connection().unwrap().execute("update agent_items set payload_json=json_set(payload_json,'$.payload.text','{}') where ordinal=5",[]).unwrap();
            }
            "archived" => {
                f.store
                    .connection()
                    .unwrap()
                    .execute("update agent_conversations set archived=1", [])
                    .unwrap();
            }
            "wrong_page" => scope.page_id = Uuid::new_v4(),
            "nil_request" => request_id = Uuid::nil(),
            _ => {}
        }
        assert!(
            note_rejection::reject(&f.store, Request { scope, request_id })
                .await
                .is_err(),
            "{case}"
        );
        let rows: i64 = f
            .store
            .connection()
            .unwrap()
            .query_row("select count(*) from agent_note_rejections", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(rows, 0);
    }
}

#[tokio::test]
async fn concurrent_rejection_uses_one_canonical_decision_and_failed_insert_can_retry() {
    let f = fixture(true);
    let (scope, _) = super::review::seeded(&f, true).await;
    let request_id = Uuid::new_v4();
    f.store.connection().unwrap().execute_batch("create trigger reject_decision before insert on agent_note_rejections begin select raise(abort,'synthetic write failure'); end;").unwrap();
    assert!(
        note_rejection::reject(&f.store, Request { scope, request_id })
            .await
            .is_err()
    );
    assert!(
        note_rejection::status(&f.store, scope)
            .await
            .unwrap()
            .is_none()
    );
    f.store
        .connection()
        .unwrap()
        .execute_batch("drop trigger reject_decision")
        .unwrap();
    let (a, b) = tokio::join!(
        note_rejection::reject(&f.store, Request { scope, request_id }),
        note_rejection::reject(
            &f.store,
            Request {
                scope,
                request_id: Uuid::new_v4()
            }
        )
    );
    assert_eq!(
        serde_json::to_value(a.unwrap()).unwrap(),
        serde_json::to_value(b.unwrap()).unwrap()
    );
    let mut db = f.store.connection().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    assert_eq!(
        db.query_row("select count(*) from agent_note_rejections", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn rejection_request_cannot_supply_proposal_or_content_or_acceptance_authority() {
    let scope = json!({"conversationId":Uuid::new_v4(),"pageId":Uuid::new_v4(),"turnId":Uuid::new_v4(),"itemId":Uuid::new_v4()});
    let request = json!({"requestId":Uuid::new_v4(),"scope":scope});
    assert!(serde_json::from_value::<Request>(request.clone()).is_ok());
    for field in ["proposalId", "proposal", "after", "allowWrite", "accept"] {
        let mut changed = request.clone();
        changed[field] = json!(true);
        assert!(serde_json::from_value::<Request>(changed).is_err());
    }
    assert!(serde_json::from_value::<Scope>(json!({"allowWrite":true})).is_err());
}

#[tokio::test]
async fn request_identity_cannot_be_reused_and_chat_deletion_does_not_delete_notes() {
    let f = fixture(true);
    let (scope, proposal) = super::review::seeded(&f, true).await;
    let request_id = Uuid::new_v4();
    let foreign_item = Uuid::new_v4();
    let proposal: serde_json::Value = serde_json::from_str(&proposal).unwrap();
    f.store.connection().unwrap().execute("insert into agent_note_rejections(decision_id,conversation_id,page_id,turn_id,item_id,proposal_id) values(?1,?2,?3,?4,?5,?6)",
        rusqlite::params![request_id.to_string(),scope.conversation_id.to_string(),scope.page_id.to_string(),scope.turn_id.to_string(),foreign_item.to_string(),proposal["proposalId"].as_str().unwrap()]).unwrap();
    assert!(matches!(
        note_rejection::reject(&f.store, Request { scope, request_id }).await,
        Err(crate::agent::note_review::Error::InvalidInput)
    ));
    assert!(
        note_rejection::status(&f.store, scope)
            .await
            .unwrap()
            .is_none()
    );
    let db = f.store.connection().unwrap();
    let blocks: i64 = db
        .query_row("select count(*) from note_blocks", [], |r| r.get(0))
        .unwrap();
    db.execute(
        "delete from agent_conversations where id=?1",
        [scope.conversation_id.to_string()],
    )
    .unwrap();
    assert_eq!(
        db.query_row("select count(*) from agent_note_rejections", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("select count(*) from note_blocks", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        blocks
    );
}
