use super::*;
use rusqlite::params;

pub(super) const NOTE: &str = "import-jobs-cli-core-note";
pub(super) const INDEPENDENT: &str = "import-cli-independent";
pub(super) const DOI: &str = "import-cli-doi-queued";
pub(super) const PDF: &str = "import-cli-pdf-running";
pub(super) const DONE: &str = "import-cli-done";
pub(super) const BUSINESS_FAILURE: &str = "synthetic original item-save failure";
pub(super) const PRIVATE: &str = "private synthetic import-job statement fault";
const BODY: &str = "Preserved synthetic import jobs Core note body";

pub(super) fn draft(id: &str, title: &str) -> CatalogDraft {
    serde_json::from_value(
        json!({"id":id,"itemType":"journalArticle","title":title,"issuedYear":2026}),
    )
    .expect("fixed synthetic import draft")
}

pub(super) fn note(store: &AppStore) -> Result<Value, Error> {
    let db = store.connection()?;
    Ok(serde_json::to_value(crate::notes::load_document_value(
        &db, NOTE,
    )?)?)
}

pub(super) fn seed(store: &AppStore) -> Result<Value, Error> {
    let mut db = store.connection()?;
    let tx = db.transaction()?;
    tx.execute(
        "insert into bibliographic_items(id,item_type,title,created_at,updated_at)
        values(?1,'document','Synthetic independent import material','2026','2026')",
        [INDEPENDENT],
    )?;
    tx.execute("insert into literature_search(item_id,title) values(?1,'Synthetic independent import material')",[INDEPENDENT])?;
    crate::notes::create_page_with_paper_in(
        &tx,
        crate::notes::NotePageDraft {
            title: "Synthetic import jobs Core note".into(),
            parent_page_id: None,
            parent_block_position: None,
        },
        None,
        Some(NOTE),
    )?;
    let mut document = crate::notes::load_document_value(&tx, NOTE)?;
    document.blocks[0].content =
        json!({"kind":"inline","items":[{"type":"text","text":BODY,"styles":[]}]});
    crate::notes::save_document_in(
        &tx,
        crate::notes::NoteDocumentSaveRequest {
            document,
            expected_revision: 0,
            save_reason: "manual".into(),
        },
    )?;
    for (id, source, state, total, completed, time) in [
        (DOI, "doi", "queued", 1, 0, "2020-01-03T00:00:00Z"),
        (PDF, "pdf", "running", 2, 1, "2020-01-02T00:00:00Z"),
        (DONE, "bibtex", "completed", 1, 1, "2020-01-01T00:00:00Z"),
    ] {
        tx.execute("insert into import_jobs(id,source_kind,state,total_count,completed_count,created_at,updated_at)
            values(?1,?2,?3,?4,?5,?6,?6)",params![id,source,state,total,completed,time])?;
    }
    for (id, job, label, state, item, raw) in [
        (
            "import-cli-doi-result",
            DOI,
            "  Original DOI input  ",
            "pending",
            None,
            None,
        ),
        (
            "import-cli-pdf-result",
            PDF,
            "Synthetic imported PDF",
            "imported",
            Some(INDEPENDENT),
            Some("{\"path\":\"synthetic.pdf\"}"),
        ),
        (
            "import-cli-pdf-pending",
            PDF,
            "Synthetic pending PDF",
            "pending",
            None,
            Some("legacy-synthetic.pdf"),
        ),
        (
            "import-cli-done-result",
            DONE,
            "Synthetic done import",
            "imported",
            Some(INDEPENDENT),
            Some("@article{synthetic}"),
        ),
    ] {
        tx.execute("insert into import_results(id,job_id,input_label,state,item_id,raw_input,created_at,updated_at)
            values(?1,?2,?3,?4,?5,?6,'2020','2020')",params![id,job,label,state,item,raw])?;
    }
    tx.execute_batch("create table synthetic_import_jobs_cli_events(seq integer primary key,phase text not null);")?;
    tx.commit()?;
    drop(db);
    let document = note(store)?;
    if document["blocks"][0]["content"]["items"][0]["text"] != BODY {
        return Err("Core writer lost the synthetic import note body".into());
    }
    Ok(document)
}

pub(super) fn clock(store: &AppStore) -> Result<String, Error> {
    Ok(store
        .connection()?
        .query_row("select strftime('%Y-%m-%dT%H:%M:%fZ','now')", [], |row| {
            row.get(0)
        })?)
}
