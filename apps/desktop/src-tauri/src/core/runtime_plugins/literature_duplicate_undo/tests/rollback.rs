use super::*;
use rusqlite::functions::FunctionFlags;

#[test]
fn late_mark_or_search_failure_rolls_back_real_source_catalog_and_note_state_on_cold_open() {
    for fail_search in [false, true] {
        let fixture = Fixture::new();
        let mut value = fixture.seed_nested();
        fixture.seed_reading_source();
        value["anchorIds"] = json!(["anchor"]);
        value["attachmentIds"] = json!(["attachment"]);
        fixture.seed_snapshot(&value);
        let before = fixture.state();
        let catalog_writes = Arc::new(AtomicUsize::new(0));
        let count = catalog_writes.clone();
        {
            let db = fixture.data.db.lock().unwrap();
            db.create_scalar_function(
                "observe_undo_source_catalog",
                0,
                FunctionFlags::SQLITE_UTF8,
                move |_| {
                    count.fetch_add(1, Ordering::SeqCst);
                    Ok(0)
                },
            )
            .unwrap();
            db.execute_batch(
                "create temp trigger observe_undo_source_catalog after update on note_page_catalog
                when old.source_label is not new.source_label
                begin select observe_undo_source_catalog();end;",
            )
            .unwrap();
            if !fail_search {
                db.execute_batch("create temp trigger fail_late_undo before update of state on merge_operations
                    when new.state='undone' begin select raise(abort,'synthetic late undo failure');end;").unwrap();
            }
        }
        if fail_search {
            // FTS virtual tables cannot have ordinary triggers. Deny the actual
            // original DELETE only after all earlier undo writes have happened.
            fixture.data.db.lock().unwrap().authorizer(Some(
                |context: AuthContext<'_>| match context.action {
                    AuthAction::Delete {
                        table_name: "literature_search",
                    } => Authorization::Deny,
                    _ => Authorization::Allow,
                },
            ));
        }
        let failure = fixture.apply().unwrap_err();
        clear_authorizer(&fixture);
        assert_eq!(failure.problem.code(), "storage_error");
        assert!(matches!(
            failure.problem.phase,
            UndoPhase::Restore | UndoPhase::Search
        ));
        assert_eq!(failure.outcome, UndoOutcome::RolledBack);
        assert!(failure.rollback_error.is_none());
        // This probe observes the real deployed reading-source/catalog trigger,
        // rather than merely checking an unchanged document after an early exit.
        assert!(catalog_writes.load(Ordering::SeqCst) > 0);
        assert_eq!(fixture.state(), before, "search failure: {fail_search}");
        assert_eq!(
            fixture.reopen().state(),
            before,
            "search failure: {fail_search}"
        );
    }
}
