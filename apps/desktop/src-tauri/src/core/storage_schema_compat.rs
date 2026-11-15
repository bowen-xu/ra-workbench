//! SQL functions retained by deployed schema, independent of enabled plugins.
use rusqlite::{Connection, functions::FunctionFlags};

/// Migration 201's persistent views require this function on every connection,
/// including builds that do not load writing management.
pub(crate) fn register_functions(connection: &Connection) -> rusqlite::Result<()> {
    connection.create_scalar_function(
        "ra_writing_note_title_v1",
        1,
        FunctionFlags::SQLITE_UTF8
            | FunctionFlags::SQLITE_DETERMINISTIC
            | FunctionFlags::SQLITE_INNOCUOUS,
        |context| Ok(writing_note_title(&context.get::<String>(0)?)),
    )
}

/// Preserve the deployed view's slash and case-insensitive `.ranote` rules.
pub(crate) fn writing_note_title(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    if name
        .get(name.len().saturating_sub(7)..)
        .is_some_and(|end| end.eq_ignore_ascii_case(".ranote"))
    {
        name[..name.len() - 7].to_owned()
    } else {
        name.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deployed_title_rules_match_sql_for_unicode_case_and_path_edges() {
        let db = Connection::open_in_memory().unwrap();
        register_functions(&db).unwrap();
        for (path, expected) in [
            ("", ""),
            ("folder/", ""),
            (".ranote", ""),
            ("folder/Note.RANOTE", "Note"),
            ("文献/研究🙂.rAnOtE", "研究🙂"),
            ("folder/name.ranote.ranote", "name.ranote"),
            (r"folder\Note.ranote", r"folder\Note"),
            ("研究🙂", "研究🙂"),
            ("文.ranoté", "文.ranoté"),
        ] {
            assert_eq!(writing_note_title(path), expected, "{path}");
            assert_eq!(
                db.query_row("select ra_writing_note_title_v1(?1)", [path], |row| {
                    row.get::<_, String>(0)
                })
                .unwrap(),
                expected,
                "{path}"
            );
        }
        let (arity, flags): (i64, i64) = db
            .query_row(
                "select narg,flags from pragma_function_list where name='ra_writing_note_title_v1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(arity, 1);
        let required = FunctionFlags::SQLITE_DETERMINISTIC | FunctionFlags::SQLITE_INNOCUOUS;
        assert_eq!(
            flags & i64::from(required.bits()),
            i64::from(required.bits())
        );
    }

    #[test]
    fn active_function_replacement_preserves_sqlite_failure_and_can_retry() {
        let db = Connection::open_in_memory().unwrap();
        register_functions(&db).unwrap();
        let mut statement = db
            .prepare("select ra_writing_note_title_v1('folder/Note.ranote')")
            .unwrap();
        let mut rows = statement.query([]).unwrap();
        assert!(rows.next().unwrap().is_some());
        assert!(matches!(
            register_functions(&db),
            Err(rusqlite::Error::SqliteFailure(error, _))
                if error.code == rusqlite::ErrorCode::DatabaseBusy
        ));
        drop(rows);
        drop(statement);
        register_functions(&db).unwrap();
    }
}
