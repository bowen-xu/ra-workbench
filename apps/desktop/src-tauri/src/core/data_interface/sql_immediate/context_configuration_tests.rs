use super::*;
use crate::data_interface::SharedDatabase;
use rusqlite::Connection;
use serde_json::json;
use std::path::Path;

fn open(root: &Path) -> SharedDatabase {
    let mut sql = Connection::open(root.join("configuration.sqlite")).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    Database::open(sql, &root.join("log")).unwrap()
}

fn fixture() -> (
    tempfile::TempDir,
    SharedDatabase,
    ContextConfigurationIntent,
) {
    let root = tempfile::tempdir().unwrap();
    let db = open(root.path());
    let conversation_id = Uuid::new_v4();
    db.lock()
        .unwrap()
        .execute(
            "insert into agent_conversations(id,title) values(?1,'fixture')",
            [conversation_id.to_string()],
        )
        .unwrap();
    let intent = ContextConfigurationIntent {
        operation_id: Uuid::new_v4(),
        conversation_id,
        expected_epoch: db.benchmark_snapshot().unwrap().epoch,
        expected_revision: 0,
        state: json!({"schemaVersion":1,"persistent":[],"temporary":[]}),
    };
    (root, db, intent)
}

fn internal_seed_fixture() -> (
    tempfile::TempDir,
    SharedDatabase,
    ContextConfigurationSeedIntent,
) {
    let (root, db, save) = fixture();
    let run = Uuid::new_v4().to_string();
    let sql = db.lock().unwrap();
    sql.execute("insert into bibliographic_items(id,item_type,title,created_at,updated_at) values('seed-paper','journalArticle','Synthetic','now','now')", []).unwrap();
    // Simulate an older database missing the automatically allocated resources.
    sql.execute("delete from structured_resources where paper_id='seed-paper' and kind in ('portrait','structuredReading')", []).unwrap();
    sql.execute("insert into ai_runs (id,task_kind,status,provider_id,model_id,pipeline_version,prompt_version,output_schema_version,user_goal,scope_hash,input_item_count,max_input_tokens,max_output_tokens,timeout_ms,created_at,updated_at) values(?1,'paper_overview','generating','fixture-generation','fixture-v1','test','test',1,'',?2,1,100,100,1000,'now','now')", params![run,"a".repeat(64)]).unwrap();
    sql.execute("insert into ai_scope_items(id,run_id,ordinal,item_kind,object_id,parent_object_id,snapshot_hash,created_at) values(?1,?2,0,'paper','seed-paper','seed-paper',?3,'now')", params![Uuid::new_v4().to_string(),run,"b".repeat(64)]).unwrap();
    sql.execute("insert into agent_legacy_ai_run_bindings(ai_run_id,conversation_id,request_id,task_kind) values(?1,?2,?3,'paper_overview')", params![run,save.conversation_id.to_string(),Uuid::new_v4().to_string()]).unwrap();
    drop(sql);
    (
        root,
        db,
        ContextConfigurationSeedIntent {
            operation_id: Uuid::new_v4(),
            conversation_id: save.conversation_id,
            expected_epoch: save.expected_epoch,
        },
    )
}

#[test]
fn internal_seed_and_receipt_commit_together_and_replay_after_reopen() {
    let (root, db, intent) = internal_seed_fixture();
    assert!(matches!(
        db.seed_fixture_configuration_owned(&intent, super::super::Fault::AfterCommit),
        Err(ContextConfigurationFailure::Storage(
            SqlImmediateFailure::Unknown(_)
        ))
    ));
    assert!(matches!(
        db.context_configuration_seed_status(&intent),
        Ok(SqlImmediateStatus::Committed(_))
    ));
    drop(db);
    let reopened = open(root.path());
    assert!(
        reopened
            .seed_fixture_configuration(&intent)
            .unwrap()
            .replayed
    );
    let sql = reopened.lock().unwrap();
    let configuration = configuration::read_in(&sql, intent.conversation_id)
        .unwrap()
        .unwrap();
    assert_eq!(configuration.revision, 1);
    assert!(configuration.state["persistent"].as_array().unwrap().len() >= 2);
    assert!(crate::structured_resources::find_by_paper(&sql, "seed-paper", "portrait").is_ok());
    assert!(
        crate::structured_resources::find_by_paper(&sql, "seed-paper", "structuredReading").is_ok()
    );
}

