use super::*;
use crate::agent::{
    note_application,
    note_review::{Error, Scope},
    note_undo::{self, Request},
};
use serde_json::json;

#[test]
fn undo_request_does_not_accept_renderer_document_or_extra_authority() {
    let value = json!({"scope":{"conversationId":Uuid::new_v4(),"pageId":Uuid::new_v4(),"turnId":Uuid::new_v4(),"itemId":Uuid::new_v4()},
        "applicationId":Uuid::new_v4(),"requestId":Uuid::new_v4(),"expectedRevision":1});
    assert!(serde_json::from_value::<Request>(value.clone()).is_ok());
    for key in ["document", "proposal", "before", "after", "allowWrite"] {
        let mut invalid = value.clone();
        invalid[key] = json!(true);
        assert!(serde_json::from_value::<Request>(invalid).is_err());
    }
}

async fn applied(f: &Fixture) -> (Scope, note_application::Receipt) {
    let (scope, _) = super::review::seeded(f, true).await;
    let receipt = note_application::accept(
        &f.store,
        note_application::Request {
            scope,
            request_id: Uuid::new_v4(),
            expected_revision: 0,
        },
    )
    .await
    .unwrap();
    (scope, receipt)
}
fn request(scope: Scope, application: &note_application::Receipt) -> Request {
    Request {
        scope,
        application_id: Uuid::parse_str(&application.application_id).unwrap(),
        request_id: Uuid::new_v4(),
        expected_revision: 1,
    }
}
fn state(store: &AppStore) -> (i64, String, i64, i64) {
    store
        .connection()
        .unwrap()
        .query_row(
            "select (select revision from note_pages limit 1),
        (select coalesce(group_concat(content_json),'') from note_blocks),(select count(*) from note_revisions),
        (select count(*) from agent_note_undos)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap()
}

#[tokio::test]
async fn undo_restores_whole_page_and_survives_reopen_without_rewriting_history() {
    let f = fixture(true);
    f.store.connection().unwrap().execute("insert into note_blocks(id,page_id,scope_kind,block_type,position,content_json,created_at,updated_at)
        values('sibling',?1,'page','paragraph',1,?2,'now','now')",params![f.page.to_string(),json!({"kind":"inline","items":[{"type":"text","text":"Unselected sibling","styles":[]}]}).to_string()]).unwrap();
    let original = state(&f.store).1;
    let (scope, application) = applied(&f).await;
    assert!(note_undo::status(&f.store, scope).await.unwrap().is_none());
    let req = request(scope, &application);
    let id = req.request_id;
    let receipt = note_undo::undo(&f.store, req).await.unwrap();
    assert_eq!(receipt.undo_id, id.to_string());
    assert_eq!(receipt.restored_revision, 2);
    assert_eq!(receipt.before_hash, application.after_hash);
    assert_eq!(receipt.restored_hash, application.before_hash);
    let after = state(&f.store);
    assert_eq!(after.0, 2);
    assert_eq!(after.1, original);
    assert_eq!(after.2, 2);
    assert_eq!(after.3, 1);
    let reopened = AppStore::for_test(
        Connection::open(f.root.path().join("fixture.sqlite")).unwrap(),
        f.root.path().join("data/objects"),
    );
    for retry_id in [id, Uuid::new_v4()] {
        let mut retry = request(scope, &application);
        retry.request_id = retry_id;
        assert_eq!(
            serde_json::to_value(note_undo::undo(&reopened, retry).await.unwrap()).unwrap(),
            serde_json::to_value(&receipt).unwrap()
        );
    }
    assert_eq!(state(&reopened), after);
    assert_eq!(
        serde_json::to_value(note_undo::status(&reopened, scope).await.unwrap().unwrap()).unwrap(),
        serde_json::to_value(receipt).unwrap()
    );
    assert_eq!(
        serde_json::to_value(
            note_application::status(&reopened, scope)
                .await
                .unwrap()
                .unwrap()
        )
        .unwrap(),
        serde_json::to_value(&application).unwrap()
    );
    let db = reopened.connection().unwrap();
    assert!(
        db.execute(
            "update agent_note_undos set restored_revision=restored_revision",
            []
        )
        .is_err()
    );
    let applied_history: String = db
        .query_row(
            "select document_json from note_revisions where page_revision=1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(applied_history.contains("Reviewed suggestion"));
}

#[tokio::test]
async fn undo_receipt_and_dependent_save_failures_roll_back_every_write() {
    for trigger in [
        "create trigger fail_undo before insert on agent_note_undos begin select raise(abort,'receipt'); end;",
        "create trigger fail_undo before update on note_index_state begin select raise(abort,'index'); end;",
    ] {
        let f = fixture(true);
        let (scope, application) = applied(&f).await;
        let before = state(&f.store);
        f.store
            .connection()
            .unwrap()
            .execute_batch(trigger)
            .unwrap();
        assert!(
            note_undo::undo(&f.store, request(scope, &application))
                .await
                .is_err()
        );
        assert_eq!(state(&f.store), before);
        f.store
            .connection()
            .unwrap()
            .execute_batch("drop trigger fail_undo")
            .unwrap();
        note_undo::undo(&f.store, request(scope, &application))
            .await
            .unwrap();
        assert_eq!(state(&f.store).0, 2);
    }
}

#[tokio::test]
async fn later_revision_or_same_revision_changed_bytes_block_undo() {
    for change in [
        "update note_pages set revision=revision+1",
        "update note_blocks set content_json=replace(content_json,'Reviewed suggestion','New manual input')",
    ] {
        let f = fixture(true);
        let (scope, application) = applied(&f).await;
        f.store.connection().unwrap().execute_batch(change).unwrap();
        let before = state(&f.store);
        assert!(matches!(
            note_undo::undo(&f.store, request(scope, &application)).await,
            Err(Error::Conflict)
        ));
        assert_eq!(state(&f.store), before);
    }
}

#[tokio::test]
async fn concurrent_undo_and_old_retries_never_overwrite_later_edits() {
    let f = fixture(true);
    let (scope, application) = applied(&f).await;
    let (a, b) = tokio::join!(
        note_undo::undo(&f.store, request(scope, &application)),
        note_undo::undo(&f.store, request(scope, &application))
    );
    let a = a.unwrap();
    let b = b.unwrap();
    assert_eq!(a.undo_id, b.undo_id);
    assert_eq!(state(&f.store).0, 2);
    f.store.connection().unwrap().execute_batch("update note_pages set revision=revision+1; update note_blocks set content_json=replace(content_json,'Frozen note evidence','Later manual input')").unwrap();
    let before = state(&f.store);
    let retry = note_undo::undo(&f.store, request(scope, &application))
        .await
        .unwrap();
    assert_eq!(retry.undo_id, a.undo_id);
    assert_eq!(state(&f.store), before);
    // Historical recovery does not need a later-deleted selected block.
    f.store
        .connection()
        .unwrap()
        .execute("delete from note_blocks where id='selected'", [])
        .unwrap();
    let before = state(&f.store);
    assert_eq!(
        note_undo::undo(&f.store, request(scope, &application))
            .await
            .unwrap()
            .undo_id,
        a.undo_id
    );
    assert_eq!(state(&f.store), before);
}

#[tokio::test]
async fn wrong_application_scope_revision_or_revoked_binding_cannot_undo() {
    for case in [
        "application",
        "page",
        "turn",
        "item",
        "revision",
        "binding",
        "archived",
    ] {
        let f = fixture(true);
        let (scope, application) = applied(&f).await;
        let mut req = request(scope, &application);
        match case {
            "application" => req.application_id = Uuid::new_v4(),
            "page" => req.scope.page_id = Uuid::new_v4(),
            "turn" => req.scope.turn_id = Uuid::new_v4(),
            "item" => req.scope.item_id = Uuid::new_v4(),
            "revision" => req.expected_revision = 2,
            "binding" => {
                f.store
                    .connection()
                    .unwrap()
                    .execute("delete from agent_conversation_bindings", [])
                    .unwrap();
            }
            "archived" => {
                f.store
                    .connection()
                    .unwrap()
                    .execute("update note_pages set status='archived'", [])
                    .unwrap();
            }
            _ => unreachable!(),
        }
        let before = state(&f.store);
        assert!(note_undo::undo(&f.store, req).await.is_err(), "{case}");
        assert_eq!(state(&f.store), before);
    }
}

#[tokio::test]
async fn sql_checks_bind_the_original_hashes_and_receipt_is_queryable() {
    let f = fixture(true);
    let (scope, application) = applied(&f).await;
    let req = request(scope, &application);
    let id = req.request_id;
    // A receipt cannot assert arbitrary restored bytes, even through direct SQL.
    assert!(f.store.connection().unwrap().execute("insert into agent_note_undos(undo_id,application_id,before_revision,restored_revision,before_hash,restored_hash)
        values(?1,?2,1,2,?3,?3)",params![id.to_string(),application.application_id,application.after_hash]).is_err());
    note_undo::undo(&f.store, req).await.unwrap();
    // The original tool record remains sufficient to identify historical status.
    assert_eq!(
        note_undo::status(&f.store, scope)
            .await
            .unwrap()
            .unwrap()
            .undo_id,
        id.to_string()
    );
}

#[tokio::test]
async fn request_id_collision_does_not_write_and_page_deletion_cascades_receipts() {
    let f = fixture(true);
    let (scope, application) = applied(&f).await;
    let other = Uuid::new_v4();
    let req = request(scope, &application);
    {
        let db = f.store.connection().unwrap();
        // An independent historical application owns the caller's request ID.
        db.execute("insert into agent_note_applications(application_id,conversation_id,page_id,turn_id,item_id,proposal_id,before_revision,applied_revision,before_hash,after_hash)
            values(?1,?2,?3,?4,?5,?6,0,1,?7,?8)",params![other.to_string(),application.conversation_id,application.page_id,application.turn_id,Uuid::new_v4().to_string(),application.proposal_id,application.before_hash,application.after_hash]).unwrap();
        db.execute("insert into agent_note_undos(undo_id,application_id,before_revision,restored_revision,before_hash,restored_hash)
            values(?1,?2,1,2,?3,?4)",params![req.request_id.to_string(),other.to_string(),application.after_hash,application.before_hash]).unwrap();
    }
    let before = state(&f.store);
    assert!(matches!(
        note_undo::undo(&f.store, req).await,
        Err(Error::InvalidInput)
    ));
    assert_eq!(state(&f.store), before);
    note_undo::undo(&f.store, request(scope, &application))
        .await
        .unwrap();
    let db = f.store.connection().unwrap();
    db.execute("delete from note_pages where id=?1", [f.page.to_string()])
        .unwrap();
    assert_eq!(
        db.query_row("select count(*) from agent_note_undos", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn undo_preserves_child_pages_assets_and_their_history_ownership() {
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
            db.execute("insert into note_blocks(id,page_id,scope_kind,block_type,position,content_json,created_at,updated_at) values(?1,?2,'page',?3,?4,?5,'now','now')",params![id,f.page.to_string(),kind,position,content.to_string()]).unwrap();
        }
    }
    let (scope, application) = applied(&f).await;
    note_undo::undo(&f.store, request(scope, &application))
        .await
        .unwrap();
    let db = f.store.connection().unwrap();
    let child_state: (String, i64, String) = db
        .query_row(
            "select status,revision,parent_page_id from note_pages where id=?1",
            [child],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(child_state, ("active".into(), 0, f.page.to_string()));
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
        2
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
