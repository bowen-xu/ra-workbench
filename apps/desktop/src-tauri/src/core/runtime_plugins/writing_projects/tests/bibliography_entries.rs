//! The actual Core reader, fixed package SQL and independently built Native host.
use super::*;
use crate::{
    agent::writing_error::WritingError,
    ai::structured_resource_validation::Flow,
    runtime_plugins::writing_projects::files::bibliography_entries::{self, EntrySql},
};
use research_workbench_native_sdk::writing_bibliography::{
    EntryDecision, EntryInput, ProjectBibliographyEntry,
};

fn statements(fixture: &Fixture) -> [String; 5] {
    let package = package::accept(
        &fixture.catalog,
        &fixture.key,
        NativeProfile::WritingProjects,
    )
    .unwrap();
    [
        (
            "bibliography-entry-rows",
            "4820c448f85cef277e58bd690901d45e9054381e005bc22d5f899aa5ec1030f3",
        ),
        (
            "bibliography-entry-support",
            "72c61bde518c9029149cae09ed43163563550f0c2714bef780a2ae810fd1a692",
        ),
        (
            "bibliography-entry-members",
            "28b2170c2acd48ad2c650daa714c2c9d17566d655a0b4c32218860e4427e9aa0",
        ),
        (
            "bibliography-entry-base",
            "e6e01ca46aab8f445507a07f1676c2aa8cac0081b06424c283ee0e676aa2424f",
        ),
        (
            "citation-bindings",
            "ad52c976fde4ee44b2acee01cec735854ce26c71a4f03b2bcd2fe6d12aace4f5",
        ),
    ]
    .map(|(name, hash)| {
        crate::runtime_plugins::data::load_text(
            &fixture.catalog,
            &package,
            "writing_projects",
            &format!("data/project-{name}.v1.sql"),
            hash,
        )
        .unwrap()
    })
}

fn seed(f: &Fixture, bibtex: &str, imported_json: &str) {
    f.database.lock().unwrap().execute(
        "insert into manuscript_bibliography_entries(id,manuscript_id,citation_key,original_key,original_bibtex,imported_draft_json,bibtex_entry) values('entry',?1,'StoredKey','OriginalKey','original',?2,?3)",
        rusqlite::params![f.active, imported_json, bibtex],
    ).unwrap();
}

