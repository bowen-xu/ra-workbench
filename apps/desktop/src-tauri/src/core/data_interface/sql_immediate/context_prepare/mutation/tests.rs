use super::*;
use crate::{
    agent::{
        context_session::sources::{Access, Resource},
        writing_context::{ContextRead, WritingContextRequest},
        writing_resources::{WritingScope, WritingScopeReader},
    },
    storage::AppStore,
};
use research_agent_core::submission::ContextText;
use std::path::Path;

struct Fixture {
    _root: tempfile::TempDir,
    store: AppStore,
    command: ContextPrepareIntent,
    manuscript: Uuid,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let mut db = Connection::open_in_memory().unwrap();
        crate::storage::migrate(&mut db).unwrap();
        let conversation = Uuid::new_v4();
        let manuscript = Uuid::new_v4();
        db.execute(
            "insert into agent_conversations(id,title) values(?1,'Synthetic')",
            [conversation.to_string()],
        )
        .unwrap();
        db.execute("insert into research_projects(id,title,summary,created_at) values('fixture-project','fixture','','now')", []).unwrap();
        db.execute("insert into manuscripts(id,project_id,title,root_path,created_at,updated_at) values(?1,'fixture-project','fixture','main.tex','now','now')", [manuscript.to_string()]).unwrap();
        db.execute(
            "insert into manuscript_files values(?1,'main.tex','tex',1,?2,'now')",
            rusqlite::params![manuscript.to_string(), "a".repeat(64)],
        )
        .unwrap();
        let database = Database::open(db, &root.path().join("log")).unwrap();
        let store = AppStore::for_test_database(database, root.path().join("objects"));
        let command = ContextPrepareIntent {
            request: context_session::PrepareRequest {
                conversation_id: conversation,
                request_id: Uuid::new_v4(),
                text: "Synthetic question".into(),
                user_input: vec![],
                resources: vec![Resource::Manuscript {
                    resource_id: manuscript.to_string(),
                    new_member_access: Some(Access::ReadOnly),
                    group_access: Some(Access::ReadOnly),
                    parts: vec![],
                }],
                model: None,
                retrieval: None,
                queue: None,
            },
            expected_epoch: None,
        };
        Self {
            _root: root,
            store,
            command,
            manuscript,
        }
    }
    fn source(&self) -> ContextPrepareSource {
        ContextPrepareSource {
            store: self.store.clone(),
            model: (
                ModelIdentity {
                    provider_id: "fixture".into(),
                    model_id: "fixture".into(),
                    revision: None,
                },
                None,
                None,
            ),
        }
    }
    fn deny_rollback(&self) {
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
        self.store
            .connection()
            .unwrap()
            .authorizer(Some(|context: AuthContext<'_>| match context.action {
                AuthAction::Transaction {
                    operation: TransactionOperation::Rollback,
                } => Authorization::Deny,
                _ => Authorization::Allow,
            }));
    }
}
struct Reader {
    scope_error: bool,
}
impl Reader {
    fn scope_failure(&self) -> crate::agent::context_catalog::ContextCatalogReadError {
        crate::agent::context_catalog::ContextCatalogReadError::Native(Box::new(
            ContextCatalogFailure {
                code: "writing_scope_source_unavailable".into(),
                message: "Fixed scope unavailable",
                package_key: "a".repeat(64),
                phase: "read",
                safe_cause: "sqlite_read_failed".into(),
                resource_id: Some("writing.projects.context-text-paths".into()),
                resource_version: Some(1),
                sqlite_code: Some("AuthorizationForStatementDenied".into()),
                sqlite_extended_code: Some(23),
            },
        ))
    }
}
impl WritingScopeReader for Reader {
    fn available(&self) -> Result<(), crate::agent::context_catalog::ContextCatalogReadError> {
        if self.scope_error {
            Err(self.scope_failure())
        } else {
            Ok(())
        }
    }
    fn is_text_path(
        &self,
        _: &str,
    ) -> Result<bool, crate::agent::context_catalog::ContextCatalogReadError> {
        Err(self.scope_failure())
    }
    fn list_text_paths(
        &self,
        _: &Connection,
        _: &str,
    ) -> Result<Vec<String>, crate::agent::context_catalog::ContextCatalogReadError> {
        Err(self.scope_failure())
    }
}
impl WritingContextReader for Reader {
    fn resource_scope(&self) -> Option<WritingScope<'_>> {
        self.scope_error.then_some(WritingScope::Native(self))
    }
    fn selected(
        &mut self,
        _: &Connection,
        _: &Path,
        _: WritingContextRequest,
    ) -> ContextRead<Vec<ContextText>> {
        panic!("only a current-file reference may pause")
    }
    fn current_file(
        &mut self,
        _: &Connection,
        _: &Path,
        id: Uuid,
        _: &str,
    ) -> ContextRead<Vec<ContextText>> {
        Err(Flow::Need(Box::new(
            research_workbench_native_sdk::writing_project::Input::PrepareStatus {
                manuscript_id: id.to_string(),
            },
        )))
    }
}

