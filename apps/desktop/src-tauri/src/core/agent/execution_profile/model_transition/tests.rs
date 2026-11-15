use super::super::super::repository::context_policy::ContextAuthentication;
use super::*;

#[test]
fn previous_model_preparation_keeps_authority_and_original_transport_failures() {
    for error in [
        AgentModelPreparationError::Credentials,
        AgentModelPreparationError::ChatGptTransportUnavailable,
    ] {
        assert!(matches!(
            preparation_error(error),
            ModelError::Authentication
        ));
    }
    for error in [
        AgentModelPreparationError::Disabled,
        AgentModelPreparationError::UnsupportedProtocol,
    ] {
        assert!(matches!(
            preparation_error(error),
            ModelError::UnsupportedCapability
        ));
    }
    for error in [
        AgentModelPreparationError::Settings,
        AgentModelPreparationError::Worker,
    ] {
        assert!(matches!(preparation_error(error), ModelError::Unavailable));
    }
    macro_rules! preserves {
        ($error:pat => $value:expr) => {
            assert!(matches!(
                preparation_error(AgentModelPreparationError::Transport($value)),
                $error
            ));
        };
    }
    preserves!(ModelError::Authentication => ModelError::Authentication);
    preserves!(ModelError::Canceled => ModelError::Canceled);
    preserves!(ModelError::UnexpectedStatus(422) => ModelError::UnexpectedStatus(422));
    preserves!(ModelError::ContextWindowExceeded => ModelError::ContextWindowExceeded);
    preserves!(ModelError::UsageLimitReached => ModelError::UsageLimitReached);
    preserves!(ModelError::ServerOverloaded => ModelError::ServerOverloaded);
    preserves!(ModelError::InternalServerError => ModelError::InternalServerError);
    preserves!(ModelError::InvalidStream => ModelError::InvalidStream);
}

fn policy(model: &str, window: Option<u64>) -> FrozenContextPolicy {
    FrozenContextPolicy {
        tool_truncation_policy: None,
        reasoning_summary: None,
        chat_developer_model: None,
        compact_prompt: None,
        token_budget_settings: None,
        schema_version: 1,
        binding: CompactionBinding {
            model: ModelIdentity {
                provider_id: "fixture".into(),
                model_id: model.into(),
                revision: None,
            },
            strategy_id: "research.text.v1".into(),
            state_schema: 1,
            kind: CompactionKind::TextSummary,
        },
        budget: window.map(|context_window_tokens| CompactionBudget {
            context_window_tokens,
            effective_percent: 95,
            configured_trigger_tokens: Some(500),
            output_reserve_tokens: 100,
        }),
        max_output_tokens: 100,
        scope: AutoCompactionScope::Total,
        unknown_window_trigger_tokens: Some(500),
        comp_hash: None,
        authentication: ContextAuthentication::ApiKey,
    }
}

#[test]
fn hash_changes_require_both_declared_hashes_and_do_not_need_size_pressure() {
    let mut previous = policy("old", None);
    let mut current = policy("new", None);
    previous.comp_hash = Some("a".into());
    assert!(!needs_old_model(&previous, &current, Some(u64::MAX)).unwrap());
    current.comp_hash = Some("a".into());
    assert!(!needs_old_model(&previous, &current, None).unwrap());
    current.comp_hash = Some("b".into());
    assert!(needs_old_model(&previous, &current, None).unwrap());
    current.binding.model = previous.binding.model.clone();
    assert!(needs_old_model(&previous, &current, Some(1)).unwrap());
    previous.comp_hash = None;
    assert!(!needs_old_model(&previous, &current, None).unwrap());
}

#[test]
fn downgrade_uses_new_scope_and_never_guesses_unknown_windows_or_usage() {
    let previous = policy("old", Some(2000));
    let mut current = policy("new", Some(1000));
    assert!(!needs_old_model(&previous, &current, Some(499)).unwrap());
    assert!(!needs_old_model(&previous, &current, Some(500)).unwrap());
    assert!(needs_old_model(&previous, &current, Some(501)).unwrap());
    assert!(!needs_old_model(&previous, &current, None).unwrap());
    current.scope = AutoCompactionScope::BodyAfterPrefix;
    assert!(!needs_old_model(&previous, &current, Some(500)).unwrap());
    assert!(!needs_old_model(&previous, &current, Some(949)).unwrap());
    assert!(needs_old_model(&previous, &current, Some(950)).unwrap());
    assert!(!needs_old_model(&policy("old", None), &current, Some(u64::MAX)).unwrap());
    assert!(!needs_old_model(&previous, &policy("new", None), Some(u64::MAX)).unwrap());
    assert!(!needs_old_model(&previous, &policy("new", Some(2000)), Some(u64::MAX)).unwrap());
    assert!(!needs_old_model(&previous, &policy("new", Some(3000)), Some(u64::MAX)).unwrap());
    current.binding.model = previous.binding.model.clone();
    assert!(!needs_old_model(&previous, &current, Some(u64::MAX)).unwrap());
}

