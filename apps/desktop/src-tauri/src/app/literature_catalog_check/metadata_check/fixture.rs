use super::*;
use research_workbench_native_sdk::catalog_save::CatalogDraft;
use rusqlite::params;

pub(super) const MAIN: &str = "metadata-cli-main";
pub(super) const FIELDS: &str = "metadata-cli-fields";
pub(super) const PARTIAL: &str = "metadata-cli-partial";
pub(super) const NOTE: &str = "metadata-cli-core-note";
const BODY: &str = "Preserved synthetic metadata Core note body";

pub(super) fn draft(title: &str) -> CatalogDraft {
    serde_json::from_value(json!({"itemType":"journalArticle","title":title,"issuedYear":2026}))
        .expect("fixed synthetic metadata draft")
}

pub(super) fn seed(store: &AppStore) -> Result<Value, Error> {
    let mut db = store.connection()?;
    let tx = db.transaction()?;
    for (id, title) in [
        (MAIN, "Original title"),
        (FIELDS, "Fields baseline"),
        (PARTIAL, "Partial baseline"),
    ] {
        tx.execute("insert into bibliographic_items(id,item_type,title,issued_year,created_at,updated_at) values(?1,'journalArticle',?2,2026,'2026','2026')",params![id,title])?;
        tx.execute(
            "insert into literature_search(item_id,title) values(?1,?2)",
            params![id, title],
        )?;
    }
    tx.execute("insert into identifiers(id,item_id,kind,normalized_value,display_value) values('metadata-cli-original-doi',?1,'doi','10.1000/original','10.1000/original')",[MAIN])?;
    for field in ["title", "volume"] {
        tx.execute("insert into metadata_provenance(item_id,field_name,source_kind,source_reference,confirmed_by_user,updated_at) values(?1,?2,'crossref','original-reference',0,'2026')",params![MAIN,field])?;
    }
    crate::notes::create_page_with_paper_in(
        &tx,
        crate::notes::NotePageDraft {
            title: "Synthetic metadata Core note".into(),
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
    tx.execute_batch(
        "create table synthetic_metadata_cli_events(seq integer primary key,phase text not null);",
    )?;
    tx.commit()?;
    drop(db);
    let original = note(store)?;
    if original["blocks"][0]["content"]["items"][0]["text"] != BODY {
        return Err("Core note writer lost the synthetic metadata body".into());
    }
    Ok(original)
}

pub(super) fn note(store: &AppStore) -> Result<Value, Error> {
    let db = store.connection()?;
    Ok(serde_json::to_value(crate::notes::load_document_value(
        &db, NOTE,
    )?)?)
}

pub(super) fn candidate(
    store: &AppStore,
    id: &str,
    item: Option<&str>,
    draft_json: &str,
    with_job: bool,
) -> Result<(), Error> {
    let mut db = store.connection()?;
    let tx = db.transaction()?;
    let result_id = if with_job {
        let job = format!("{id}-job");
        let result = format!("{id}-result");
        // No job is scheduled. A stable original state avoids claiming cold
        // migration recovery as an unchanged running/queued job.
        tx.execute("insert into import_jobs(id,source_kind,state,total_count,completed_count,created_at,updated_at) values(?1,'doi','interrupted',1,0,'2026','2026')",[&job])?;
        tx.execute("insert into import_results(id,job_id,input_label,state,raw_input,created_at,updated_at) values(?1,?2,'Synthetic metadata DOI','pending','10.1000/candidate','2026','2026')",params![result,job])?;
        Some(result)
    } else {
        None
    };
    tx.execute("insert into metadata_candidates(id,item_id,source_kind,source_reference,draft_json,raw_payload,import_result_id,state,created_at) values(?1,?2,'crossref','candidate-reference',?3,'{}',?4,'pending','2026')",params![id,item,draft_json,result_id])?;
    tx.commit()?;
    Ok(())
}
