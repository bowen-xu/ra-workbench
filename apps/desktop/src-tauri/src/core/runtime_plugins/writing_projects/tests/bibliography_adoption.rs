//! The production transaction body with actual independently built decisions.
use super::*;
use crate::{
    agent::writing_error::WritingError,
    ai::structured_resource_validation::Flow,
    data_interface::guarded_transaction,
    runtime_plugins::writing_projects::files::bibliography_adoption::{
        AdoptionBatch, AdoptionSql, adopt_in,
    },
};
use research_workbench_native_sdk::writing_bibliography::adoption::{
    AdoptionDecision, AdoptionInput, ImportedFile,
};
use research_workbench_native_sdk::writing_project::ManuscriptFile;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};

fn statements(f: &Fixture) -> [String; 4] {
    let package = package::accept(&f.catalog, &f.key, NativeProfile::WritingProjects).unwrap();
    [
        (
            "registered",
            "cc37859ff466c406a221cb98ffe104c99b5cb269f483f71b76f9913810aa07c4",
        ),
        (
            "insert-file",
            "2c4bd53fee141211551ad9ac4d90cce6f3f159629d38d41db4a69382da864f9f",
        ),
        (
            "insert-entry",
            "c3df55e05f2dcfc959470f71fe0f917df9020df750325137bae86bab53932242",
        ),
        (
            "insert-member",
            "9471a188ce2b00510bc9dc248b9853b0e288027f424c38c5ca937ec857901d26",
        ),
    ]
    .map(|(name, hash)| {
        crate::runtime_plugins::data::load_text(
            &f.catalog,
            &package,
            "writing_projects",
            &format!("data/project-bibliography-adoption-{name}.v1.sql"),
            hash,
        )
        .unwrap()
    })
}

fn sql(text: &[String; 4]) -> AdoptionSql<'_> {
    AdoptionSql {
        registered: &text[0],
        insert_file: &text[1],
        insert_entry: &text[2],
        insert_member: &text[3],
    }
}

async fn decide(f: &Fixture, input: AdoptionInput) -> AdoptionDecision {
    let input = Input::BibliographyAdoption {
        input: Box::new(input),
    };
    let decided = f
        .service
        .project_decision(f.key.clone(), input.clone())
        .await
        .unwrap();
    assert_eq!(decided.output.input, input);
    let Decision::BibliographyAdoption { decision } = decided.output.decision else {
        panic!("wrong actual Native adoption phase");
    };
    *decision
}

async fn prepare(f: &Fixture, store: &AppStore, text: &str) -> Vec<ImportedFile> {
    store
        .manuscript_files(&f.active)
        .save("source.bib", text, None)
        .unwrap();
    let AdoptionDecision::Parsed { files } = decide(
        f,
        AdoptionInput::Parse {
            physical_files: vec![("source.bib".into(), text.into())],
            originals: None,
            occupied: vec![],
        },
    )
    .await
    else {
        panic!("parse");
    };
    files
}

fn counts(db: &rusqlite::Connection, id: &str) -> [i64; 3] {
    [
        "manuscript_bibliography_files",
        "manuscript_bibliography_entries",
        "manuscript_bibliography_members",
    ]
    .map(|table| {
        db.query_row(
            &format!("select count(*) from {table} where manuscript_id=?1"),
            [id],
            |row| row.get(0),
        )
        .unwrap()
    })
}

