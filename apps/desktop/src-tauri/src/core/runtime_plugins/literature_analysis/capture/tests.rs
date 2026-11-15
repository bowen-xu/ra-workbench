use super::*;

const WORKFLOW: &str = "select 'workflow','paper','snapshot','failed','provider','model',null,'v1',null,1,null,'old','now',null,null where ?1 is not null";
const TURN: &str = "select 'turn','workflow',0,'paper_overview','interrupted',null,'old',null where ?1 is not null";

fn statements() -> Statements<'static> {
    Statements {
        workflow: WORKFLOW,
        latest: "select 'workflow' where ?1 is not null",
        turns: TURN,
        attempts: "select 'attempt','turn',1,'interrupted','[]','provider','model',null,null,null,null,null,null,null,null,null,'old',null,null,null,null,null,null,null,null,null,null,null where ?1 is not null",
    }
}
fn request() -> HistoryRequestV1 {
    HistoryRequestV1::Get {
        workflow_id: "workflow".into(),
    }
}

#[test]
fn absent_header_preserves_lazy_reads() {
    let db = Connection::open_in_memory().unwrap();
    let mut sql = statements();
    sql.workflow = "select 'never' where ?1 is null";
    sql.turns = "select * from must_not_be_read where ?1 is not null";
    assert!(capture(&db, &request(), &sql).unwrap().is_none());
    sql.latest = "select 'never' where ?1 is null";
    sql.workflow = "select * from must_not_be_read where ?1 is not null";
    assert!(
        capture(
            &db,
            &HistoryRequestV1::Latest {
                paper_id: "paper".into()
            },
            &sql
        )
        .unwrap()
        .is_none()
    );
}

#[test]
fn workflow_optional_conversation_keeps_original_decode_position() {
    let db = Connection::open_in_memory().unwrap();
    let mut sql = statements();
    sql.workflow = "select 'workflow',x'ff','snapshot','failed','provider','model',null,'v1',null,1,null,'old','now',null,x'ff' where ?1 is not null";
    let error = capture(&db, &request(), &sql).unwrap_err();
    assert!(matches!(
        error,
        AiError::Database(rusqlite::Error::InvalidColumnType(14, _, Type::Blob))
    ));
}

#[test]
fn attempt_json_is_decoded_before_its_identifier_and_shared_ids() {
    let db = Connection::open_in_memory().unwrap();
    let mut sql = statements();
    sql.attempts = "select x'ff','turn',1,'interrupted','broken','provider','model',null,null,null,null,null,null,null,null,null,'old',null,null,null,null,null,x'ff',null,null,null,null,null where ?1 is not null";
    let error = capture(&db, &request(), &sql).unwrap_err();
    assert!(matches!(
        error,
        AiError::Database(rusqlite::Error::FromSqlConversionFailure(4, Type::Text, _))
    ));
}

#[test]
fn owned_capture_keeps_messages_and_releases_the_connection() {
    let db = Connection::open_in_memory().unwrap();
    let capture = capture(&db, &request(), &statements()).unwrap().unwrap();
    assert_eq!(capture.row.id, "workflow");
    assert_eq!(capture.turns[0].attempts[0].status, "interrupted");
    assert!(capture.turns[0].attempts[0].normalized_messages.is_empty());
    drop(db);
    assert_eq!(capture.turns[0].row.id, "turn");
}
