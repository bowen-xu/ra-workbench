use super::*;
use crate::agent::repository::SqlRepositoryError as E;
use rusqlite::{
    hooks::{AuthAction, AuthContext, Authorization},
    params,
};
use sha2::{Digest, Sha256};
fn hash(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}
fn trace(db: &Connection, id: &str, paper: Option<&str>, query: &str, text: &str, expires: &str) {
    db.execute("insert into concept_retrieval_traces(id,paper_id,query_sha256,status,context_text,context_sha256,expires_at) values(?1,?2,?3,'previewed',?4,?5,?6)",params![id,paper,hash(query.trim()),text,hash(text),expires]).unwrap();
}
fn state(db: &Connection, id: &str) -> (String, Option<String>) {
    db.query_row(
        "select status,sent_at from concept_retrieval_traces where id=?1",
        [id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .unwrap()
}
fn selection(id: &str, query: &str) -> RetrievalSelection {
    RetrievalSelection {
        trace_id: id.into(),
        query: query.into(),
    }
}
fn assert_sql(cause: Result<ContextText, E<RetrievalContextError>>) {
    match cause {
        Err(E::Sql(error)) => assert_eq!(
            error.sqlite_error().unwrap().extended_code,
            rusqlite::ffi::SQLITE_AUTH
        ),
        _ => panic!("expected the original authorization SQL error"),
    }
}
#[test]
fn app_trace_bridge_preserves_scope_expiry_source_then_mark_and_sql_errors() {
    let mut db = Connection::open_in_memory().unwrap();
    db.execute_batch("create table concept_retrieval_traces(id text primary key,paper_id text,query_sha256 text not null,status text not null,context_text text not null,context_sha256 text not null,expires_at text not null,sent_at text);").unwrap();
    let source = trace_source();
    trace(
        &db,
        "global",
        None,
        "question",
        "[CN-S1] frozen context",
        "9999",
    );
    let selected = selection("global", " question ");
    for (scope, resource, anchor) in [
        (
            RetrievalScope::Conversation {
                conversation_id: "conversation",
            },
            "global-research",
            serde_json::json!({"conversationId":"conversation","traceId":"global","query":" question "}),
        ),
        (
            RetrievalScope::Manuscript {
                manuscript_id: "manuscript",
            },
            "global-research",
            serde_json::json!({"manuscriptId":"manuscript","traceId":"global","query":" question "}),
        ),
        (
            RetrievalScope::Note {
                resource_id: "page",
                anchor_key: "notePageId",
            },
            "page",
            serde_json::json!({"notePageId":"page","traceId":"global","query":" question "}),
        ),
    ] {
        let tx = db.transaction().unwrap();
        let frozen =
            consume_in_sql(source, &tx, &selected, scope).unwrap_or_else(|cause| match cause {
                E::Sql(cause) => panic!("SQL source error: {cause}"),
                E::Domain(cause) => panic!("trace source error: {cause}"),
            });
        assert_eq!(frozen.source_id, "retrieval:global");
        assert_eq!(frozen.revision, "trace:global");
        assert_eq!(frozen.resource_id, resource);
        assert_eq!(frozen.text, "[CN-S1] frozen context");
        assert!(matches!(
            frozen.kind,
            research_agent_core::submission::ContextSourceKind::Retrieval
        ));
        assert_eq!(frozen.anchor.as_deref(), Some(anchor.to_string().as_str()));
        let (status, sent) = state(&tx, "global");
        assert_eq!(status, "sent");
        assert!(sent.is_some());
        tx.rollback().unwrap();
        assert_eq!(state(&db, "global"), ("previewed".into(), None));
    }
    trace(
        &db,
        "paper-trace",
        Some("paper"),
        "question",
        "paper frozen",
        "9999",
    );
    assert!(matches!(
        consume_in_sql(
            source,
            &db,
            &selection("paper-trace", "question"),
            RetrievalScope::Paper {
                paper_id: "other",
                snapshot_id: "snapshot"
            }
        ),
        Err(E::Domain(RetrievalContextError::Conflict))
    ));
    assert!(matches!(
        consume_in_sql(
            source,
            &db,
            &selection("paper-trace", "changed query"),
            RetrievalScope::Paper {
                paper_id: "paper",
                snapshot_id: "snapshot"
            }
        ),
        Err(E::Domain(RetrievalContextError::Conflict))
    ));
    {
        let tx = db.transaction().unwrap();
        let frozen = consume_in_sql(
            source,
            &tx,
            &selection("paper-trace", "question"),
            RetrievalScope::Paper {
                paper_id: "paper",
                snapshot_id: "snapshot",
            },
        )
        .unwrap_or_else(|cause| match cause {
            E::Sql(cause) => panic!("SQL source error: {cause}"),
            E::Domain(cause) => panic!("trace source error: {cause}"),
        });
        assert_eq!(frozen.resource_id, "paper");
        assert_eq!(frozen.source_id, "retrieval:paper-trace");
        assert_eq!(frozen.revision, "trace:paper-trace");
        assert_eq!(frozen.anchor.as_deref(),Some(serde_json::json!({"paperId":"paper","snapshotId":"snapshot","traceId":"paper-trace","query":"question"}).to_string().as_str()));
        assert_eq!(state(&tx, "paper-trace").0, "sent");
        tx.rollback().unwrap();
    }
    assert_eq!(state(&db, "paper-trace"), ("previewed".into(), None));
    trace(&db, "expired", None, "question", "expired frozen", "2000");
    {
        let tx = db.transaction().unwrap();
        assert!(matches!(
            consume_in_sql(
                source,
                &tx,
                &selection("expired", "question"),
                RetrievalScope::Conversation {
                    conversation_id: "conversation"
                }
            ),
            Err(E::Domain(RetrievalContextError::Conflict))
        ));
        assert_eq!(state(&tx, "expired"), ("expired".into(), None));
        tx.rollback().unwrap();
    }
    assert_eq!(state(&db, "expired"), ("previewed".into(), None));
    // Loading succeeds, but the shared ContextText constructor rejects the
    // empty resource. The original bare connection must remain unmarked.
    assert!(matches!(
        consume_in_sql(
            source,
            &db,
            &selected,
            RetrievalScope::Note {
                resource_id: "",
                anchor_key: "notePageId"
            }
        ),
        Err(E::Domain(RetrievalContextError::Invalid))
    ));
    assert_eq!(state(&db, "global"), ("previewed".into(), None));
    db.authorizer(Some(|context: AuthContext<'_>| match context.action {
        AuthAction::Update {
            table_name: "concept_retrieval_traces",
            ..
        } => Authorization::Deny,
        _ => Authorization::Allow,
    }));
    assert_sql(consume_in_sql(
        source,
        &db,
        &selected,
        RetrievalScope::Conversation {
            conversation_id: "conversation",
        },
    ));
    assert_eq!(state(&db, "global"), ("previewed".into(), None));
    db.authorizer(Some(|context: AuthContext<'_>| match context.action {
        AuthAction::Read {
            table_name: "concept_retrieval_traces",
            ..
        } => Authorization::Deny,
        _ => Authorization::Allow,
    }));
    assert_sql(consume_in_sql(
        source,
        &db,
        &selected,
        RetrievalScope::Conversation {
            conversation_id: "conversation",
        },
    ));
    assert!(matches!(
        consume_in_sql(
            source,
            &db,
            &selection("", "question"),
            RetrievalScope::Conversation {
                conversation_id: "conversation"
            }
        ),
        Err(E::Domain(RetrievalContextError::Invalid))
    ));
    db.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    assert!(db.is_autocommit());
}

