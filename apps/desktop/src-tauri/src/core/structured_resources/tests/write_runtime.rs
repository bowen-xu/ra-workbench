use super::*;
use crate::{
    ai::{
        candidate_contracts::AiEvidenceBundle,
        error::AiError,
        structured_resource_validation::{
            DecisionOrigin, Failure, FailureCause, Flow, StructuredPolicy, ValidationSources,
        },
    },
    hbs_storage::{
        self,
        boundary::{BoundaryError, HbsNeed, HbsOutcome, HbsSources},
    },
};
use research_workbench_native_sdk::editable_template::EditableTemplateInputV1;

#[derive(Default)]
struct Policy {
    ready: bool,
    calls: usize,
    origin: bool,
    technical: bool,
    reject: bool,
}
impl StructuredPolicy for Policy {
    type Pause = HbsNeed;
    fn validate(
        &mut self,
        _: &str,
        _: serde_json::Value,
        _: &AiEvidenceBundle,
    ) -> Result<serde_json::Value, Flow<AiError, HbsNeed>> {
        panic!("template cannot consume overview validation")
    }
    fn validate_editable_template(
        &mut self,
        value: serde_json::Value,
    ) -> Result<serde_json::Value, Flow<AiError, HbsNeed>> {
        self.calls += 1;
        if !self.ready {
            return Err(Flow::Need(HbsNeed::EditableTemplate {
                slot: 0,
                input: EditableTemplateInputV1 {
                    candidate_json: value.to_string(),
                },
            }));
        }
        if self.technical {
            return Err(Flow::Fail(Failure {
                error: AiError::InvalidInput("来源读取尚未完成".into()),
                cause: FailureCause::SourceReadUnavailable,
            }));
        }
        if self.reject {
            return Err(Flow::Fail(Failure {
                error: AiError::InvalidInput("synthetic template domain rejection".into()),
                cause: if self.origin {
                    FailureCause::NativeRejection(Box::new(DecisionOrigin {
                        package_key: "fixture-package-key".into(),
                        instance_id: "fixture-instance".into(),
                        native_request_id: "fixture-native:1".into(),
                    }))
                } else {
                    FailureCause::Original
                },
            }));
        }
        Ok(value)
    }
}
#[derive(Default)]
struct Sources {
    policy: Policy,
    reading: usize,
}
impl HbsSources for Sources {
    type Policy = Policy;
    fn policy_only(&mut self) -> Result<&mut Policy, Flow<String, HbsNeed>> {
        Ok(&mut self.policy)
    }
    fn require(&mut self) -> Result<ValidationSources<'_, Policy>, Flow<String, HbsNeed>> {
        self.reading += 1;
        Err(Flow::Need(HbsNeed::Sources))
    }
}
fn target(db: &Connection) -> Resource {
    create_template(
        db,
        "runtime-template",
        "create-runtime-template",
        Some("test-conversation"),
    )
    .unwrap()
}
fn count(db: &Connection, table: &str, request: &str) -> i64 {
    let sql = match table {
        "intent" => "select count(*) from structured_resource_write_intents where request_id=?1",
        "operation" => "select count(*) from structured_resource_operations where request_id=?1",
        "receipt" => "select count(*) from structured_resource_receipts where request_id=?1",
        _ => panic!("closed fixture table"),
    };
    db.query_row(sql, [request], |row| row.get(0)).unwrap()
}

