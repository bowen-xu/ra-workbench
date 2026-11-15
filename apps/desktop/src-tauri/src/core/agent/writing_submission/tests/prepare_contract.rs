use super::*;

#[tokio::test]
async fn bound_source_and_model_become_one_durable_submission_with_idempotent_retry() {
    let f = fixture(true);
    let service = WritingSubmissionService::from_app_store(&f.store);
    let id = Uuid::new_v4();
    let prepared = service.prepare(&f.models, request(&f, id)).await.unwrap();
    assert_eq!(prepared.record.input.context[0].text, "Frozen manuscript");
    assert_eq!(prepared.record.input.model, *prepared.model.identity());
    assert_eq!(prepared.record.input.model.model_id, "fixture-model");
    let retry = service.prepare(&f.models, request(&f, id)).await.unwrap();
    assert_eq!(retry.record, prepared.record);
    let saved = SqliteSubmissionStore::new(f.store.shared_agent_connection())
        .get(f.conversation, id)
        .await
        .unwrap();
    assert_eq!(saved, prepared.record);
    assert_eq!(count(&f), 1);
    let mut changed = request(&f, id);
    changed.text = "Different".into();
    assert!(matches!(
        service.prepare(&f.models, changed).await,
        Err(WritingSubmissionError::Storage(cause)) if cause.as_inner().code()=="DATA_SQL_OPERATION_CONFLICT"
            && cause.as_inner().diagnostic().is_some_and(|detail|detail.operation_id==id && detail.safe_cause=="operation_id_content_changed")
    ));
    assert_eq!(count(&f), 1);
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row("select count(*) from agent_items", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn writing_submission_consumes_a_manuscript_scoped_global_retrieval_trace() {
    let f = fixture(true);
    let query = "find related concepts";
    let context = "[CN-S1] retrieved context";
    let trace_id = "writing-trace-1";
    f.store
        .connection()
        .unwrap()
        .execute(
            "insert into concept_retrieval_traces
             (id, paper_id, query_sha256, status, config_json, context_text, context_sha256,
              seed_count, edge_count, source_count, external_paper_count, created_at, expires_at)
             values (?1, null, ?2, 'previewed', '{}', ?3, ?4, 0, 0, 0, 0,
                     strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                     strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '+15 minutes'))",
            params![
                trace_id,
                format!("{:x}", Sha256::digest(query.as_bytes())),
                context,
                format!("{:x}", Sha256::digest(context.as_bytes())),
            ],
        )
        .unwrap();
    let mut input = request(&f, Uuid::new_v4());
    input.retrieval = Some(crate::agent::retrieval_context::RetrievalSelection {
        trace_id: trace_id.into(),
        query: query.into(),
    });
    let prepared = WritingSubmissionService::from_app_store(&f.store)
        .prepare(&f.models, input)
        .await
        .unwrap();
    let retrieval = prepared
        .record
        .input
        .context
        .iter()
        .find(|source| source.kind == research_agent_core::submission::ContextSourceKind::Retrieval)
        .unwrap();
    assert_eq!(retrieval.resource_id, "global-research");
    assert_eq!(retrieval.text, context);
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row(
                "select status from concept_retrieval_traces where id=?1",
                [trace_id],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "sent"
    );
}
#[tokio::test]
async fn unbound_or_archived_conversation_fails_before_credentials_and_disk_reads() {
    let mut f = fixture(false);
    f.models = AgentModelResolver::from_app_store(
        &f.store,
        JsonCredentialStore::from_path(f.root.path().to_owned()),
    );
    let service = WritingSubmissionService::from_app_store(&f.store);
    assert!(matches!(
        service
            .prepare(&f.models, request(&f, Uuid::new_v4()))
            .await,
        Err(WritingSubmissionError::Storage(cause)) if matches!(cause.as_inner(),crate::data_interface::SqlImmediateFailure::Invalid(detail) if detail.safe_cause=="writing_binding_unavailable" && detail.phase=="writing-prepare-materialize")
    ));
    f.store.connection().unwrap().execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values (?1,'manuscript',?2)",params![f.conversation.to_string(),f.manuscript.to_string()]).unwrap();
    f.store
        .connection()
        .unwrap()
        .execute("update agent_conversations set archived=1", [])
        .unwrap();
    assert!(matches!(
        service
            .prepare(&f.models, request(&f, Uuid::new_v4()))
            .await,
        Err(WritingSubmissionError::Storage(cause)) if matches!(cause.as_inner(),crate::data_interface::SqlImmediateFailure::Invalid(detail) if detail.safe_cause=="writing_binding_unavailable" && detail.phase=="writing-prepare-materialize")
    ));
    assert_eq!(count(&f), 0);
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row(
                "select count(*) from data_sql_operation_receipts",
                [],
                |row| row.get::<_, u64>(0)
            )
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn fresh_binding_rejection_preserves_full_public_identity_for_ordinary_and_manual_preparation()
 {
    for compact in [false, true] {
        for archived in [false, true] {
            let f = fixture(archived);
            if archived {
                f.store
                    .connection()
                    .unwrap()
                    .execute("update agent_conversations set archived=1", [])
                    .unwrap();
            }
            let service = WritingSubmissionService::from_app_store(&f.store);
            let operation = Uuid::new_v4();
            let mut input = request(&f, operation);
            if compact {
                input.text = "/compact".into();
            }
            let wire = input.wire_request();
            let error = if compact {
                crate::agent::compaction_commands::preparation::prepare(
                    &service,
                    &f.models,
                    crate::agent::compaction_commands::preparation::Prepare {
                        request: wire,
                        expected_revision: 0,
                    },
                )
                .await
                .err()
                .unwrap()
            } else {
                crate::agent::writing_commands::prepare(&service, &f.models, wire)
                    .await
                    .err()
                    .unwrap()
            };
            let value = serde_json::to_value(error).unwrap();
            assert_eq!(value["code"], "storage");
            let reason = &value["reason"];
            assert_eq!(reason["conversationId"], f.conversation.to_string());
            assert_eq!(reason["manuscriptId"], f.manuscript.to_string());
            assert_eq!(reason["requestId"], operation.to_string());
            assert_eq!(reason["operationId"], operation.to_string());
            assert_eq!(reason["code"], "DATA_SQL_INVALID");
            assert_eq!(reason["reason"]["operationId"], operation.to_string());
            assert_eq!(reason["reason"]["phase"], "writing-prepare-materialize");
            assert_eq!(reason["reason"]["safeCause"], "writing_binding_unavailable");
            assert_eq!(count(&f), 0);
            assert_eq!(
                f.store
                    .connection()
                    .unwrap()
                    .query_row(
                        "select count(*) from data_sql_operation_receipts",
                        [],
                        |row| row.get::<_, u64>(0)
                    )
                    .unwrap(),
                0
            );
        }
    }
}
#[tokio::test]
async fn context_conflict_or_insert_failure_leaves_no_partial_submission() {
    let f = fixture(true);
    let service = WritingSubmissionService::from_app_store(&f.store);
    let mut stale = request(&f, Uuid::new_v4());
    let stale_id = stale.request_id;
    stale.context.files[0].expected_disk_sha256 = "0".repeat(64);
    assert!(matches!(
        service.prepare(&f.models, stale).await,
        Err(WritingSubmissionError::Storage(cause)) if cause.as_inner().code()=="DATA_SQL_OPERATION_CONFLICT"
            && cause.as_inner().diagnostic().is_some_and(|detail|detail.operation_id==stale_id && detail.safe_cause=="writing_context_changed")
    ));
    assert_eq!(count(&f), 0);
    f.store.connection().unwrap().execute_batch("create trigger fail_submission before insert on agent_submissions begin select raise(abort,'fixture'); end;").unwrap();
    assert!(matches!(
        service
            .prepare(&f.models, request(&f, Uuid::new_v4()))
            .await,
        Err(WritingSubmissionError::Storage(cause)) if cause.as_inner().code()=="DATA_SQL_UNAVAILABLE"
            && cause.as_inner().diagnostic().is_some_and(|detail|detail.phase=="writing-prepare-record" && detail.safe_cause=="sql_registration_failed" && detail.sqlite_code.as_deref()==Some("ConstraintViolation") && detail.sqlite_extended_code==Some(rusqlite::ffi::SQLITE_CONSTRAINT_TRIGGER))
    ));
    assert_eq!(count(&f), 0);
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row(
                "select count(*) from data_sql_operation_receipts",
                [],
                |row| row.get::<_, u64>(0)
            )
            .unwrap(),
        0
    );
}
