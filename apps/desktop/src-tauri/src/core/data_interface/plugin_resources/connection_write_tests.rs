use super::*;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

const UPDATE_SQL: &str = "update records set value=?1 where id=?2";

fn resource(sql: &str, access: DataAccess, columns: Vec<DataField>) -> AcceptedDataResource {
    PackageFixture::new(&[("write.sql", sql)]).resource(
        "write.sql",
        access,
        vec![text(), text()],
        columns,
    )
}

fn database() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(
        "create table records(id text primary key,value text); \
         create table observed(value text); insert into records values('candidate','pending');",
    )
    .unwrap();
    db
}

fn transaction_count(db: &Connection) -> Arc<AtomicUsize> {
    let count = Arc::new(AtomicUsize::new(0));
    let observed = count.clone();
    db.authorizer(Some(move |context: AuthContext<'_>| {
        if matches!(context.action, AuthAction::Transaction { .. }) {
            observed.fetch_add(1, Ordering::SeqCst);
        }
        Authorization::Allow
    }));
    count
}

fn values(db: &Connection) -> (String, u64) {
    db.query_row(
        "select (select value from records where id='candidate'),(select count(*) from observed)",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .unwrap()
}

#[test]
fn connection_write_keeps_autocommit_and_original_ignore_fail_trigger_effects() {
    let write = resource(UPDATE_SQL, DataAccess::Write, vec![]);
    for (raise, retained) in [
        ("ignore", true),
        ("fail,'private trigger body'", true),
        ("abort,'private trigger body'", false),
    ] {
        let original = database();
        let adapted = database();
        let trigger = format!(
            "create trigger observation before update on records begin \
             insert into observed values('private observation'); select raise({raise}); end;"
        );
        original.execute_batch(&trigger).unwrap();
        adapted.execute_batch(&trigger).unwrap();
        let transactions = transaction_count(&adapted);
        let expected = original.execute(UPDATE_SQL, ["dismiss", "candidate"]);
        let actual =
            write_on_connection(&adapted, &write, &call(&write, &["dismiss", "candidate"]));
        match (expected, actual) {
            (Ok(expected), Ok(actual)) => assert_eq!(actual, expected as u64),
            (Err(rusqlite::Error::SqliteFailure(expected, _)), Err(actual)) => {
                assert_eq!(actual.code, "plugin_data_sql_failed");
                assert_eq!(actual.phase, "execute");
                assert_eq!(actual.sqlite_extended_code, Some(expected.extended_code));
                assert_eq!(actual.safe_cause, format!("{:?}", expected.code));
                let safe = serde_json::to_string(&actual).unwrap();
                for private in [
                    UPDATE_SQL,
                    "private trigger body",
                    "private observation",
                    "dismiss",
                    "candidate",
                ] {
                    assert!(!safe.contains(private));
                }
            }
            _ => panic!("adapter must retain the original statement outcome"),
        }
        assert_eq!(values(&adapted), values(&original));
        assert_eq!(values(&adapted), ("pending".into(), u64::from(retained)));
        assert!(adapted.is_autocommit());
        assert_eq!(transactions.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn connection_write_returns_actual_zero_and_does_not_finish_an_existing_transaction() {
    let write = resource(UPDATE_SQL, DataAccess::Write, vec![]);
    let db = database();
    assert_eq!(
        write_on_connection(&db, &write, &call(&write, &["dismiss", "absent"])).unwrap(),
        0
    );
    db.execute_batch("begin deferred").unwrap();
    let transactions = transaction_count(&db);
    assert_eq!(
        write_on_connection(&db, &write, &call(&write, &["dismiss", "candidate"])).unwrap(),
        1
    );
    assert_eq!(values(&db).0, "dismiss");
    assert!(!db.is_autocommit());
    assert_eq!(transactions.load(Ordering::SeqCst), 0);
    db.execute_batch("rollback").unwrap();
    assert_eq!(values(&db).0, "pending");
}

#[test]
fn connection_write_keeps_fixed_access_call_and_statement_checks_before_writes() {
    let db = database();
    let transactions = transaction_count(&db);
    let read = resource("select ?1,?2", DataAccess::Read, vec![text(), text()]);
    assert_eq!(
        write_on_connection(&db, &read, &call(&read, &["dismiss", "candidate"]))
            .unwrap_err()
            .safe_cause,
        "resource_access_mismatch"
    );
    let write = resource(UPDATE_SQL, DataAccess::Write, vec![]);
    let valid = call(&write, &["dismiss", "candidate"]);
    for invalid in [
        DataCall {
            id: "literature.other".into(),
            ..valid.clone()
        },
        DataCall {
            version: 2,
            ..valid.clone()
        },
        DataCall {
            arguments: vec![],
            ..valid.clone()
        },
        DataCall {
            arguments: vec![DataValue::Null, DataValue::Text("candidate".into())],
            ..valid.clone()
        },
    ] {
        assert_eq!(
            write_on_connection(&db, &write, &invalid).unwrap_err().code,
            "plugin_data_call_invalid"
        );
    }
    for sql in [
        "select ?1,?2",
        "update records set value=?1 where id=?2 returning value",
    ] {
        let incompatible = resource(sql, DataAccess::Write, vec![]);
        assert_eq!(
            write_on_connection(
                &db,
                &incompatible,
                &call(&incompatible, &["dismiss", "candidate"])
            )
            .unwrap_err()
            .code,
            "plugin_data_statement_incompatible"
        );
    }
    assert_eq!(values(&db), ("pending".into(), 0));
    assert_eq!(transactions.load(Ordering::SeqCst), 0);
    assert!(db.is_autocommit());
}