#[test]
fn runtime_template_pause_preserves_durable_raw_and_exact_intent_then_publishes_once() {
    let mut db = database();
    let target = target(&db);
    let input = draft(&target.id, &template(), "runtime-template-write");
    let mut sources = Sources::default();
    let result = write_with(&mut sources, &mut db, input.clone());
    let Err(BoundaryError::Pause(pause)) = result else {
        panic!("actual template pause")
    };
    assert_eq!(pause.outcome(), HbsOutcome::RolledBack);
    assert!(db.is_autocommit());
    assert!(matches!(pause.need(), HbsNeed::EditableTemplate { .. }));
    let staged = repository::get_draft(&db, &target.id, input.conversation_id.as_deref())
        .unwrap()
        .unwrap();
    assert_eq!(staged.content, input.content);
    assert_eq!(get(&db, &target.id).unwrap().revision, 0);
    assert_eq!(
        (
            count(&db, "intent", &input.request_id),
            count(&db, "operation", &input.request_id),
            count(&db, "receipt", &input.request_id)
        ),
        (1, 0, 0)
    );
    sources.policy.ready = true;
    let applied = write_with(&mut sources, &mut db, input.clone())
        .unwrap_or_else(|_| panic!("resume original intent"));
    assert_eq!(applied.status, "applied");
    assert_eq!(applied.resource.revision, 1);
    assert!(applied.validation_origin.is_none());
    assert_eq!(count(&db, "intent", &input.request_id), 1);
    assert_eq!(count(&db, "receipt", &input.request_id), 1);
    let calls = sources.policy.calls;
    sources.policy.ready = false;
    let replay = write_with(&mut sources, &mut db, input.clone())
        .unwrap_or_else(|_| panic!("operation replay before policy"));
    assert_eq!(encode(&replay).unwrap(), encode(&applied).unwrap());
    assert_eq!(calls, sources.policy.calls);
    assert_eq!(sources.reading, 0);
    let mut changed = input;
    changed.content.push(' ');
    assert!(matches!(
        write_with(&mut sources, &mut db, changed),
        Err(BoundaryError::Failure(_))
    ));
    assert_eq!(calls, sources.policy.calls);
}

#[test]
fn runtime_template_domain_invalid_origin_is_persisted_only_when_consumed() {
    let mut db = database();
    let target = target(&db);
    let input = draft(
        &target.id,
        r#"{"kind":"reading_template_v1","schemaVersion":1,"name":"invalid","fields":[]}"#,
        "invalid-native-template",
    );
    let mut sources = Sources {
        policy: Policy {
            ready: true,
            origin: true,
            reject: true,
            ..Policy::default()
        },
        ..Sources::default()
    };
    let invalid = write_with(&mut sources, &mut db, input.clone())
        .unwrap_or_else(|_| panic!("normal invalid result"));
    assert_eq!(invalid.status, "invalid");
    assert_eq!(invalid.errors[0].code, "invalid_content");
    assert_eq!(invalid.errors[0].path, "$");
    assert!(invalid.receipt.is_none());
    assert_eq!(invalid.draft.as_ref().unwrap().content, input.content);
    let origin = invalid.validation_origin.as_ref().unwrap();
    assert_eq!(origin.request_id, input.request_id);
    assert_eq!(origin.native_request_id, "fixture-native:1");
    let json = serde_json::to_value(&invalid).unwrap();
    assert_eq!(json["validationOrigin"].as_object().unwrap().len(), 4);
    let mut legacy = json.clone();
    legacy.as_object_mut().unwrap().remove("validationOrigin");
    let legacy: WriteResult = serde_json::from_value(legacy).unwrap();
    assert!(legacy.validation_origin.is_none());
    assert!(
        serde_json::to_value(legacy)
            .unwrap()
            .get("validationOrigin")
            .is_none()
    );
    sources.policy.ready = false;
    let calls = sources.policy.calls;
    let replay =
        write_with(&mut sources, &mut db, input).unwrap_or_else(|_| panic!("invalid replay"));
    assert_eq!(encode(&replay).unwrap(), encode(&invalid).unwrap());
    assert_eq!(calls, sources.policy.calls);
    assert_eq!(sources.reading, 0);
}

