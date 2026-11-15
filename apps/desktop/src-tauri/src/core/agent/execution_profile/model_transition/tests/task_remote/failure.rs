//! A failed native fallback keeps the installed window and the first error.
use super::*;
use crate::agent::tasks::TaskCompletion;
use research_agent_core::{protocol::EventPayload, runner::RunError};
use uuid::Uuid;

#[derive(Clone, Copy, PartialEq, Eq)]
enum AccountFailure {
    None,
    Changed,
    Revoked,
}

#[tokio::test]
async fn remote_fallback_auth_failure_keeps_original_error_and_never_samples() {
    for native in [
        NativeCompactionPolicy::ChatGptResponsesV2,
        NativeCompactionPolicy::ChatGptResponsesFullWindow,
    ] {
        exercise(native, false, AccountFailure::None).await;
    }
}

#[tokio::test]
async fn remote_fallback_invalid_candidate_keeps_original_window_and_error() {
    for native in [
        NativeCompactionPolicy::ChatGptResponsesV2,
        NativeCompactionPolicy::ChatGptResponsesFullWindow,
    ] {
        exercise(native, true, AccountFailure::None).await;
    }
}

#[tokio::test]
async fn remote_fallback_account_change_rejects_before_http_and_preserves_original_window() {
    for native in [
        NativeCompactionPolicy::ChatGptResponsesV2,
        NativeCompactionPolicy::ChatGptResponsesFullWindow,
    ] {
        exercise(native, false, AccountFailure::Changed).await;
    }
}

#[tokio::test]
async fn remote_fallback_revoked_lease_rejects_before_http_and_preserves_original_window() {
    for native in [
        NativeCompactionPolicy::ChatGptResponsesV2,
        NativeCompactionPolicy::ChatGptResponsesFullWindow,
    ] {
        exercise(native, false, AccountFailure::Revoked).await;
    }
}

