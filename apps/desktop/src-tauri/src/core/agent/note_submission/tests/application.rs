use super::*;
use crate::agent::{
    note_application::{self, Request},
    note_rejection,
    note_review::Error,
};
use serde_json::{Value, json};

fn apply_request(scope: crate::agent::note_review::Scope, id: Uuid) -> Request {
    Request {
        scope,
        request_id: id,
        expected_revision: 0,
    }
}
fn state(store: &AppStore) -> (i64, String, i64, i64, i64) {
    store
        .connection()
        .unwrap()
        .query_row(
            "select (select revision from note_pages limit 1),
         (select group_concat(content_json) from note_blocks),
         (select count(*) from note_revisions),(select count(*) from agent_note_applications),
         (select count(*) from agent_items)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap()
}

#[tokio::test]
async fn acceptance_saves_the_full_page_and_durable_receipt_exactly_once() {
    let f = fixture(true);
    // The frozen source selects one block; unrelated page content must survive.
    f.store.connection().unwrap().execute(
        "insert into note_blocks(id,page_id,scope_kind,block_type,position,content_json,created_at,updated_at)
         values ('sibling',?1,'page','paragraph',1,?2,'now','now')",
        params![f.page.to_string(),json!({"kind":"inline","items":[{"type":"text","text":"Unselected sibling","styles":[]}]}).to_string()],
    ).unwrap();
    let (scope, _) = super::review::seeded(&f, true).await;
    assert!(
        note_application::status(&f.store, scope)
            .await
            .unwrap()
            .is_none()
    );
    let before = state(&f.store);
    let request_id = Uuid::new_v4();
    let receipt = note_application::accept(&f.store, apply_request(scope, request_id))
        .await
        .unwrap();
    assert_eq!(receipt.before_revision, 0);
    assert_eq!(receipt.applied_revision, 1);
    assert_eq!(receipt.application_id, request_id.to_string());
    assert_ne!(receipt.before_hash, receipt.after_hash);
    let after = state(&f.store);
    assert_eq!(after.0, 1);
    assert!(after.1.contains("Reviewed suggestion"));
    assert!(after.1.contains("Unselected sibling"));
    assert_eq!(after.2, 1);
    assert_eq!(after.3, 1);
    assert_eq!(after.4, before.4);
    let original: String = f
        .store
        .connection()
        .unwrap()
        .query_row("select document_json from note_revisions", [], |r| r.get(0))
        .unwrap();
    assert!(original.contains("Frozen note evidence"));
    assert!(original.contains("Unselected sibling"));
    let reopened = AppStore::for_test(
        Connection::open(f.root.path().join("fixture.sqlite")).unwrap(),
        f.root.path().join("data/objects"),
    );
    for id in [request_id, Uuid::new_v4()] {
        let retry = note_application::accept(&reopened, apply_request(scope, id))
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(retry).unwrap(),
            serde_json::to_value(&receipt).unwrap()
        );
    }
    assert_eq!(state(&reopened), after);
    assert_eq!(
        serde_json::to_value(
            note_application::status(&reopened, scope)
                .await
                .unwrap()
                .unwrap()
        )
        .unwrap(),
        serde_json::to_value(receipt).unwrap()
    );
    assert!(
        reopened
            .connection()
            .unwrap()
            .execute(
                "update agent_note_applications set applied_revision=applied_revision",
                []
            )
            .is_err()
    );
}

#[tokio::test]
async fn failed_receipt_or_dependent_write_rolls_back_the_entire_acceptance() {
    for trigger in [
        "create trigger synthetic_failure before insert on agent_note_applications begin select raise(abort,'synthetic receipt failure'); end;",
        "create trigger synthetic_failure before update on note_index_state begin select raise(abort,'synthetic index failure'); end;",
    ] {
        let f = fixture(true);
        let (scope, _) = super::review::seeded(&f, true).await;
        let before = state(&f.store);
        let request_id = Uuid::new_v4();
        f.store
            .connection()
            .unwrap()
            .execute_batch(trigger)
            .unwrap();
        assert!(
            note_application::accept(&f.store, apply_request(scope, request_id))
                .await
                .is_err()
        );
        assert_eq!(state(&f.store), before);
        f.store
            .connection()
            .unwrap()
            .execute_batch("drop trigger synthetic_failure")
            .unwrap();
        note_application::accept(&f.store, apply_request(scope, request_id))
            .await
            .unwrap();
        assert_eq!(state(&f.store).0, 1);
    }
}

