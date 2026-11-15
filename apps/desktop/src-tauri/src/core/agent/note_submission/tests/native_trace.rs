#![cfg(unix)]
use super::*;
use std::{fs, time::Duration};
use tokio::time::timeout;

#[path = "native_trace/fixture.rs"]
mod fixture;
use fixture::*;

#[tokio::test]
#[ignore = "requires an independently built host via RESEARCH_ASSISTANT_TEST_LITERATURE_HOST"]
async fn real_host_commits_the_original_note_trace_and_lost_reply_restores_without_reconsumption() {
    let mut f = fixture_with_owner(true, true);
    let host = attach(&mut f, &real_host());
    trace(&f);
    let id = Uuid::new_v4();
    let saved = timeout(WAIT, prepare(&f.store, &f.models, input(&f, id)))
        .await
        .unwrap()
        .unwrap();
    let context = saved
        .input
        .context
        .iter()
        .find(|value| value.kind == ContextSourceKind::Retrieval)
        .unwrap();
    assert_eq!(context.text, BODY);
    assert_eq!(context.resource_id, f.page.to_string());
    assert_eq!(context.source_id, format!("retrieval:{TRACE}"));
    assert_eq!(context.revision, format!("trace:{TRACE}"));
    let anchor: serde_json::Value = serde_json::from_str(context.anchor.as_ref().unwrap()).unwrap();
    assert_eq!(anchor["pageId"], f.page.to_string());
    assert_eq!(anchor["traceId"], TRACE);
    assert_eq!(anchor["query"], QUERY);
    assert_eq!(status(&f), "sent");
    assert_eq!(count(&f), 1);
    // Simulate a lost response: use the original receipt, not another prepare.
    assert_eq!(restore(&f.store, receipt(&f, id)).await.unwrap(), saved);
    assert_eq!(
        prepare(&f.store, &f.models, input(&f, id)).await,
        Err(Error::Conflict)
    );
    host.service.shutdown().await.unwrap();
    // The original recovery reader must not need the native host or credentials.
    f.store
        .connection()
        .unwrap()
        .execute("update llm_provider_configs set enabled=0", [])
        .unwrap();
    let Fixture {
        root,
        store,
        models,
        conversation,
        page,
    } = f;
    drop(models);
    drop(store);
    let database = crate::data_interface::Database::open(
        Connection::open(root.path().join("fixture.sqlite")).unwrap(),
        &root.path().join("log"),
    )
    .unwrap();
    let reopened = AppStore::for_test_database(database, root.path().join("data/objects"));
    assert_eq!(
        restore(
            &reopened,
            Receipt {
                conversation_id: conversation,
                request_id: id,
                page_id: page
            }
        )
        .await
        .unwrap(),
        saved
    );
    let db = reopened.connection().unwrap();
    assert_eq!(
        db.query_row(
            "select status from concept_retrieval_traces where id=?1",
            [TRACE],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "sent"
    );
    assert_eq!(
        db.query_row("select count(*) from agent_submissions", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[tokio::test]
#[ignore = "requires an independently built host via RESEARCH_ASSISTANT_TEST_LITERATURE_HOST"]
async fn original_submission_failure_rolls_back_native_trace_consumption_and_allows_explicit_retry()
{
    let mut f = fixture_with_owner(true, true);
    let host = attach(&mut f, &real_host());
    trace(&f);
    f.store.connection().unwrap().execute_batch(
        "create trigger reject_native_note_submission before insert on agent_submissions begin select raise(abort,'synthetic'); end;",
    ).unwrap();
    let id = Uuid::new_v4();
    assert!(
        timeout(WAIT, prepare(&f.store, &f.models, input(&f, id)))
            .await
            .unwrap()
            .is_err()
    );
    assert_eq!(count(&f), 0);
    assert_eq!(status(&f), "previewed");
    let reopened = Connection::open(f.root.path().join("fixture.sqlite")).unwrap();
    assert_eq!(
        reopened
            .query_row(
                "select status from concept_retrieval_traces where id=?1",
                [TRACE],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "previewed"
    );
    assert_eq!(
        reopened
            .query_row("select count(*) from agent_submissions", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    f.store
        .connection()
        .unwrap()
        .execute_batch("drop trigger reject_native_note_submission")
        .unwrap();
    timeout(WAIT, prepare(&f.store, &f.models, input(&f, id)))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(count(&f), 1);
    assert_eq!(status(&f), "sent");
    host.service.shutdown().await.unwrap();
}

#[tokio::test]
async fn an_explicit_native_package_failure_does_not_fall_back_to_the_valid_static_trace() {
    let mut f = fixture_with_owner(true, true);
    let host = attach(&mut f, b"#!/bin/sh\nexit 41\n");
    trace(&f);
    let id = Uuid::new_v4();
    let error = timeout(WAIT, prepare(&f.store, &f.models, input(&f, id)))
        .await
        .unwrap()
        .unwrap_err();
    let Error::Native(diagnostic) = error else {
        panic!("expected structured native diagnostic");
    };
    assert_eq!(diagnostic.conversation_id, f.conversation);
    assert_eq!(diagnostic.request_id, id);
    assert_eq!(diagnostic.page_id, f.page);
    assert_eq!(diagnostic.plugin_id, "literature_library");
    assert_eq!(diagnostic.package_key.as_deref(), Some(host.key.as_str()));
    assert_eq!(diagnostic.exit_code, Some(41));
    assert!(diagnostic.instance_id.is_some() && diagnostic.native_request_id.is_some());
    let wire = serde_json::to_value(Error::Native(diagnostic)).unwrap();
    assert!(wire.get("code").is_some());
    assert!(wire.get("native").is_none());
    assert_eq!(count(&f), 0);
    assert_eq!(status(&f), "previewed");
    host.service.shutdown().await.unwrap();
}

#[tokio::test]
async fn pending_native_decision_releases_the_database_for_another_core_note_save() {
    let mut f = fixture_with_owner(true, true);
    let marker = f.root.path().join("native-waiting");
    let host = attach(&mut f, &paused_host(&marker, None));
    trace(&f);
    let mut pending = Box::pin(prepare(&f.store, &f.models, input(&f, Uuid::new_v4())));
    let marked = wait_for_marker(&marker);
    tokio::pin!(marked);
    tokio::select! {
        result = &mut pending => panic!("unexpected early prepare result: {result:?}"),
        _ = &mut marked => {}
    }
    let save_store = f.store.clone();
    let saved = timeout(
        Duration::from_secs(2),
        tokio::task::spawn_blocking(move || {
            let mut db = save_store.connection().unwrap();
            let tx = db
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .unwrap();
            let id = crate::notes::create_page_with_paper_in(
                &tx,
                crate::notes::NotePageDraft {
                    title: "Another Core note while native waits".into(),
                    parent_page_id: None,
                    parent_block_position: None,
                },
                None,
                None,
            )
            .unwrap();
            tx.commit().unwrap();
            id
        }),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(!saved.is_empty());
    drop(pending);
    timeout(WAIT, host.service.shutdown())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(count(&f), 0);
    assert_eq!(status(&f), "previewed");
}

#[tokio::test]
async fn t0_to_t1_sent_or_sql_expiry_changes_are_rejected_after_a_real_child_wait() {
    for update in [
        "update concept_retrieval_traces set status='sent',sent_at='now'",
        "update concept_retrieval_traces set expires_at='2000-01-01T00:00:00.000Z'",
    ] {
        let mut f = fixture_with_owner(true, true);
        let marker = f.root.path().join("native-waiting");
        let gate = f.root.path().join("release-native");
        let host = attach(&mut f, &paused_host(&marker, Some(&gate)));
        trace(&f);
        let mut pending = Box::pin(prepare(&f.store, &f.models, input(&f, Uuid::new_v4())));
        let marked = wait_for_marker(&marker);
        tokio::pin!(marked);
        tokio::select! {
            result = &mut pending => panic!("unexpected early prepare result: {result:?}"),
            _ = &mut marked => {}
        }
        f.store.connection().unwrap().execute(update, []).unwrap();
        fs::write(&gate, "release").unwrap();
        let error = timeout(WAIT, pending).await.unwrap().unwrap_err();
        let Error::Native(diagnostic) = error else {
            panic!("expected data recheck diagnostic");
        };
        assert_eq!(diagnostic.code, "plugin_data_snapshot_changed");
        assert_eq!(diagnostic.phase.as_deref(), Some("recheck"));
        assert!(diagnostic.instance_id.is_some() && diagnostic.native_request_id.is_some());
        assert_eq!(count(&f), 0);
        host.service.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn note_and_selection_errors_precede_a_missing_native_package() {
    let mut f = fixture_with_owner(true, true);
    let host = attach(&mut f, b"#!/bin/sh\nexit 41\n");
    trace(&f);
    fs::remove_file(host.directory.path().join("literature/host")).unwrap();
    let mut invalid = input(&f, Uuid::new_v4());
    invalid.context.expected_revision = 1;
    invalid.retrieval.as_mut().unwrap().query = "\n".into();
    assert_eq!(
        prepare(&f.store, &f.models, invalid).await,
        Err(Error::Conflict)
    );
    let mut invalid = input(&f, Uuid::new_v4());
    invalid.retrieval.as_mut().unwrap().query = "\n".into();
    assert_eq!(
        prepare(&f.store, &f.models, invalid).await,
        Err(Error::InvalidInput)
    );
    assert_eq!(count(&f), 0);
    assert_eq!(status(&f), "previewed");
    host.service.shutdown().await.unwrap();
}

#[tokio::test]
async fn the_generic_queued_preparation_retains_its_existing_trace_route() {
    let mut f = fixture_with_owner(true, true);
    let host = attach(&mut f, b"#!/bin/sh\nexit 41\n");
    trace(&f);
    let request = input(&f, Uuid::new_v4());
    let request = SelectedRequest {
        conversation_id: request.conversation_id,
        request_id: request.request_id,
        text: request.text,
        user_input: request.user_input,
        context: Selection::Page(request.context),
        retrieval: request.retrieval,
        model: request.model,
        session_config: None,
        turn_config: None,
    };
    let result: Result<SubmissionRecord, Error> =
        prepare_selected_with(&f.store, &f.models, request, |tx, receipt, input| {
            insert_in(tx, receipt.conversation_id, receipt.request_id, input)
                .map_err(submission_error)
        })
        .await;
    result.unwrap();
    assert_eq!(status(&f), "sent");
    assert_eq!(count(&f), 1);
    host.service.shutdown().await.unwrap();
}
