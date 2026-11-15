//! Real retrieval consumption must not hide an already committed preparation.
use super::*;

#[tokio::test]
async fn committed_retrieval_prepare_replays_original_request_after_its_reply_is_lost() {
    let f = managed_fixture(true);
    let query = "find original references";
    let context = "[CN-S1] original frozen retrieval context";
    let trace = "ordinary-prepare-lost-reply";
    f.store
        .connection()
        .unwrap()
        .execute(
            "insert into concept_retrieval_traces
        (id,paper_id,query_sha256,status,config_json,context_text,context_sha256,
         seed_count,edge_count,source_count,external_paper_count,created_at,expires_at)
        values(?1,null,?2,'previewed','{}',?3,?4,0,0,0,0,
        strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now','+15 minutes'))",
            params![
                trace,
                format!("{:x}", Sha256::digest(query.as_bytes())),
                context,
                format!("{:x}", Sha256::digest(context.as_bytes()))
            ],
        )
        .unwrap();
    let id = Uuid::new_v4();
    let original = || {
        let mut input = request(&f, id);
        input.retrieval = Some(crate::agent::retrieval_context::RetrievalSelection {
            trace_id: trace.into(),
            query: query.into(),
        });
        input
    };
    let service = WritingSubmissionService::from_app_store(&f.store);
    // The SQL commit is real; the caller deliberately does not use its reply.
    // No second request or replacement trace is introduced to recover it.
    service.prepare(&f.models, original()).await.unwrap();
    let saved = SqliteSubmissionStore::new(f.store.shared_agent_connection())
        .get(f.conversation, id)
        .await
        .unwrap();
    let replay = service.prepare(&f.models, original()).await.unwrap();
    assert_eq!(replay.record, saved);
    assert_eq!(count(&f), 1);
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row(
                "select status from concept_retrieval_traces where id=?1",
                [trace],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "sent"
    );
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row(
                "select count(*) from agent_items where conversation_id=?1",
                [f.conversation.to_string()],
                |row| row.get::<_, u64>(0),
            )
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn command_replay_and_status_read_original_fact_after_source_and_model_are_unavailable() {
    let f = managed_fixture(true);
    let service = WritingSubmissionService::from_app_store(&f.store);
    let wire = request(&f, Uuid::new_v4()).wire_request();
    let original = crate::agent::writing_commands::prepare(&service, &f.models, wire.clone())
        .await
        .unwrap();
    f.store
        .connection()
        .unwrap()
        .execute(
            "update llm_provider_configs set enabled=0 where provider_id='deepseek'",
            [],
        )
        .unwrap();
    std::fs::remove_file(
        f.store
            .manuscript_directory()
            .join(f.manuscript.to_string())
            .join("source/main.tex"),
    )
    .unwrap();
    let replay = crate::agent::writing_commands::prepare(&service, &f.models, wire.clone())
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&replay).unwrap(),
        serde_json::to_value(&original).unwrap()
    );
    let status = serde_json::to_value(
        crate::agent::writing_commands::prepare_status(&service, wire.clone())
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(status["intent"], serde_json::to_value(wire).unwrap());
    assert_eq!(status["status"]["type"], "committed");
    assert_eq!(
        status["status"]["result"],
        serde_json::to_value(original).unwrap()
    );
    assert_eq!(status["status"]["proof"]["sqlFence"], "0");
    assert_eq!(status["status"]["proof"]["result"], serde_json::Value::Null);
    assert_eq!(count(&f), 1);
}

#[tokio::test]
async fn schema_123_legacy_restore_keeps_original_input_without_fabricating_a_prepare_proof() {
    let f = legacy_fixture(true);
    let input = request(&f, Uuid::new_v4());
    let id = input.request_id;
    let model = f.models.prepare(AgentHost::Writing, None).await.unwrap();
    let record = {
        let mut db = f.store.connection().unwrap();
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        let frozen = materialize_submission_in(
            &tx,
            &f.store,
            input,
            FrozenWritingModel {
                identity: model.identity().clone(),
                reasoning_effort: model.reasoning_effort().map(str::to_owned),
                service_tier: model.service_tier().map(str::to_owned),
            },
        )
        .unwrap();
        let record =
            crate::agent::repository::submissions::insert_in(&tx, f.conversation, id, frozen)
                .unwrap();
        tx.commit().unwrap();
        record
    };
    std::fs::remove_file(
        f.store
            .manuscript_directory()
            .join(f.manuscript.to_string())
            .join("source/main.tex"),
    )
    .unwrap();
    let restored = WritingSubmissionService::from_app_store(&f.store)
        .restore(
            &f.models,
            WritingSubmissionReceipt {
                conversation_id: f.conversation,
                request_id: id,
                manuscript_id: f.manuscript,
            },
        )
        .await
        .unwrap();
    assert_eq!(restored.record, record);
    let db = f.store.connection().unwrap();
    assert_eq!(
        db.query_row("select max(version) from schema_migrations", [], |row| row
            .get::<_, u64>(
            0
        ))
        .unwrap(),
        123
    );
    assert!(
        !db.query_row(
            "select exists(select 1 from sqlite_schema where name='data_sql_operation_receipts')",
            [],
            |row| row.get::<_, bool>(0)
        )
        .unwrap()
    );
}

#[tokio::test]
async fn native_serde_prepare_status_examples_include_optional_fields_supplemental_grants_and_safe_diagnostics()
 {
    let mut examples = serde_json::Map::new();
    for compact in [false, true] {
        let f = managed_fixture(true);
        let note = Uuid::new_v4();
        f.store.connection().unwrap().execute("insert into note_pages(id,title,created_at,updated_at) values(?1,'Synthetic supplemental note','now','now')",[note.to_string()]).unwrap();
        let mut request = request(&f, Uuid::new_v4());
        if compact {
            request.text = "/compact".into();
        }
        request.resources =
            vec![crate::agent::context_session::sources::Resource::Note { resource_id: note }];
        let wire = request.wire_request();
        let service = WritingSubmissionService::from_app_store(&f.store);
        let output = if compact {
            let input = crate::agent::compaction_commands::preparation::Prepare {
                request: wire.clone(),
                expected_revision: 0,
            };
            crate::agent::compaction_commands::preparation::prepare(
                &service,
                &f.models,
                input.clone(),
            )
            .await
            .unwrap();
            serde_json::to_value(
                crate::agent::compaction_commands::preparation::prepare_status(&service, input)
                    .await
                    .unwrap(),
            )
            .unwrap()
        } else {
            crate::agent::writing_commands::prepare(&service, &f.models, wire.clone())
                .await
                .unwrap();
            serde_json::to_value(
                crate::agent::writing_commands::prepare_status(&service, wire.clone())
                    .await
                    .unwrap(),
            )
            .unwrap()
        };
        let result = if compact {
            &output["status"]["result"]["preview"]
        } else {
            &output["status"]["result"]
        };
        assert!(
            result["input"]["context"]
                .as_array()
                .unwrap()
                .iter()
                .any(|source| source["kind"] == "note"
                    && source["anchor"]
                        .as_str()
                        .unwrap()
                        .contains("supplementalContext"))
        );
        assert_eq!(output["status"]["proof"]["result"], serde_json::Value::Null);
        examples.insert(if compact { "compact" } else { "ordinary" }.into(), output);
        if !compact {
            f.store
                .connection()
                .unwrap()
                .execute(
                    "delete from data_sql_operation_receipts where operation_id=?1",
                    [wire.request_id.to_string()],
                )
                .unwrap();
            let unconfirmed = serde_json::to_value(
                crate::agent::writing_commands::prepare_status(&service, wire.clone())
                    .await
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(unconfirmed["status"]["type"], "unconfirmed");
            assert_eq!(
                unconfirmed["status"]["detail"].as_object().unwrap().len(),
                7
            );
            examples.insert("unconfirmed".into(), unconfirmed);
            f.store
                .connection()
                .unwrap()
                .execute_batch(
                    "alter table data_runtime_state rename to synthetic_unavailable_prepare_state",
                )
                .unwrap();
            let failed = crate::agent::writing_commands::prepare_status(&service, wire)
                .await
                .err()
                .unwrap();
            let failure = serde_json::to_value(failed).unwrap();
            assert_eq!(failure["reason"]["reason"]["sqliteCode"], "Unknown");
            assert_eq!(failure["reason"]["reason"]["sqliteExtendedCode"], 1);
            assert!(
                !failure
                    .to_string()
                    .contains("synthetic_unavailable_prepare_state")
            );
            examples.insert("storageFailure".into(), failure);
        }
    }
    // Capture the actual public serializer output for the TS contract fixture;
    // all material here is local synthetic data, never a provider request.
    println!(
        "WRITING_PREPARE_NATIVE_GOLDEN={}",
        serde_json::Value::Object(examples)
    );
}