#[test]
fn internal_seed_failure_rolls_back_resource_and_configuration_writes() {
    let (root, db, intent) = internal_seed_fixture();
    assert!(matches!(
        db.seed_fixture_configuration_owned(&intent, super::super::Fault::BeforeCommit),
        Err(ContextConfigurationFailure::Storage(
            SqlImmediateFailure::Unknown(_)
        ))
    ));
    drop(db);
    let reopened = open(root.path());
    assert!(matches!(
        reopened.context_configuration_seed_status(&intent),
        Ok(SqlImmediateStatus::NoSqlReceipt { .. })
    ));
    {
        let sql = reopened.lock().unwrap();
        assert!(
            configuration::read_in(&sql, intent.conversation_id)
                .unwrap()
                .is_none()
        );
        assert!(
            crate::structured_resources::find_by_paper(&sql, "seed-paper", "portrait").is_err()
        );
        assert!(
            crate::structured_resources::find_by_paper(&sql, "seed-paper", "structuredReading")
                .is_err()
        );
    }
    assert!(
        !reopened
            .seed_fixture_configuration(&intent)
            .unwrap()
            .replayed
    );
}

#[test]
fn internal_seed_does_not_replace_existing_empty_configuration() {
    let (_root, db, save) = fixture();
    db.save_context_configuration(
        &save,
        Some(crate::agent::writing_resources::test_sources::source()),
    )
    .unwrap();
    let intent = ContextConfigurationSeedIntent {
        operation_id: Uuid::new_v4(),
        conversation_id: save.conversation_id,
        expected_epoch: save.expected_epoch,
    };
    assert!(!db.seed_fixture_configuration(&intent).unwrap().replayed);
    assert!(db.seed_fixture_configuration(&intent).unwrap().replayed);
    let current = configuration::read_in(&db.lock().unwrap(), save.conversation_id)
        .unwrap()
        .unwrap();
    assert_eq!(current.revision, 1);
    assert_eq!(current.state, save.state);
    let wrong = ContextConfigurationSeedIntent {
        expected_epoch: Uuid::new_v4(),
        ..intent.clone()
    };
    assert!(matches!(
        db.context_configuration_seed_status(&wrong),
        Err(SqlImmediateFailure::Conflict(_))
    ));
}

#[test]
fn exact_replay_preserves_later_configuration_and_checks_original_content() {
    let (_root, db, first) = fixture();
    let proof = db
        .save_context_configuration(
            &first,
            Some(crate::agent::writing_resources::test_sources::source()),
        )
        .unwrap();
    let second = ContextConfigurationIntent {
        operation_id: Uuid::new_v4(),
        expected_revision: 1,
        state: json!({"schemaVersion":1,"persistent":[],"temporary":[]}),
        ..first.clone()
    };
    db.save_context_configuration(
        &second,
        Some(crate::agent::writing_resources::test_sources::source()),
    )
    .unwrap();
    assert!(
        db.save_context_configuration(
            &first,
            Some(crate::agent::writing_resources::test_sources::source())
        )
        .unwrap()
        .replayed
    );
    assert_eq!(
        configuration::read_in(&db.lock().unwrap(), first.conversation_id)
            .unwrap()
            .unwrap()
            .revision,
        2
    );
    let changed = ContextConfigurationIntent {
        expected_revision: 1,
        ..first.clone()
    };
    assert!(matches!(
        db.context_configuration_status(&changed),
        Err(SqlImmediateFailure::Conflict(_))
    ));
    assert!(
        matches!(db.context_configuration_status(&first), Ok(SqlImmediateStatus::Committed(receipt)) if receipt.operation_id==proof.operation_id)
    );
}

#[test]
fn unknown_commit_is_observed_and_reopen_replays_without_mutation() {
    let (root, db, intent) = fixture();
    assert!(matches!(
        db.save_context_configuration_owned(
            &intent,
            super::super::Fault::AfterCommit,
            Some(crate::agent::writing_resources::test_sources::source())
        ),
        Err(ContextConfigurationFailure::Storage(
            SqlImmediateFailure::Unknown(_)
        ))
    ));
    assert!(matches!(
        db.context_configuration_status(&intent),
        Ok(SqlImmediateStatus::Committed(_))
    ));
    assert!(db.lock().is_err());
    drop(db);
    let reopened = open(root.path());
    assert!(
        reopened
            .save_context_configuration(
                &intent,
                Some(crate::agent::writing_resources::test_sources::source())
            )
            .unwrap()
            .replayed
    );
    assert_eq!(
        configuration::read_in(&reopened.lock().unwrap(), intent.conversation_id)
            .unwrap()
            .unwrap()
            .revision,
        1
    );
}