async fn adopt(
    f: &Fixture,
    store: &AppStore,
    parsed: &[ImportedFile],
    files: &[ManuscriptFile],
    register_before_retry: bool,
) -> (Result<(), WritingError>, usize) {
    let text = statements(f);
    let sql = sql(&text);
    let mut cache: Vec<(AdoptionInput, AdoptionDecision)> = Vec::new();
    let mut waits = 0;
    loop {
        let mut cursor = 0;
        let attempt = {
            let mut guard = f.database.lock().unwrap();
            guarded_transaction::run(&mut guard, |db| {
                let attempt = adopt_in(
                    db,
                    &sql,
                    &AdoptionBatch {
                        manuscript_id: &f.active,
                        parsed,
                        originals: None,
                        files,
                    },
                    &mut |path| {
                        store
                            .manuscript_files(&f.active)
                            .read(path)
                            .map(|(_, hash)| hash)
                            .map_err(Into::into)
                    },
                    &mut |input| {
                        let index = cursor;
                        cursor += 1;
                        if let Some((known, decision)) = cache.get(index)
                            && *known == input
                        {
                            return Ok(decision.clone());
                        }
                        cache.truncate(index);
                        Err(Flow::Need(Box::new(input)))
                    },
                    &mut |_, _| Ok(None),
                );
                let unknown = attempt.requires_recovery;
                (Ok::<_, std::convert::Infallible>(attempt), unknown)
            })
            .unwrap()
        };
        assert!(!attempt.requires_recovery && attempt.rollback_error.is_none());
        match attempt.result {
            Ok(()) => return (Ok(()), waits),
            Err(Flow::Fail(failure)) => return (Err(failure.error), waits),
            Err(Flow::Need(input)) => {
                {
                    // File and preceding-entry INSERTs were rolled back. The
                    // actual owner can be acquired before the real Native wait.
                    let db = f.database.lock().unwrap();
                    assert!(db.is_autocommit());
                    assert_eq!(counts(&db, &f.active), [0, 0, 0]);
                }
                let input = *input;
                let decision = decide(f, input.clone()).await;
                waits += 1;
                if register_before_retry && waits == 1 {
                    let file = &parsed[0];
                    let hash = store
                        .manuscript_files(&f.active)
                        .read(&file.path)
                        .unwrap()
                        .1;
                    f.database
                        .lock()
                        .unwrap()
                        .execute(
                            sql.insert_file,
                            rusqlite::params![
                                f.active,
                                file.path,
                                file.original,
                                file.support,
                                hash,
                            ],
                        )
                        .unwrap();
                }
                cache.push((input, decision));
            }
        }
    }
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_adoption_transaction_rolls_back_every_native_wait_before_one_commit() {
    let f = Fixture::new();
    let store = files::store(&f);
    let source = "@article{alpha,title={Alpha}}\n@book{beta,title={Beta}}";
    let parsed = prepare(&f, &store, source).await;
    let (result, waits) = adopt(&f, &store, &parsed, &[], false).await;
    result.unwrap();
    assert_eq!(waits, 4);
    {
        let db = f.database.lock().unwrap();
        assert!(db.is_autocommit());
        assert_eq!(counts(&db, &f.active), [1, 2, 2]);
        let keys = db
            .prepare(
                "select citation_key from manuscript_bibliography_entries order by citation_key",
            )
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(keys, ["alpha", "beta"]);
        let orphans: i64 = db.query_row("select count(*) from manuscript_bibliography_members m left join manuscript_bibliography_entries e on e.id=m.entry_id where e.id is null", [], |row| row.get(0)).unwrap();
        assert_eq!(orphans, 0);
    }
    assert_eq!(
        store
            .manuscript_files(&f.active)
            .read("source.bib")
            .unwrap()
            .0,
        source
    );
    // Registration is checked before any read, decision or duplicate INSERT.
    let changes = f.database.lock().unwrap().total_changes();
    let (result, waits) = adopt(&f, &store, &parsed, &[], false).await;
    result.unwrap();
    assert_eq!(waits, 0);
    assert_eq!(f.database.lock().unwrap().total_changes(), changes);
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_adoption_transaction_rechecks_registration_after_native_wait() {
    let f = Fixture::new();
    let store = files::store(&f);
    let parsed = prepare(&f, &store, "@article{alpha,title={Alpha}}").await;
    let (result, waits) = adopt(&f, &store, &parsed, &[], true).await;
    result.unwrap();
    assert_eq!(waits, 1);
    assert_eq!(counts(&f.database.lock().unwrap(), &f.active), [1, 0, 0]);
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_adoption_transaction_keeps_file_sql_failure_before_migration_refusal() {
    let f = Fixture::new();
    let store = files::store(&f);
    let parsed = prepare(&f, &store, "@article{1,title={Legacy}}").await;
    let files = vec![ManuscriptFile {
        manuscript_id: f.active.clone(),
        relative_path: "main.tex".into(),
        kind: "tex".into(),
        size_bytes: 0,
        sha256: "original".into(),
        modified_at: "original".into(),
    }];
    f.database.lock().unwrap().execute_batch("create trigger fail_adoption before insert on manuscript_bibliography_files begin select raise(abort,'original-file-write'); end;").unwrap();
    let (result, waits) = adopt(&f, &store, &parsed, &files, false).await;
    assert!(matches!(result, Err(WritingError::Database(_))));
    assert_eq!(waits, 0);
    f.database
        .lock()
        .unwrap()
        .execute_batch("drop trigger fail_adoption")
        .unwrap();
    let (result, waits) = adopt(&f, &store, &parsed, &files, false).await;
    assert!(
        matches!(result, Err(WritingError::BibliographyImport(ref cause)) if cause.contains("需要迁移"))
    );
    assert_eq!(waits, 1);
    assert_eq!(counts(&f.database.lock().unwrap(), &f.active), [0, 0, 0]);
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_adoption_transaction_unconfirmed_rollback_never_yields_and_fences_owner() {
    let f = Fixture::new();
    let store = files::store(&f);
    let parsed = prepare(&f, &store, "@article{alpha,title={Alpha}}").await;
    let text = statements(&f);
    {
        let mut guard = f.database.lock().unwrap();
        guard.authorizer(Some(|ctx: AuthContext<'_>| match ctx.action {
            AuthAction::Transaction {
                operation: TransactionOperation::Rollback,
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }));
        let attempt = guarded_transaction::run(&mut guard, |db| {
            let attempt = adopt_in(
                db,
                &sql(&text),
                &AdoptionBatch {
                    manuscript_id: &f.active,
                    parsed: &parsed,
                    originals: None,
                    files: &[],
                },
                &mut |path| {
                    store
                        .manuscript_files(&f.active)
                        .read(path)
                        .map(|(_, hash)| hash)
                        .map_err(Into::into)
                },
                &mut |input| Err(Flow::Need(Box::new(input))),
                &mut |_, _| Ok(None),
            );
            let unknown = attempt.requires_recovery;
            (Ok::<_, std::convert::Infallible>(attempt), unknown)
        })
        .unwrap();
        assert!(attempt.requires_recovery);
        assert_eq!(
            attempt.rollback_error.unwrap().phase,
            "bibliography_adoption_rollback"
        );
        assert!(matches!(attempt.result, Err(Flow::Fail(_))));
        guard.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    }
    assert!(f.database.lock().is_err());
    f.service.shutdown().await.unwrap();
}