#[test]
fn model_revision_alone_does_not_trigger_downshift_without_a_hash_change() {
    let mut previous = policy("same-model", Some(2000));
    let mut current = policy("same-model", Some(1000));
    previous.binding.model.revision = Some("old".into());
    current.binding.model.revision = Some("new".into());
    assert!(!needs_old_model(&previous, &current, Some(u64::MAX)).unwrap());
    previous.comp_hash = Some("old-hash".into());
    current.comp_hash = Some("new-hash".into());
    assert!(needs_old_model(&previous, &current, None).unwrap());
}

#[test]
fn previous_model_threshold_uses_codex_window_and_strict_auto_limit() {
    let previous = policy("old", Some(2000));
    let mut current = policy("new", Some(1000));
    current.budget.as_mut().unwrap().configured_trigger_tokens = None;
    assert!(!needs_old_model(&previous, &current, Some(850)).unwrap());
    assert!(!needs_old_model(&previous, &current, Some(900)).unwrap());
    assert!(needs_old_model(&previous, &current, Some(901)).unwrap());
    current.budget.as_mut().unwrap().effective_percent = 80;
    assert!(!needs_old_model(&previous, &current, Some(799)).unwrap());
    assert!(needs_old_model(&previous, &current, Some(800)).unwrap());
    current.scope = AutoCompactionScope::BodyAfterPrefix;
    assert!(!needs_old_model(&previous, &current, Some(799)).unwrap());
    assert!(needs_old_model(&previous, &current, Some(800)).unwrap());
}

#[test]
fn same_model_name_with_changed_provider_or_revision_does_not_trigger_downshift() {
    let previous = policy("shared-model-name", Some(2000));
    for change_provider in [false, true] {
        let mut current = policy("shared-model-name", Some(1000));
        if change_provider {
            current.binding.model.provider_id = "another-provider".into();
        } else {
            current.binding.model.revision = Some("new-revision".into());
        }
        assert!(!needs_old_model(&previous, &current, Some(499)).unwrap());
        assert!(!needs_old_model(&previous, &current, Some(u64::MAX)).unwrap());
        // Identity changes alone never fabricate a capacity or usage estimate.
        assert!(!needs_old_model(&previous, &current, None).unwrap());
        current.budget = None;
        assert!(!needs_old_model(&previous, &current, Some(u64::MAX)).unwrap());
    }
    let mut unchanged = previous.clone();
    unchanged.budget = policy("shared-model-name", Some(1000)).budget;
    assert!(!needs_old_model(&previous, &unchanged, Some(u64::MAX)).unwrap());
}

