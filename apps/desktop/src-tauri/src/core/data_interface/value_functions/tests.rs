use super::*;
use rusqlite::params;

#[test]
fn unicode_and_nul_text_keep_the_complete_original_characters() {
    let db = Connection::open_in_memory().unwrap();
    register(&db).unwrap();
    let values = db
        .query_row(
            "select data_text_trim_v1(?1), data_text_char_count_v1(?2),
         data_text_prefix_v1(?2,3), data_text_trim_end_matches_v1(?3,?4),
         data_text_collapse_whitespace_v1(?5), data_text_lowercase_v1(?6)",
            params![
                "\u{2003}纸\0字\u{2003}",
                "纸\0🦀字",
                "纸`🦀``",
                "`🦀",
                "\u{2003}纸\0字\t\n另\u{a0}一\u{2003}",
                "ΟΣ İ"
            ],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(
        values,
        (
            "纸\0字".into(),
            4,
            "纸\0🦀".into(),
            "纸".into(),
            "纸\0字 另 一".into(),
            "ος i\u{307}".into()
        )
    );
    for count in [0_i64, i64::MAX] {
        let result: String = db
            .query_row(
                "select data_text_prefix_v1(?1,?2)",
                params!["纸\0字", count],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(result, if count == 0 { "" } else { "纸\0字" });
    }
}

#[test]
fn quote_and_hash_match_exact_utf8_bytes_including_after_nul() {
    let db = Connection::open_in_memory().unwrap();
    register(&db).unwrap();
    let (quoted, hash): (String, String) = db
        .query_row(
            "select data_json_quote_v1(?1),data_sha256_v1(?2)",
            params!["纸\0字\n\"\\", "a\0b"],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(quoted, r#""纸\u0000字\n\"\\""#);
    assert_eq!(
        hash,
        "59b271ae1bbcb1d31d41929817f4b16fb439eb4f31520b5ad1d5ce98920a7138"
    );
    let empty: String = db
        .query_row("select data_sha256_v1('')", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        empty,
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn invalid_types_and_invalid_utf8_are_rejected_without_sqlite_coercion_or_input_text() {
    let db = Connection::open_in_memory().unwrap();
    register(&db).unwrap();
    for name in [
        "data_text_trim_v1",
        "data_text_char_count_v1",
        "data_text_collapse_whitespace_v1",
        "data_text_lowercase_v1",
        "data_json_quote_v1",
        "data_sha256_v1",
    ] {
        for argument in ["null", "42", "1.5", "x'610062'", "cast(x'ff' as text)"] {
            let sql = format!("select {name}({argument})");
            let error = db
                .query_row(&sql, [], |r| r.get::<_, String>(0))
                .unwrap_err();
            assert!(error.to_string().contains("DATA_VALUE_FUNCTION_INVALID"));
        }
    }
    for sql in [
        "select data_text_prefix_v1('text',-1)",
        "select data_text_prefix_v1('text','2')",
        "select data_text_prefix_v1('text',2.0)",
        "select data_text_prefix_v1(null,2)",
        "select data_text_prefix_v1('text',null)",
        "select data_text_trim_end_matches_v1(7,'x')",
        "select data_text_trim_end_matches_v1('x',7)",
    ] {
        assert!(db.query_row(sql, [], |r| r.get::<_, String>(0)).is_err());
    }
    let error = db
        .query_row(
            "select data_text_trim_v1(cast(?1 as text))",
            [vec![255_u8]],
            |r| r.get::<_, String>(0),
        )
        .unwrap_err()
        .to_string();
    assert!(!error.contains("255"));
    assert!(error.contains("utf8-text"));
    assert!(db.is_autocommit());
    assert_eq!(
        db.query_row("select data_text_char_count_v1('仍可用')", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        3
    );
}

#[test]
fn exact_function_flags_allow_existing_schema_execution_with_trusted_schema_off() {
    let db = Connection::open_in_memory().unwrap();
    register(&db).unwrap();
    register(&db).unwrap();
    let mut statement=db.prepare("select name,narg,flags from pragma_function_list where name like 'data_%_v1' order by name").unwrap();
    let rows = statement
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i32>(1)?,
                r.get::<_, i32>(2)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(rows.len(), 8);
    for (name, arity, flags) in rows {
        assert_eq!(
            arity,
            if ["data_text_prefix_v1", "data_text_trim_end_matches_v1"].contains(&name.as_str()) {
                2
            } else {
                1
            }
        );
        assert_ne!(flags & FunctionFlags::SQLITE_DETERMINISTIC.bits(), 0);
        assert_ne!(flags & FunctionFlags::SQLITE_INNOCUOUS.bits(), 0);
        assert_eq!(flags & FunctionFlags::SQLITE_DIRECTONLY.bits(), 0);
    }
    drop(statement);
    db.execute_batch("pragma trusted_schema=off;
        create table synthetic_values(value text not null);
        create trigger synthetic_trim after insert on synthetic_values begin
          update synthetic_values set value=data_text_trim_v1(new.value) where rowid=new.rowid;
        end;
        create view synthetic_quoted as select data_json_quote_v1(value) as value from synthetic_values;").unwrap();
    db.execute(
        "insert into synthetic_values(value) values(?1)",
        ["\u{2003}纸\0字\u{2003}"],
    )
    .unwrap();
    assert_eq!(
        db.query_row("select value from synthetic_quoted", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        r#""纸\u0000字""#
    );
}

#[test]
fn production_migration_registers_values_on_both_fresh_and_reopened_connections() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("synthetic-values.sqlite3");
    for _ in 0..2 {
        let mut db = Connection::open(&path).unwrap();
        crate::storage::migrate(&mut db).unwrap();
        assert_eq!(
            db.query_row(
                "select data_text_char_count_v1(?1)",
                ["纸\0🦀字"],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            4
        );
    }
}
