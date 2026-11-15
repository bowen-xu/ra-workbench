use super::*;
#[path = "tests/context.rs"]
mod context;
#[path = "tests/deletion.rs"]
mod deletion;
#[path = "tests/ordering.rs"]
mod ordering;
#[path = "../writing_start_status/tests.rs"]
mod start_status;
use crate::{
    agent::{
        WritingContextFile, WritingContextRequest,
        writing_commands::{ModelSelection, PrepareRequest},
    },
    storage::AppStore,
};
use rusqlite::params;
use sha2::{Digest, Sha256};

struct Fixture {
    root: tempfile::TempDir,
    db: crate::data_interface::SharedDatabase,
    store: AppStore,
    intent: WritingPrepareIntent,
}
impl Fixture {
    fn new(compact: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let mut sql = Connection::open(root.path().join("prepare.sqlite3")).unwrap();
        crate::storage::migrate(&mut sql).unwrap();
        let c = Uuid::new_v4();
        let m = Uuid::new_v4();
        let r = Uuid::new_v4();
        sql.execute(
            "insert into agent_conversations(id,title) values(?1,'Synthetic preparation')",
            [c.to_string()],
        )
        .unwrap();
        sql.execute("insert into research_projects(id,title,summary,created_at) values('project','Synthetic','','now')", []).unwrap();
        sql.execute("insert into manuscripts(id,project_id,title,root_path,created_at,updated_at) values(?1,'project','Synthetic','main.tex','now','now')", [m.to_string()]).unwrap();
        sql.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values(?1,'manuscript',?2)", params![c.to_string(),m.to_string()]).unwrap();
        let hash = format!("{:x}", Sha256::digest(b"Frozen manuscript"));
        sql.execute("insert into manuscript_files(manuscript_id,relative_path,file_kind,byte_size,sha256,modified_at) values(?1,'main.tex','tex',17,?2,'now')", params![m.to_string(),hash]).unwrap();
        sql.execute("insert into agent_host_shell_grants(conversation_id,host_kind,resource_id,revision) values(?1,'manuscript',?2,'synthetic-grant')", params![c.to_string(),m.to_string()]).unwrap();
        sql.execute("insert into concept_retrieval_traces(id,paper_id,query_sha256,status,config_json,context_text,context_sha256,seed_count,edge_count,source_count,external_paper_count,created_at,expires_at) values('prepare-trace',null,?1,'previewed','{}','Frozen retrieval',?2,0,0,0,0,'now',strftime('%Y-%m-%dT%H:%M:%fZ','now','+15 minutes'))", params![format!("{:x}",Sha256::digest(b"references")),format!("{:x}",Sha256::digest(b"Frozen retrieval"))]).unwrap();
        let db = Database::open(sql, &root.path().join("log")).unwrap();
        let store = AppStore::for_test_database(db.clone(), root.path().join("objects"));
        let source = store
            .manuscript_directory()
            .join(m.to_string())
            .join("source");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("main.tex"), "Frozen manuscript").unwrap();
        let request = PrepareRequest {
            conversation_id: c,
            request_id: r,
            text: if compact { "/compact" } else { "Explain" }.into(),
            user_input: vec![],
            context: WritingContextRequest {
                manuscript_id: m,
                files: vec![WritingContextFile {
                    relative_path: "main.tex".into(),
                    expected_disk_sha256: hash,
                    buffer: None,
                    selection: None,
                }],
            },
            resources: vec![],
            retrieval: Some(crate::agent::retrieval_context::RetrievalSelection {
                trace_id: "prepare-trace".into(),
                query: "references".into(),
            }),
            model: Some(ModelSelection {
                provider_id: "deepseek".into(),
                model_id: "fixture-model".into(),
                reasoning_effort: None,
                service_tier: None,
            }),
            session_config: None,
            turn_config: None,
        };
        Self {
            root,
            db,
            store,
            intent: WritingPrepareIntent {
                request,
                compact_revision: compact.then_some(0),
            },
        }
    }
    fn source(&self) -> WritingPrepareSource {
        WritingPrepareSource {
            store: self.store.clone(),
            request: self.intent.request.clone().into(),
            model: writing_submission::FrozenWritingModel {
                identity: research_agent_core::protocol::ModelIdentity {
                    provider_id: "deepseek".into(),
                    model_id: "fixture-model".into(),
                    revision: None,
                },
                reasoning_effort: None,
                service_tier: None,
            },
        }
    }
    fn prepare(&self) -> Result<SubmissionRecord, SqlImmediateFailure> {
        self.db.prepare_writing(&self.intent, self.source())
    }
    fn count(&self, table: &str) -> u64 {
        self.db
            .raw
            .lock()
            .unwrap()
            .query_row(&format!("select count(*) from {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
    }
    fn trace(&self) -> String {
        self.db
            .raw
            .lock()
            .unwrap()
            .query_row(
                "select status from concept_retrieval_traces where id='prepare-trace'",
                [],
                |row| row.get(0),
            )
            .unwrap()
    }
}

#[test]
fn ordinary_and_manual_compact_commit_all_preparation_facts_and_replay_at_fence_zero() {
    for compact in [false, true] {
        let f = Fixture::new(compact);
        let original = f.prepare().unwrap();
        assert_eq!(f.trace(), "sent");
        assert_eq!(f.count("agent_submissions"), 1);
        assert_eq!(f.count("agent_turn_resource_grants"), 1);
        assert_eq!(f.count("agent_turn_shell_grants"), 1);
        assert_eq!(f.count("agent_compaction_preparations"), u64::from(compact));
        assert_eq!(f.count("data_sql_operation_receipts"), 1);
        assert_eq!(f.count("agent_items"), 0);
        std::fs::remove_file(
            f.store
                .manuscript_directory()
                .join(f.intent.manuscript().to_string())
                .join("source/main.tex"),
        )
        .unwrap();
        let before = f.db.raw.lock().unwrap().total_changes();
        assert_eq!(
            f.db.replay_writing_prepare(&f.intent).unwrap(),
            Some(original.clone())
        );
        let WritingPrepareStatus::Committed { record, receipt } =
            f.db.writing_prepare_status(&f.intent).unwrap()
        else {
            panic!("committed fact required")
        };
        assert_eq!(*record, original);
        assert_eq!(receipt.sql_fence.position, 0);
        assert!(receipt.replayed);
        assert_eq!(f.db.raw.lock().unwrap().total_changes(), before);
    }
}

#[test]
fn exact_intent_rejects_changed_request_purpose_fence_and_cross_family_identity() {
    for compact in [false, true] {
        let f = Fixture::new(compact);
        f.prepare().unwrap();
        for field in 0..5 {
            let mut changed = f.intent.clone();
            match field {
                0 => changed.request.text.push('!'),
                1 => changed.request.context.manuscript_id = Uuid::new_v4(),
                2 => changed.request.model.as_mut().unwrap().model_id = "different".into(),
                3 => {
                    changed.request.turn_config = Some(crate::agent::config::ConfigOverride {
                        max_bytes: Some(99),
                        fallback_filenames: None,
                    })
                }
                _ => changed.compact_revision = if compact { Some(1) } else { Some(0) },
            }
            assert!(f.db.replay_writing_prepare(&changed).is_err());
            assert!(f.db.writing_prepare_status(&changed).is_err());
        }
        let db = f.db.raw.lock().unwrap();
        db.execute(
            "update data_sql_operation_receipts set kind='different.family' where operation_id=?1",
            [f.intent.operation().to_string()],
        )
        .unwrap();
        drop(db);
        assert!(matches!(
            f.db.writing_prepare_status(&f.intent),
            Err(SqlImmediateFailure::Conflict(_))
        ));
    }
}

#[test]
fn any_preparation_or_receipt_insert_failure_rolls_back_grants_header_and_trace() {
    for table in [
        "agent_submissions",
        "agent_turn_resource_grants",
        "agent_turn_shell_grants",
        "agent_compaction_preparations",
        "data_sql_operation_receipts",
        "concept_retrieval_traces",
    ] {
        let f = Fixture::new(true);
        let mutation = if table == "concept_retrieval_traces" {
            "update"
        } else {
            "insert"
        };
        f.db.raw.lock().unwrap().execute_batch(&format!("create trigger synthetic_prepare_fault before {mutation} on {table} begin select raise(abort,'synthetic failure'); end;")).unwrap();
        let error = f.prepare().expect_err("SQL failure required");
        assert_eq!(error.code(), "DATA_SQL_UNAVAILABLE");
        let detail = error.diagnostic().unwrap();
        assert_eq!(detail.operation_id, f.intent.operation());
        assert_eq!(
            detail.sqlite_code.as_deref(),
            Some("ConstraintViolation"),
            "{table}"
        );
        assert_eq!(
            detail.sqlite_extended_code,
            Some(rusqlite::ffi::SQLITE_CONSTRAINT_TRIGGER),
            "{table}"
        );
        assert_eq!(f.trace(), "previewed");
        for table in [
            "agent_submissions",
            "agent_turn_resource_grants",
            "agent_turn_shell_grants",
            "agent_compaction_preparations",
            "data_sql_operation_receipts",
        ] {
            assert_eq!(f.count(table), 0);
        }
        assert!(f.db.raw.lock().unwrap().is_autocommit());
    }
}

#[test]
fn empty_context_cannot_commit_a_record_proof_or_consume_retrieval() {
    let mut f = Fixture::new(false);
    f.intent.request.context.files.clear();
    f.intent.request.retrieval = None;
    assert!(f.prepare().is_err());
    assert_eq!(f.trace(), "previewed");
    assert_eq!(f.count("agent_submissions"), 0);
    assert_eq!(f.count("data_sql_operation_receipts"), 0);
    assert!(f.db.lock().is_ok());
}

#[test]
fn unknown_commit_remains_blocked_and_only_post_commit_fact_is_observable() {
    for after in [false, true] {
        let f = Fixture::new(true);
        let result = f.db.prepare_writing_owned(
            &f.intent,
            f.source(),
            if after {
                Fault::AfterCommit
            } else {
                Fault::BeforeCommit
            },
        );
        assert!(matches!(result, Err(SqlImmediateFailure::Unknown(_))));
        assert!(f.db.lock().is_err());
        assert!(f.prepare().is_err());
        let state = f.db.writing_prepare_status(&f.intent).unwrap();
        if after {
            assert!(matches!(state, WritingPrepareStatus::Committed { .. }));
            assert_eq!(f.trace(), "sent");
        } else {
            assert!(matches!(state, WritingPrepareStatus::Unconfirmed(_)));
            assert_eq!(f.trace(), "previewed");
        }
    }
}

#[test]
fn legacy_record_has_no_fabricated_proof_and_closed_status_does_not_unblock() {
    let f = Fixture::new(false);
    f.prepare().unwrap();
    f.db.raw
        .lock()
        .unwrap()
        .execute(
            "delete from data_sql_operation_receipts where operation_id=?1",
            [f.intent.operation().to_string()],
        )
        .unwrap();
    assert!(matches!(
        f.db.writing_prepare_status(&f.intent).unwrap(),
        WritingPrepareStatus::Unconfirmed(_)
    ));
    assert!(matches!(
        f.db.replay_writing_prepare(&f.intent),
        Err(SqlImmediateFailure::Conflict(_))
    ));
    assert_eq!(f.count("data_sql_operation_receipts"), 0);
    f.db.shutdown().unwrap();
    assert!(matches!(
        f.db.writing_prepare_status(&f.intent).unwrap(),
        WritingPrepareStatus::Unconfirmed(_)
    ));
    assert!(f.prepare().is_err());
}

#[test]
fn committed_pair_reopens_with_original_epoch_fence_and_no_source_or_trace_consumption() {
    let f = Fixture::new(true);
    let original = f.prepare().unwrap();
    let WritingPrepareStatus::Committed { receipt, .. } =
        f.db.writing_prepare_status(&f.intent).unwrap()
    else {
        panic!("proof required")
    };
    f.db.shutdown().unwrap();
    let Fixture {
        root,
        db,
        store,
        intent,
    } = f;
    drop(store);
    drop(db);
    let sql = Connection::open(root.path().join("prepare.sqlite3")).unwrap();
    let db = Database::open(sql, &root.path().join("log")).unwrap();
    let WritingPrepareStatus::Committed {
        record,
        receipt: reopened,
    } = db.writing_prepare_status(&intent).unwrap()
    else {
        panic!("proof required")
    };
    assert_eq!(*record, original);
    assert_eq!(reopened.epoch, receipt.epoch);
    assert_eq!(reopened.sql_fence, receipt.sql_fence);
    assert_eq!(db.replay_writing_prepare(&intent).unwrap(), Some(original));
    db.shutdown().unwrap();
}