#[test]
fn unknown_before_commit_keeps_original_intent_until_reopen() {
    let (root, db, intent) = fixture();
    assert!(matches!(
        db.save_context_configuration_owned(
            &intent,
            super::super::Fault::BeforeCommit,
            Some(crate::agent::writing_resources::test_sources::source())
        ),
        Err(ContextConfigurationFailure::Storage(
            SqlImmediateFailure::Unknown(_)
        ))
    ));
    assert!(matches!(
        db.context_configuration_status(&intent),
        Ok(SqlImmediateStatus::Unconfirmed { blocked: true, .. })
    ));
    let request = ContextConfigurationEnvironmentRequest {
        operation_id: intent.operation_id,
        conversation_id: intent.conversation_id,
    };
    assert!(matches!(
        db.context_configuration_environment(&request),
        Err(ContextConfigurationFailure::Storage(
            SqlImmediateFailure::RecoveryRequired(_)
        ))
    ));
    drop(db);
    let reopened = open(root.path());
    assert!(matches!(
        reopened.context_configuration_status(&intent),
        Ok(SqlImmediateStatus::NoSqlReceipt { .. })
    ));
    assert!(
        !reopened
            .save_context_configuration(
                &intent,
                Some(crate::agent::writing_resources::test_sources::source())
            )
            .unwrap()
            .replayed
    );
}

#[test]
fn incorrect_epoch_and_unrepresentable_revision_do_not_write() {
    let (_root, db, intent) = fixture();
    let wrong_epoch = ContextConfigurationIntent {
        expected_epoch: Uuid::new_v4(),
        ..intent.clone()
    };
    assert!(matches!(
        db.save_context_configuration(
            &wrong_epoch,
            Some(crate::agent::writing_resources::test_sources::source())
        ),
        Err(ContextConfigurationFailure::Storage(
            SqlImmediateFailure::Conflict(_)
        ))
    ));
    assert!(matches!(
        db.context_configuration_status(&wrong_epoch),
        Err(SqlImmediateFailure::Conflict(_))
    ));
    let too_large = ContextConfigurationIntent {
        expected_revision: 9_007_199_254_740_991,
        ..intent.clone()
    };
    assert!(matches!(
        db.save_context_configuration(
            &too_large,
            Some(crate::agent::writing_resources::test_sources::source())
        ),
        Err(ContextConfigurationFailure::Storage(
            SqlImmediateFailure::Invalid(_)
        ))
    ));
    assert!(
        configuration::read_in(&db.lock().unwrap(), intent.conversation_id)
            .unwrap()
            .is_none()
    );
}

#[test]
fn environment_read_cleanup_failure_fences_owner() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
    let (_root, db, intent) = fixture();
    db.lock()
        .unwrap()
        .authorizer(Some(|context: AuthContext<'_>| match context.action {
            AuthAction::Transaction {
                operation: TransactionOperation::Rollback,
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }));
    let request = ContextConfigurationEnvironmentRequest {
        operation_id: intent.operation_id,
        conversation_id: intent.conversation_id,
    };
    assert!(matches!(
        db.context_configuration_environment(&request),
        Err(ContextConfigurationFailure::Storage(
            SqlImmediateFailure::RecoveryRequired(_)
        ))
    ));
    let raw = db.raw.lock().unwrap();
    raw.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    if !raw.is_autocommit() {
        raw.execute_batch("rollback").unwrap();
    }
    drop(raw);
    assert!(db.lock().is_err());
}