#[tokio::test]
async fn stale_forged_incomplete_or_revoked_proposals_cannot_be_applied() {
    for case in [
        "revision",
        "unchanged_revision_bytes",
        "incomplete",
        "forged",
        "archived",
        "nil_request",
        "wrong_revision",
    ] {
        let f = fixture(true);
        let (scope, _) = super::review::seeded(&f, case != "incomplete").await;
        let mut request = apply_request(scope, Uuid::new_v4());
        match case {
            "revision" => {
                f.store
                    .connection()
                    .unwrap()
                    .execute("update note_pages set revision=1", [])
                    .unwrap();
            }
            "unchanged_revision_bytes" => {
                f.store.connection().unwrap().execute("update note_blocks set content_json=json_set(content_json,'$.items[0].text','Changed evidence')",[]).unwrap();
            }
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
            "nil_request" => request.request_id = Uuid::nil(),
            "wrong_revision" => request.expected_revision = 1,
            _ => {}
        }
        let before = state(&f.store);
        assert!(
            note_application::accept(&f.store, request).await.is_err(),
            "{case}"
        );
        assert_eq!(state(&f.store), before, "{case}");
    }
}

#[tokio::test]
async fn accept_and_reject_are_mutually_exclusive_in_either_order_and_when_concurrent() {
    for first in ["accept", "reject", "race"] {
        let f = fixture(true);
        let (scope, _) = super::review::seeded(&f, true).await;
        let apply = apply_request(scope, Uuid::new_v4());
        let reject = note_rejection::Request {
            scope,
            request_id: Uuid::new_v4(),
        };
        match first {
            "accept" => {
                note_application::accept(&f.store, apply).await.unwrap();
                assert!(matches!(
                    note_rejection::reject(&f.store, reject).await,
                    Err(Error::Decided)
                ));
            }
            "reject" => {
                note_rejection::reject(&f.store, reject).await.unwrap();
                assert!(matches!(
                    note_application::accept(&f.store, apply).await,
                    Err(Error::Decided)
                ));
            }
            _ => {
                let (a, b) = tokio::join!(
                    note_application::accept(&f.store, apply),
                    note_rejection::reject(&f.store, reject)
                );
                assert_ne!(a.is_ok(), b.is_ok());
            }
        }
        let (applied,rejected):(i64,i64)=f.store.connection().unwrap().query_row(
            "select (select count(*) from agent_note_applications),(select count(*) from agent_note_rejections)",[],|r|Ok((r.get(0)?,r.get(1)?)),
        ).unwrap();
        assert_eq!(applied + rejected, 1);
        assert_eq!(state(&f.store).0, applied);
    }
}

#[tokio::test]
async fn simultaneous_acceptance_and_recovery_do_not_reapply_or_replace_later_edits() {
    let f = fixture(true);
    let (scope, _) = super::review::seeded(&f, true).await;
    let id = Uuid::new_v4();
    let (a, b) = tokio::join!(
        note_application::accept(&f.store, apply_request(scope, id)),
        note_application::accept(&f.store, apply_request(scope, Uuid::new_v4()))
    );
    assert_eq!(
        serde_json::to_value(a.unwrap()).unwrap(),
        serde_json::to_value(b.unwrap()).unwrap()
    );
    f.store
        .connection()
        .unwrap()
        .execute("update note_pages set revision=2", [])
        .unwrap();
    f.store.connection().unwrap().execute("update note_blocks set content_json=json_set(content_json,'$.items[0].text','Later user edit')",[]).unwrap();
    let before = state(&f.store);
    let retry = note_application::accept(&f.store, apply_request(scope, id))
        .await
        .unwrap();
    assert_eq!(retry.applied_revision, 1);
    assert_eq!(state(&f.store), before);
    let mut changed = apply_request(scope, id);
    changed.expected_revision = 2;
    assert!(matches!(
        note_application::accept(&f.store, changed).await,
        Err(Error::Conflict)
    ));
    let mut db = f.store.connection().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    db.execute("delete from agent_conversations", []).unwrap();
    drop(db);
    let after = state(&f.store);
    assert_eq!(after.0, before.0);
    assert_eq!(after.1, before.1);
    assert_eq!(after.3, 0);
}