fn trace_source() -> RetrievalTraceSource {
    use research_literature_library_native::retrieval_trace::{self, RetrievalTraceError};
    use research_workbench_native_sdk::retrieval_trace::RetrievalTraceRow;
    use rusqlite::OptionalExtension;
    RetrievalTraceSource {
        load_for_scope: |db, id, scope, query| {
            let observed = db
                .query_row(
                    include_str!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/../../../plugins/literature_library/native/data/trace-read.v1.sql"
                    )),
                    [id],
                    |row| {
                        Ok(RetrievalTraceRow {
                            paper_id: row.get(0)?,
                            query_sha256: row.get(1)?,
                            status: row.get(2)?,
                            context_text: row.get(3)?,
                            context_sha256: row.get(4)?,
                            expired: row.get(5)?,
                        })
                    },
                )
                .optional()
                .map_err(E::Sql)?;
            let validated = retrieval_trace::validate_context(scope, query, observed);
            if matches!(validated, Err(RetrievalTraceError::Expired)) {
                db.execute(
                    include_str!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/../../../plugins/literature_library/native/data/trace-expire.v1.sql"
                    )),
                    [id],
                )
                .map_err(E::Sql)?;
            }
            let value = validated.map_err(|_| E::Domain(RetrievalContextError::Conflict))?;
            Ok(LoadedRetrievalContext {
                trace_id: id.into(),
                context_text: value.context_text,
            })
        },
        mark_sent: |db, id| {
            let changed = db
                .execute(
                    include_str!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/../../../plugins/literature_library/native/data/trace-mark-sent.v1.sql"
                    )),
                    [id],
                )
                .map_err(E::Sql)?;
            retrieval_trace::validate_mark_sent(changed)
                .map_err(|_| E::Domain(RetrievalContextError::Conflict))
        },
    }
}