fn assert_uncertain_write_keeps_stage_and_fences_owner(deny_commit: bool) {
    use crate::data_interface::{Database, guarded_transaction};
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("uncertain-template.sqlite");
    let log = directory.path().join("log");
    let mut sql = Connection::open(&path).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    let target = target(&sql);
    let input = draft(&target.id, &template(), "uncertain-template-write");
    let database = Database::open(sql, &log).unwrap();
    let mut sources = Sources {
        policy: Policy {
            ready: deny_commit,
            ..Policy::default()
        },
        ..Sources::default()
    };
    {
        let commits = Arc::new(AtomicUsize::new(0));
        let mut guard = database.lock().unwrap();
        guard.authorizer(Some(move |context: AuthContext<'_>| match context.action {
            AuthAction::Transaction {
                // Pinned rusqlite 0.37 maps SQLite COMMIT/END to Unknown.
                operation: TransactionOperation::Unknown,
            } if deny_commit => {
                if commits.fetch_add(1, Ordering::SeqCst) == 0 {
                    Authorization::Allow
                } else {
                    Authorization::Deny
                }
            }
            AuthAction::Transaction {
                operation: TransactionOperation::Rollback,
            } if !deny_commit => Authorization::Deny,
            _ => Authorization::Allow,
        }));
        let result = guarded_transaction::run_mut(&mut guard, |db| {
            let result = write_with(&mut sources, db, input.clone());
            let recovery = !db.is_autocommit()
                || matches!(&result, Err(BoundaryError::Failure(failure)) if failure.needs_recovery());
            (result, recovery)
        });
        let Err(BoundaryError::Failure(failure)) = result else {
            panic!("uncertain settlement cannot return success or Need")
        };
        assert_eq!(failure.outcome, HbsOutcome::Unknown);
        assert_eq!(failure.rollback_error.is_some(), !deny_commit);
        assert!(!guard.is_autocommit());
    }
    assert!(database.lock().is_err());
    drop(database);
    let reopened = Database::open(Connection::open(&path).unwrap(), &log).unwrap();
    let guard = reopened.lock().unwrap();
    assert_eq!(get(&guard, &target.id).unwrap().revision, 0);
    let draft = repository::get_draft(&guard, &target.id, input.conversation_id.as_deref())
        .unwrap()
        .unwrap();
    assert_eq!(draft.content, input.content);
    assert_eq!(
        (
            count(&guard, "intent", &input.request_id),
            count(&guard, "operation", &input.request_id),
            count(&guard, "receipt", &input.request_id)
        ),
        (1, 0, 0)
    );
}

#[test]
fn runtime_template_denied_pause_rollback_preserves_durable_stage_and_blocks_owner() {
    assert_uncertain_write_keeps_stage_and_fences_owner(false);
}

#[test]
fn runtime_template_denied_publication_commit_preserves_durable_stage_and_blocks_owner() {
    assert_uncertain_write_keeps_stage_and_fences_owner(true);
}

#[test]
fn runtime_template_malformed_and_stale_results_do_not_wait_for_unused_policy() {
    let mut db = database();
    let target = target(&db);
    let mut sources = Sources::default();
    let bad = write_with(
        &mut sources,
        &mut db,
        draft(&target.id, "{invalid", "malformed-runtime"),
    )
    .unwrap_or_else(|_| panic!("JSON is original invalid"));
    assert_eq!(bad.errors[0].code, "invalid_json");
    assert!(bad.validation_origin.is_none());
    assert_eq!(sources.policy.calls, 0);
    let mut stale = draft(&target.id, &template(), "stale-runtime");
    stale.expected_revision = 19;
    let result =
        write_with(&mut sources, &mut db, stale).unwrap_or_else(|_| panic!("CAS precedes need"));
    assert_eq!(result.status, "conflict");
    assert_eq!(result.errors[0].code, "revision_conflict");
    assert!(result.validation_origin.is_none());
    assert_eq!(sources.reading, 0);
}

#[test]
fn runtime_template_technical_checked_does_not_become_domain_invalid_or_publication_fallback() {
    let mut db = database();
    let target = target(&db);
    let input = draft(&target.id, &template(), "technical-runtime");
    let mut sources = Sources {
        policy: Policy {
            ready: true,
            technical: true,
            ..Policy::default()
        },
        ..Sources::default()
    };
    let Err(BoundaryError::Failure(failure)) = write_with(&mut sources, &mut db, input.clone())
    else {
        panic!("typed technical failure")
    };
    assert!(matches!(
        failure.failure.cause,
        FailureCause::SourceReadUnavailable
    ));
    assert_eq!(failure.outcome, HbsOutcome::RolledBack);
    assert!(db.is_autocommit());
    assert_eq!(
        (
            count(&db, "intent", &input.request_id),
            count(&db, "operation", &input.request_id),
            count(&db, "receipt", &input.request_id)
        ),
        (1, 0, 0)
    );
    let staged = repository::get_draft(&db, &target.id, input.conversation_id.as_deref())
        .unwrap()
        .unwrap();
    assert_eq!(staged.content, input.content);
    assert!(staged.errors.is_empty());
}