#[tokio::test]
async fn native_selection_reads_the_exact_previous_snapshot_and_reauthorizes_its_model() {
    use crate::agent::repository::{SqliteSubmissionStore, create_conversation};
    use research_agent_core::{conversation::ConversationId, submission::*};
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("transition.sqlite");
    let mut db = rusqlite::Connection::open(&database).unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let conversation = uuid::Uuid::new_v4();
    create_conversation(
        &mut db,
        ConversationId::from_uuid(conversation),
        "Transition",
    )
    .unwrap();
    db.execute("insert into llm_provider_configs(provider_id,enabled,base_url,timeout_ms,updated_at) values ('deepseek',1,'',60000,'now') on conflict(provider_id) do update set enabled=1,base_url=''", []).unwrap();
    let store = AppStore::for_test(db, root.path().join("objects"));
    let credentials = crate::ai::JsonCredentialStore::from_path(root.path().join("synthetic.json"));
    credentials
        .set("deepseek", "synthetic-transition-fixture")
        .unwrap();
    let models = AgentModelResolver::from_app_store(&store, credentials.clone());
    let submissions = SqliteSubmissionStore::new(store.shared_agent_connection());
    let mut policies = vec![];
    let mut records = vec![];
    for (model, window) in [("deepseek-v4-pro", 2000), ("deepseek-v4-flash", 1000)] {
        let mut frozen = policy(model, Some(window));
        frozen.binding.model.provider_id = "deepseek".into();
        if model == "deepseek-v4-pro" {
            frozen.chat_developer_model = Some(frozen.binding.model.clone());
        }
        let input = SubmissionInput {
            remote_compaction_v2: None,
            user_input: vec![],
            skill_instructions: vec![],
            instructions: vec![],
            instruction_configs: vec![],
            skills: vec![],
            text: "Synthetic input".into(),
            model: frozen.binding.model.clone(),
            reasoning_effort: None,
            service_tier: None,
            context: vec![],
        };
        let record = submissions
            .put(conversation, uuid::Uuid::new_v4(), input)
            .await
            .unwrap();
        let saved =
            SqliteContextPolicy::new(store.shared_agent_connection(), &record, frozen).unwrap();
        saved.prepare().await.unwrap();
        policies.push(saved);
        records.push(record);
    }
    let history = ModelHistory {
        store,
        models,
        host: AgentHost::Writing,
    };
    let old = &records[0];
    assert!(
        history
            .select(&policies[1], old.turn_id, &old.input.model, Some(499))
            .await
            .unwrap()
            .is_none()
    );
    let selected = history
        .select(&policies[1], old.turn_id, &old.input.model, Some(501))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(selected.execution_model(), old.input.model);
    assert_eq!(selected.binding(), policies[1].snapshot().binding);
    credentials.delete("deepseek").unwrap();
    assert!(matches!(
        history
            .select(&policies[1], old.turn_id, &old.input.model, Some(501))
            .await,
        Err(ModelError::Authentication)
    ));
    credentials
        .set("deepseek", "synthetic-transition-fixture")
        .unwrap();
    assert!(
        history
            .select(
                &policies[1],
                uuid::Uuid::new_v4(),
                &old.input.model,
                Some(501)
            )
            .await
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        history
            .select(
                &policies[1],
                old.turn_id,
                &records[1].input.model,
                Some(501)
            )
            .await,
        Err(ModelError::InvalidStream)
    ));

    // Fork through the old completed turn, even though a newer submission is
    // present in the parent. Nested lineage must retain the exact old snapshot.
    use crate::agent::repository::{SqliteConversationCatalog, append_batch};
    use research_agent_core::{conversation::*, protocol::*};
    let item = uuid::Uuid::new_v4();
    let payloads = [
        EventPayload::TurnStarted {
            task_kind: TaskKind::Regular,
            turn_id: old.turn_id,
            context_snapshot_id: old.context_snapshot_id,
            model: old.input.model.clone(),
        },
        EventPayload::ItemStarted {
            turn_id: old.turn_id,
            item_id: item,
            kind: ItemKind::UserMessage,
        },
        EventPayload::TextDelta {
            turn_id: old.turn_id,
            item_id: item,
            text: old.input.text.clone(),
        },
        EventPayload::ItemCompleted {
            turn_id: old.turn_id,
            item_id: item,
        },
        EventPayload::TurnFinished {
            turn_id: old.turn_id,
            outcome: TurnOutcome::Completed,
        },
    ];
    let events = payloads
        .into_iter()
        .enumerate()
        .map(|(index, payload)| {
            AgentEvent::new(
                ConversationId::from_uuid(conversation),
                index as u64 + 1,
                1,
                old.request_id,
                payload,
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    {
        let mut db = history.store.connection().unwrap();
        db.execute("insert into note_pages(id,title,created_at,updated_at) values ('transition-note','Note','now','now')", []).unwrap();
        db.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values (?1,'note','transition-note')", [conversation.to_string()]).unwrap();
        append_batch(
            &mut db,
            ConversationId::from_uuid(conversation),
            uuid::Uuid::new_v4(),
            0,
            &events,
        )
        .unwrap();
    }
    let catalog = SqliteConversationCatalog::new(history.store.shared_agent_connection());
    let mut parent = conversation;
    for _ in 0..2 {
        let child = uuid::Uuid::new_v4();
        catalog
            .fork_legacy_for_tests(ConversationForkRequest {
                child_id: ConversationId::from_uuid(child),
                parent_id: ConversationId::from_uuid(parent),
                through_turn_id: old.turn_id,
                binding: ConversationBinding {
                    kind: BindingKind::Note,
                    resource_id: "transition-note".into(),
                },
            })
            .await
            .unwrap();
        parent = child;
    }
    let child_record = submissions
        .put(parent, uuid::Uuid::new_v4(), records[1].input.clone())
        .await
        .unwrap();
    let child_policy = SqliteContextPolicy::new(
        history.store.shared_agent_connection(),
        &child_record,
        policies[1].snapshot(),
    )
    .unwrap();
    child_policy.prepare().await.unwrap();
    let reopened = std::sync::Arc::new(std::sync::Mutex::new(
        rusqlite::Connection::open(&database).unwrap(),
    ));
    let restored =
        SqliteContextPolicy::new(reopened.clone(), &child_record, policies[1].snapshot()).unwrap();
    let (origin, frozen) = restored
        .previous(old.turn_id, &old.input.model)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(origin, *old);
    assert_eq!(frozen, policies[0].snapshot());
    let selected = history
        .select(&restored, old.turn_id, &old.input.model, Some(501))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(selected.execution_model(), old.input.model);
    assert_eq!(selected.binding(), child_policy.snapshot().binding);
    {
        let db = reopened.lock().unwrap();
        assert_eq!(
            db.query_row(
                "select count(*) from agent_submissions where conversation_id=?1",
                [parent.to_string()],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        db.execute("update agent_fork_turn_sources set context_snapshot_id=?1 where conversation_id=?2 and turn_id=?3", rusqlite::params![uuid::Uuid::new_v4().to_string(),parent.to_string(),old.turn_id.to_string()]).unwrap();
    }
    assert!(matches!(
        restored.previous(old.turn_id, &old.input.model).await,
        Err(ModelError::InvalidStream)
    ));
    {
        let db = reopened.lock().unwrap();
        db.execute("update agent_fork_turn_sources set context_snapshot_id=?1 where conversation_id=?2 and turn_id=?3", rusqlite::params![old.context_snapshot_id.to_string(),parent.to_string(),old.turn_id.to_string()]).unwrap();
        db.execute(
            "delete from agent_context_policies where turn_id=?1",
            [old.turn_id.to_string()],
        )
        .unwrap();
    }
    assert!(
        restored
            .previous(old.turn_id, &old.input.model)
            .await
            .unwrap()
            .is_none()
    );
}

#[path = "tests/factory_auth.rs"]
mod factory_auth;
#[path = "tests/remote.rs"]
mod remote;
#[path = "tests/sqlite_remote.rs"]
mod sqlite_remote;
#[path = "tests/task_remote.rs"]
mod task_remote;

#[test]
fn remote_handoff_requires_known_native_codecs_and_codex_authentication() {
    for native in [
        NativeCompactionPolicy::ChatGptResponsesV2,
        NativeCompactionPolicy::ChatGptResponsesFullWindow,
    ] {
        let mut previous = policy("old", Some(2000));
        previous.binding.model.provider_id = "openai".into();
        previous.binding = native.binding(previous.binding.model.clone());
        previous.authentication = ContextAuthentication::CodexBackend;
        let mut current = previous.clone();
        current.binding.model.model_id = "new".into();
        assert!(allows_handoff(&previous, &current));
        assert_eq!(frozen_native_policy(&previous.binding).unwrap(), native);
        let other = if native == NativeCompactionPolicy::ChatGptResponsesV2 {
            NativeCompactionPolicy::ChatGptResponsesFullWindow
        } else {
            NativeCompactionPolicy::ChatGptResponsesV2
        };
        let mut switched = current.clone();
        switched.binding = other.binding(current.binding.model.clone());
        assert!(allows_handoff(&previous, &switched));
        for alter in 0..5 {
            let mut rejected = current.clone();
            match alter {
                0 => rejected.authentication = ContextAuthentication::ApiKey,
                1 => rejected.binding.model.provider_id = "custom".into(),
                2 => rejected.binding.state_schema += 1,
                3 => rejected.binding.strategy_id = "foreign.codec".into(),
                _ => rejected.binding.kind = CompactionKind::TextSummary,
            }
            assert!(!allows_handoff(&previous, &rejected));
        }
        previous.authentication = ContextAuthentication::ApiKey;
        assert!(!allows_handoff(&previous, &current));
    }
}