async fn exercise(native: NativeCompactionPolicy, invalid: bool, failure: AccountFailure) {
    let (lease, epoch) = crate::agent::authentication::tests::synthetic_lease();
    let account = if failure == AccountFailure::Revoked {
        lease.scope_id()
    } else {
        Uuid::from_u128(1)
    };
    let mut fixture = Fixture::new().await;
    let old_model = model("old");
    let current_model = model("current");
    let item = json!({"type":"compaction","encrypted_content":"original-window"});
    let failed_old = if native == NativeCompactionPolicy::ChatGptResponsesV2 {
        wire("text/event-stream", [json!({"type":"response.created","response":{"id":"old-failed","model":"gpt-5.6-terra"}}), json!({"type":"error","code":"context_length_exceeded"})].into_iter().map(|e| format!("data: {e}\n\n")).collect())
    } else {
        "HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into()
    };
    let fallback_response = if invalid {
        if native == NativeCompactionPolicy::ChatGptResponsesV2 {
            // Completed without a done compaction item cannot create a window.
            wire("text/event-stream", [json!({"type":"response.created","response":{"id":"bad","model":"gpt-5.6-terra"}}), json!({"type":"response.completed","response":{"id":"bad","model":"gpt-5.6-terra","status":"completed","output":[]}})].into_iter().map(|e| format!("data: {e}\n\n")).collect())
        } else {
            wire(
                "application/json",
                json!({"model":"gpt-5.6-terra","output":[{}]}).to_string(),
            )
        }
    } else {
        "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into()
    };
    let (old_url, old_peer) = server(vec![
        sample("old-response"),
        compact(native, &item),
        failed_old,
    ])
    .await;
    let (current_url, current_peer) = if failure != AccountFailure::None {
        // An open endpoint distinguishes rejection before I/O from a failed
        // connection. Keep it alive until the native task has terminated.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        (url, Err(listener))
    } else {
        let (url, peer) = server(vec![fallback_response]).await;
        (url, Ok(peer))
    };
    let first = fixture
        .record(old_model.clone(), "historical evidence")
        .await;
    let old_transport = fixture.transport_with_account(&first, &old_url, native, account);
    fixture
        .run(
            &first,
            fixture.execution(&first, old_transport.clone(), native),
            false,
        )
        .await;
    let manual = fixture.record(old_model.clone(), "/compact").await;
    fixture
        .run(
            &manual,
            fixture.execution(
                &manual,
                fixture.transport_with_account(&manual, &old_url, native, account),
                native,
            ),
            true,
        )
        .await;
    let original = fixture
        .service
        .active_context_checkpoint(fixture.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(original.credential_scope_id, Some(account));
    let next = fixture
        .record(current_model.clone(), "fresh submission")
        .await;
    let mut execution = fixture.execution(
        &next,
        fixture.transport_with_account(&next, &current_url, native, account),
        native,
    );
    if failure != AccountFailure::None {
        // Preparation still sees the original account. Only the current-model
        // fallback producer carries changed or revoked account authority.
        let mut fallback = fixture.transport_with_account(
            &next,
            &current_url,
            native,
            if failure == AccountFailure::Changed {
                Uuid::from_u128(2)
            } else {
                account
            },
        );
        if failure == AccountFailure::Revoked {
            // The checkpoint and lease scopes match; only live revocation can
            // reject this producer. Preparation uses the original transport.
            lease.check().unwrap();
            epoch.send(Uuid::new_v4()).unwrap();
            Arc::get_mut(&mut fallback)
                .unwrap()
                .use_synthetic_lease(lease);
        }
        execution.compaction_strategy = Some(Arc::new(
            crate::agent::execution_profile::compaction::NativeReadStrategy {
                inner: native
                    .responses_strategy(fallback, current_model.clone())
                    .unwrap(),
                policy: native,
            },
        ));
    }
    let selected = Arc::new(
        PreviousModelStrategy::compose(
            native
                .responses_strategy(old_transport.clone(), old_model.clone())
                .unwrap(),
            old_transport,
            native.binding(current_model.clone()),
            None,
            64,
            true,
        )
        .unwrap(),
    );
    execution.transport = Arc::new(Selected {
        inner: execution.transport.clone(),
        selected,
    });
    fixture
        .run_expecting(&next, execution, false, TaskCompletion::Failed)
        .await;
    let projection = fixture
        .service
        .open(research_agent_core::conversation::ConversationId::from_uuid(fixture.id))
        .await
        .unwrap();
    let turn = projection.turn(next.turn_id).unwrap();
    assert_eq!(turn.outcome, Some(TurnOutcome::Failed));
    assert!(
        turn.attempts
            .values()
            .all(|attempt| attempt.actual_model.is_none())
    );
    assert!(turn.items.values().all(|item| !matches!(
        item.kind,
        research_agent_core::protocol::ItemKind::AssistantMessage
    )));
    let errors: Vec<_> = fixture
        .events(next.turn_id)
        .into_iter()
        .filter_map(|event| match event.payload() {
            EventPayload::ModelRequestFailed { error, .. } => Some(error.clone()),
            _ => None,
        })
        .collect();
    let original_error = RunError::Model(if native == NativeCompactionPolicy::ChatGptResponsesV2 {
        ModelError::ContextWindowExceeded
    } else {
        ModelError::UnexpectedStatus(400)
    })
    .to_string();
    assert_eq!(errors.len(), 3, "{errors:?}"); // Two attempts plus the preparation diagnostic.
    assert_eq!(errors[0], original_error);
    assert_eq!(errors[2], original_error);
    if !invalid {
        let expected = if failure == AccountFailure::None {
            ModelError::HttpAuthentication(401)
        } else {
            ModelError::Authentication
        };
        assert_eq!(errors[1], RunError::Model(expected).to_string());
    }
    let retained = fixture
        .service
        .active_context_checkpoint(fixture.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::to_value(&retained).unwrap(),
        serde_json::to_value(&original).unwrap()
    );
    fixture.reopen().await;
    assert_eq!(
        serde_json::to_value(
            fixture
                .service
                .active_context_checkpoint(fixture.id)
                .await
                .unwrap()
                .unwrap()
        )
        .unwrap(),
        serde_json::to_value(&original).unwrap()
    );
    let old_requests = old_peer.await.unwrap();
    assert_eq!(old_requests.len(), 3);
    let fallback_requests = match current_peer {
        Ok(peer) => peer.await.unwrap(),
        Err(listener) => {
            assert!(
                tokio::time::timeout(std::time::Duration::from_millis(250), listener.accept())
                    .await
                    .is_err(),
                "invalid account authority must not send compact or sampling requests"
            );
            return;
        }
    };
    assert_eq!(fallback_requests.len(), 1);
    assert!(
        fallback_requests[0]["input"]
            .as_array()
            .unwrap()
            .contains(&item)
    );
    assert!(
        !fallback_requests[0]
            .to_string()
            .contains("fresh submission")
    );
}