#[test]
fn consumed_scope_failure_keeps_exact_diagnostics_and_original_rollback_outcome() {
    for uncertain in [false, true] {
        let f = Fixture::new();
        if uncertain {
            f.deny_rollback();
        }
        let result = f
            .store
            .shared_agent_connection()
            .prepare_context_attempt(
                &f.command,
                f.source(),
                Some(&mut Reader { scope_error: true }),
                super::super::super::Fault::None,
            )
            .unwrap();
        let failure = match result {
            Err(failure) => failure,
            Ok(_) => panic!("scope must reject"),
        };
        assert_eq!(failure.catalog.package_key, "a".repeat(64));
        assert_eq!(failure.catalog.sqlite_extended_code, Some(23));
        if uncertain {
            assert!(matches!(failure.error, SqlImmediateFailure::Unknown(_)));
            assert!(f.store.shared_agent_connection().lock().is_err());
        } else {
            assert!(matches!(failure.error, SqlImmediateFailure::Unavailable(_)));
            assert!(f.store.connection().unwrap().is_autocommit());
            let count: u64 = f
                .store
                .connection()
                .unwrap()
                .query_row("select count(*) from agent_submissions", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(count, 0);
        }
        let serialized = serde_json::to_value(context_session::prepare_status::Error::scope(
            failure,
            f.command.request.conversation_id,
            f.command.request.request_id,
        ))
        .unwrap();
        assert_eq!(
            serialized["contextSource"]["requestId"],
            f.command.request.request_id.to_string()
        );
        assert_eq!(
            serialized["contextSource"]["source"]["error"]["dataError"]["sqliteExtendedCode"],
            23
        );
    }
}

#[test]
fn reference_need_rolls_back_before_native_and_cannot_escape_uncertain_cleanup() {
    for uncertain in [false, true] {
        let mut f = Fixture::new();
        if uncertain {
            f.deny_rollback();
        }
        let file = Resource::ManuscriptFile {
            resource_id: f.manuscript,
            path: "main.tex".into(),
        };
        f.command.request.user_input = serde_json::from_value(serde_json::json!([
            {"type":"text", "text":f.command.request.text, "textElements":[]},
            {"type":"mention", "name":"main.tex", "target": {
                "kind":"file",
                "sourceId":format!("context:{}", serde_json::to_string(&file).unwrap()),
                "resourceId":f.manuscript.to_string()
            }}
        ]))
        .unwrap();
        let result = f.store.shared_agent_connection().prepare_context_attempt(
            &f.command,
            f.source(),
            Some(&mut Reader { scope_error: false }),
            super::super::super::Fault::None,
        );
        if uncertain {
            assert!(matches!(
                result,
                Err(Flow::Fail(
                    crate::ai::structured_resource_validation::Failure {
                        error: SqlImmediateFailure::Unknown(_),
                        ..
                    }
                ))
            ));
            assert!(f.store.shared_agent_connection().lock().is_err());
        } else {
            assert!(matches!(result, Err(Flow::Need(_))));
            assert!(f.store.connection().unwrap().is_autocommit());
            let counts: (u64, u64) = f
                .store
                .connection()
                .unwrap()
                .query_row(
                    "select (select count(*) from agent_submissions),
                    (select count(*) from data_sql_operation_receipts)",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(counts, (0, 0));
        }
    }
}