#[test]
fn receipt_failure_rolls_back_and_business_conflict_does_not_block_owner() {
    let (_root, db, intent) = fixture();
    db.lock().unwrap().execute_batch("create trigger reject_configuration_receipt before insert on data_sql_operation_receipts when new.kind='agent.context.configuration.save' begin select raise(abort,'synthetic'); end;").unwrap();
    assert!(matches!(
        db.save_context_configuration(
            &intent,
            Some(crate::agent::writing_resources::test_sources::source())
        ),
        Err(ContextConfigurationFailure::Storage(
            SqlImmediateFailure::Unavailable(_)
        ))
    ));
    {
        let guard = db.lock().unwrap();
        assert!(
            configuration::read_in(&guard, intent.conversation_id)
                .unwrap()
                .is_none()
        );
        guard
            .execute_batch("drop trigger reject_configuration_receipt")
            .unwrap();
    }
    let wrong = ContextConfigurationIntent {
        expected_revision: 1,
        ..intent.clone()
    };
    assert!(matches!(
        db.save_context_configuration(
            &wrong,
            Some(crate::agent::writing_resources::test_sources::source())
        ),
        Err(ContextConfigurationFailure::Domain(
            Error::ConfirmationMismatch
        ))
    ));
    assert!(db.lock().is_ok());
    assert!(
        !db.save_context_configuration(
            &intent,
            Some(crate::agent::writing_resources::test_sources::source())
        )
        .unwrap()
        .replayed
    );
}

#[test]
fn manuscript_validation_uses_the_explicit_source_without_changing_receipt_identity() {
    let (_root, db, mut intent) = fixture();
    let manuscript = Uuid::new_v4();
    {
        let sql = db.lock().unwrap();
        sql.execute("insert into research_projects(id,title,summary,created_at) values ('resource-port-project','Fixture','','now')",[]).unwrap();
        sql.execute("insert into manuscripts(id,project_id,title,root_path,created_at,updated_at) values (?1,'resource-port-project','Fixture','main.tex','now','now')",[manuscript.to_string()]).unwrap();
    }
    intent.state = json!({"schemaVersion":1,"persistent":[{"label":"Writing","resource":{"kind":"manuscript","resourceId":manuscript.to_string(),"groupAccess":"readWrite","newMemberAccess":"readWrite","parts":[]}}],"temporary":[]});
    let identity = intent.identity().unwrap();
    assert!(matches!(
        db.save_context_configuration(&intent, None),
        Err(ContextConfigurationFailure::Storage(
            SqlImmediateFailure::Unavailable(_)
        ))
    ));
    assert!(
        configuration::read_in(&db.lock().unwrap(), intent.conversation_id)
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        db.context_configuration_status(&intent),
        Ok(SqlImmediateStatus::NoSqlReceipt { .. })
    ));
    let proof = db
        .save_context_configuration(
            &intent,
            Some(crate::agent::writing_resources::test_sources::source()),
        )
        .unwrap();
    assert!(!proof.replayed);
    assert!(
        db.save_context_configuration(
            &intent,
            Some(crate::agent::writing_resources::test_sources::source())
        )
        .unwrap()
        .replayed
    );
    assert_eq!(identity.fingerprint, intent.identity().unwrap().fingerprint);
    assert_eq!(
        configuration::read_in(&db.lock().unwrap(), intent.conversation_id)
            .unwrap()
            .unwrap()
            .state,
        intent.state
    );
}

#[path = "context_configuration_scope_tests.rs"]
mod scope_runtime_tests;

trait SeedFixture {
    fn seed_fixture_configuration(
        &self,
        intent: &ContextConfigurationSeedIntent,
    ) -> Result<SqlImmediateReceipt, ContextConfigurationFailure>;
    fn seed_fixture_configuration_owned(
        &self,
        intent: &ContextConfigurationSeedIntent,
        fault: super::super::Fault,
    ) -> Result<SqlImmediateReceipt, ContextConfigurationFailure>;
}
impl SeedFixture for Database {
    fn seed_fixture_configuration(
        &self,
        intent: &ContextConfigurationSeedIntent,
    ) -> Result<SqlImmediateReceipt, ContextConfigurationFailure> {
        self.seed_fixture_configuration_owned(intent, super::super::Fault::None)
    }
    fn seed_fixture_configuration_owned(
        &self,
        intent: &ContextConfigurationSeedIntent,
        fault: super::super::Fault,
    ) -> Result<SqlImmediateReceipt, ContextConfigurationFailure> {
        self.seed_context_configuration_with(intent, fault, |tx| {
            let configured: bool = tx.query_row(
                "select exists(select 1 from agent_context_configurations where conversation_id=?1)",
                [intent.conversation_id.to_string()],
                |row| row.get(0),
            )?;
            if configured {
                return Ok(());
            }
            crate::ai::test_paper_grants(
                tx,
                "seed-paper",
                &[],
                intent.conversation_id,
                "structuredReading",
                None,
                Some(crate::agent::writing_resources::test_sources::source()),
            )?;
            Ok(())
        })
    }
}