#[test]
fn acceptance_request_rejects_document_bytes_and_unknown_authority() {
    let request = json!({"scope":{"conversationId":Uuid::new_v4(),"pageId":Uuid::new_v4(),"turnId":Uuid::new_v4(),"itemId":Uuid::new_v4()},"requestId":Uuid::new_v4(),"expectedRevision":0});
    assert!(serde_json::from_value::<Request>(request.clone()).is_ok());
    for field in ["document", "before", "after", "proposal", "allowWrite"] {
        let mut invalid = request.clone();
        invalid[field] = Value::Bool(true);
        assert!(serde_json::from_value::<Request>(invalid).is_err());
    }
}

#[tokio::test]
async fn acceptance_preserves_child_pages_and_assets_outside_the_selected_source() {
    let f = fixture(true);
    let child = Uuid::new_v4().to_string();
    {
        let db = f.store.connection().unwrap();
        db.execute("insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at) values(?1,'synthetic-asset','image/png',0,'now')",["a".repeat(64)]).unwrap();
        db.execute("insert into note_pages(id,parent_page_id,title,created_at,updated_at) values(?1,?2,'Child','now','now')",params![child,f.page.to_string()]).unwrap();
        for (id, kind, content, position) in [
            (
                "child-link",
                "childPage",
                json!({"kind":"reference","targetKind":"note_page","targetId":child,"labelSnapshot":"Child","quoteSnapshot":""}),
                1,
            ),
            (
                "asset",
                "image",
                json!({"kind":"asset","assetId":"a".repeat(64),"displayName":"Evidence","mimeType":"image/png","caption":"Source"}),
                2,
            ),
        ] {
            db.execute("insert into note_blocks(id,page_id,scope_kind,block_type,position,content_json,created_at,updated_at) values(?1,?2,'page',?3,?4,?5,'now','now')",
                params![id,f.page.to_string(),kind,position,content.to_string()]).unwrap();
        }
    }
    let (scope, _) = super::review::seeded(&f, true).await;
    note_application::accept(&f.store, apply_request(scope, Uuid::new_v4()))
        .await
        .unwrap();
    let db = f.store.connection().unwrap();
    let (status, revision, parent): (String, i64, String) = db
        .query_row(
            "select status,revision,parent_page_id from note_pages where id=?1",
            [child],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(status, "active");
    assert_eq!(revision, 0);
    assert_eq!(parent, f.page.to_string());
    assert_eq!(db.query_row("select count(*) from note_blocks where id in ('asset','child-link') and status='active'",[],|r|r.get::<_,i64>(0)).unwrap(),2);
    assert_eq!(
        db.query_row(
            "select count(*) from note_block_assets where block_id='asset'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("select count(*) from note_revision_assets", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row(
            "select count(*) from note_links where source_block_id='child-link'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}

#[tokio::test]
async fn database_itself_blocks_opposite_decisions() {
    for accepted in [true, false] {
        let f = fixture(true);
        let (scope, proposal) = super::review::seeded(&f, true).await;
        let proposal: Value = serde_json::from_str(&proposal).unwrap();
        if accepted {
            note_application::accept(&f.store, apply_request(scope, Uuid::new_v4()))
                .await
                .unwrap();
        } else {
            note_rejection::reject(
                &f.store,
                note_rejection::Request {
                    scope,
                    request_id: Uuid::new_v4(),
                },
            )
            .await
            .unwrap();
        }
        let db = f.store.connection().unwrap();
        let sql = if accepted {
            "insert into agent_note_rejections(decision_id,conversation_id,page_id,turn_id,item_id,proposal_id) values(?1,?2,?3,?4,?5,?6)"
        } else {
            "insert into agent_note_applications(application_id,conversation_id,page_id,turn_id,item_id,proposal_id,before_revision,applied_revision,before_hash,after_hash) values(?1,?2,?3,?4,?5,?6,0,1,printf('%064d',0),printf('%064d',1))"
        };
        assert!(
            db.execute(
                sql,
                params![
                    Uuid::new_v4().to_string(),
                    scope.conversation_id.to_string(),
                    scope.page_id.to_string(),
                    scope.turn_id.to_string(),
                    scope.item_id.to_string(),
                    proposal["proposalId"].as_str().unwrap()
                ]
            )
            .is_err()
        );
    }
}