#[test]
fn runtime_publication_sql_failure_reuses_original_durable_draft_fallback() {
    let mut db = database();
    let target = target(&db);
    let input = draft(&target.id, &template(), "runtime-sql-failure");
    db.execute_batch("create trigger reject_runtime_receipt before insert on structured_resource_receipts when new.request_id='runtime-sql-failure' begin select raise(abort,'PRIVATE_DRIVER_MESSAGE');end;").unwrap();
    let mut sources = Sources {
        policy: Policy {
            ready: true,
            ..Policy::default()
        },
        ..Sources::default()
    };
    let result = write_with(&mut sources, &mut db, input.clone())
        .unwrap_or_else(|_| panic!("confirmed rollback retains original invalid fallback"));
    assert_eq!(result.status, "invalid");
    assert_eq!(result.errors[0].code, "storage_error");
    assert!(result.receipt.is_none());
    assert!(result.validation_origin.is_none());
    assert_eq!(get(&db, &target.id).unwrap().revision, 0);
    assert_eq!(count(&db, "operation", &input.request_id), 1);
    assert_eq!(count(&db, "receipt", &input.request_id), 0);
    db.execute_batch("drop trigger reject_runtime_receipt")
        .unwrap();
    let mut retry = input;
    retry.request_id = "runtime-sql-retry".into();
    retry.expected_draft_revision = Some(result.draft.unwrap().revision);
    let applied =
        write_with(&mut sources, &mut db, retry).unwrap_or_else(|_| panic!("original draft retry"));
    assert_eq!(applied.status, "applied");
    assert_eq!(applied.resource.revision, 1);
}

#[test]
fn runtime_template_hbs_saved_baseline_and_preservation_use_policy_without_reading_sources() {
    let mut db = database();
    let target = target(&db);
    let mut sources = Sources {
        policy: Policy {
            ready: true,
            ..Policy::default()
        },
        ..Sources::default()
    };
    let mut input = draft(&target.id, &template(), "hbs-template-first");
    input.editor_kind = "human".into();
    let result = write_with(&mut sources, &mut db, input)
        .unwrap_or_else(|_| panic!("template creation write"));
    let content = result.resource.content.unwrap();
    let checkpoint = json!({"format":1,"kind":"structured","observed":{"version":1,"saved":content,"baseline":content},"content":content,"ancestor":content,"reviewAncestor":content,"baselineAncestor":content,"ordinary":[]});
    let saved = hbs_storage::preservation::save_with(
        &mut sources,
        &mut db,
        "structured",
        &target.id,
        checkpoint.clone(),
    )
    .unwrap_or_else(|_| panic!("template preservation"));
    let reopened =
        hbs_storage::preservation::read_with(&mut sources, &db, "structured", &target.id)
            .unwrap_or_else(|_| panic!("template preservation reopen"));
    assert_eq!(saved["revision"], 1);
    assert_eq!(reopened, checkpoint);
    assert_eq!(sources.reading, 0);
    assert_eq!(sources.policy.calls, 13);
    let request = hbs_storage::Commit {
        id: "template-hbs-commit".into(),
        expected_version: 1,
        saved: content.clone(),
        baseline: content,
    };
    let receipt =
        hbs_storage::commit_with(&mut sources, &mut db, "structured", &target.id, &request)
            .unwrap_or_else(|_| panic!("template saved/B commit"));
    assert_eq!(receipt["type"], "receipt");
    assert_eq!(sources.reading, 0);
}