/// This harness supplies owned replies to the same synchronous production reader.
/// It deliberately changes a consumed row after the first actual host reply.
async fn read(
    f: &Fixture,
    change_after_first_reply: bool,
) -> (Result<Vec<ProjectBibliographyEntry>, WritingError>, usize) {
    let text = statements(f);
    let sql = EntrySql {
        rows: &text[0],
        support: &text[1],
        members: &text[2],
        base: &text[3],
        bindings: &text[4],
    };
    let mut cache: Vec<(EntryInput, EntryDecision)> = Vec::new();
    let mut preparations = 0;
    let mut changed = false;
    loop {
        let mut cursor = 0;
        let result = {
            let db = f.database.lock().unwrap();
            bibliography_entries::read_in(
                &db,
                &sql,
                &f.active,
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
                &mut |_, _, _| Ok(None),
            )
        };
        match result {
            Ok(entries) => return (Ok(entries), preparations),
            Err(Flow::Fail(failure)) => return (Err(failure.error), preparations),
            Err(Flow::Need(input)) => {
                // The real database owner is available before the actual wait.
                assert!(f.database.lock().unwrap().is_autocommit());
                preparations += usize::from(matches!(*input, EntryInput::PrepareImported { .. }));
                let input = *input;
                let decided = f
                    .service
                    .project_decision(
                        f.key.clone(),
                        Input::BibliographyEntries {
                            input: Box::new(input.clone()),
                        },
                    )
                    .await
                    .unwrap();
                let Decision::BibliographyEntries { decision } = decided.output.decision else {
                    panic!("wrong actual Native stage");
                };
                assert_eq!(
                    decided.output.input,
                    Input::BibliographyEntries {
                        input: Box::new(input.clone())
                    }
                );
                if change_after_first_reply && !changed {
                    f.database.lock().unwrap().execute(
                        r#"update manuscript_bibliography_entries set imported_draft_json='{"revision":2}' where id='entry'"#,
                        [],
                    ).unwrap();
                    changed = true;
                }
                cache.push((input, *decision));
            }
        }
    }
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_entry_reader_replays_complete_current_rows_and_skips_duplicate_bindings() {
    let f = Fixture::new();
    seed(
        &f,
        "@article{StoredKey,title={Current title}}",
        "{\"revision\":1}",
    );
    {
        let db = f.database.lock().unwrap();
        for (source, key, bibtex) in [
            ("duplicate", "StoredKey", "broken BibTeX"),
            (
                "automatic",
                "ZKey",
                "@article{FirstKey,title={Automatic title}}",
            ),
        ] {
            db.execute("insert into bibliographic_items(id,item_type,title,created_at,updated_at) values(?1,'journalArticle',?1,'now','now')", [source]).unwrap();
            db.execute("insert into citation_bindings(manuscript_id,source_item_id,citation_key,bibtex_entry,state,pinned_at,updated_at) values(?1,?2,?3,?4,'missing_source','pinned','updated')", rusqlite::params![f.active,source,key,bibtex]).unwrap();
        }
        db.execute("insert into manuscript_citation_key_bases(manuscript_id,reference_id,base_key) values(?1,'entry','OriginalBase')", [&f.active]).unwrap();
    }
    let changes = f.database.lock().unwrap().total_changes();
    let (entries, preparations) = read(&f, true).await;
    let entries = entries.unwrap();
    assert_eq!(preparations, 2);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].id, "entry");
    assert_eq!(entries[0].manuscript_id, f.active);
    assert_eq!(entries[0].original_key, "OriginalKey");
    assert_eq!(entries[0].base_key.as_deref(), Some("OriginalBase"));
    assert_eq!(entries[0].imported, serde_json::json!({"revision":2}));
    assert_eq!(entries[0].current["title"], "Current title");
    assert_eq!(entries[1].id, "binding:automatic");
    assert_eq!(entries[1].citation_key, "ZKey");
    assert_eq!(entries[1].current["title"], "Automatic title");
    assert_eq!(entries[1].paths, ["references.bib"]);
    assert!(entries[1].library_item.is_none());
    assert_eq!(f.database.lock().unwrap().total_changes(), changes + 1);
    f.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_entry_reader_preserves_parse_before_sql_before_imported_json() {
    for (bibtex, expect_parse_error) in [
        ("@article{StoredKey,title={unclosed}", true),
        ("@article{StoredKey,title={Valid}}", false),
    ] {
        let f = Fixture::new();
        seed(&f, bibtex, "invalid JSON");
        f.database
            .lock()
            .unwrap()
            .execute("drop table manuscript_bibliography_members", [])
            .unwrap();
        let changes = f.database.lock().unwrap().total_changes();
        let (entries, _) = read(&f, false).await;
        let error = entries.unwrap_err();
        if expect_parse_error {
            assert!(
                matches!(error, WritingError::BibliographyImport(_)),
                "{error}"
            );
        } else {
            assert!(matches!(error, WritingError::Database(_)), "{error}");
        }
        assert_eq!(f.database.lock().unwrap().total_changes(), changes);
        f.service.shutdown().await.unwrap();
    }
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_entry_reader_preserves_existing_imported_json_values() {
    let f = Fixture::new();
    let imported_json = r#"{"numbers":[1.0000000000000002,0.12345678901234566,173.00000000000003,95.25979139581439,1.234e-15],"nested":{"text":"原文\r\n","null":null,"array":[true,false]}}"#;
    let expected: serde_json::Value = serde_json::from_str(imported_json).unwrap();
    seed(
        &f,
        "@article{StoredKey,title={Current title}}",
        imported_json,
    );
    let changes = f.database.lock().unwrap().total_changes();
    let (entries, _) = read(&f, false).await;
    assert_eq!(entries.unwrap()[0].imported, expected);
    {
        let db = f.database.lock().unwrap();
        let retained: String = db
            .query_row(
                "select imported_draft_json from manuscript_bibliography_entries where id='entry'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(retained, imported_json);
        assert_eq!(db.total_changes(), changes);
    }
    f.service.shutdown().await.unwrap();
}
