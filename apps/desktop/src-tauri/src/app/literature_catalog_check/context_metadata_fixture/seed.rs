use super::*;
use crate::structured_resources::{self, WriteDraft};
use research_workbench_native_sdk::catalog_save::CatalogDraft;

const TITLE: &str = "L09 metadata fixture_marker Ω";
const NOTE_BODY: &str = "Core reading Note fixture_marker Ω, seeded synthetic business content.";

pub(super) async fn prepare(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
) -> Result<Value, Error> {
    if store.paper_metadata_source().is_some()
        || store.paper_reading_source().is_some()
        || store.generation_policy_source().is_some()
    {
        return Err("context metadata fixture unexpectedly has a static domain source".into());
    }
    let before = empty_agent_and_binary_state(store)?;
    let draft: CatalogDraft = serde_json::from_value(json!({
        "id":null,"itemType":"journalArticle","title":TITLE,"issuedYear":2026,
        "abstractText":"Explicit synthetic metadata for ordinary context reads.",
        "url":"https://example.invalid/context-fixture","language":"zh",
        "creators":[{"familyName":"Synthetic","givenName":"Researcher Ω","literalName":"","role":"author"}],
        "tags":[]
    }))?;
    let paper = service
        .save_catalog(store.shared_agent_connection(), key.into(), draft)
        .await?;
    let paper_id = uuid::Uuid::parse_str(&paper.id)?.to_string();
    let note_id = uuid::Uuid::new_v4().to_string();
    let (note, portrait) = {
        let mut db = store.connection()?;
        let tx = db.transaction()?;
        crate::notes::create_page_with_paper_in(
            &tx,
            crate::notes::NotePageDraft {
                title: "L09 Core reading Note".into(),
                parent_page_id: None,
                parent_block_position: None,
            },
            Some(&paper_id),
            Some(&note_id),
        )?;
        let mut document = crate::notes::load_document_value(&tx, &note_id)?;
        document.blocks[0].content =
            json!({"kind":"inline","items":[{"type":"text","text":NOTE_BODY,"styles":[]}]});
        crate::notes::save_document_in(
            &tx,
            crate::notes::NoteDocumentSaveRequest {
                document,
                expected_revision: 0,
                save_reason: "manual".into(),
            },
        )?;
        tx.commit()?;
        let note = serde_json::to_value(crate::notes::load_document_value(&db, &note_id)?)?;
        let portrait = structured_resources::ensure_paper_resources(&db, &paper_id)?
            .into_iter()
            .find(|resource| resource.kind == "portrait")
            .ok_or("Core portrait resource was not created")?;
        (note, portrait)
    };
    let content = json!({"kind":"paper_portrait_v1","schemaVersion":1,
        "coverage":"Seeded synthetic business portrait fixture_marker Ω; no scientific conclusion is asserted.",
        "claims":[
            {"id":"claim_genre","dimension":"genre","label":"不确定","description":"Synthetic uncertain genre.","certainty":"uncertain","evidence":[]},
            {"id":"claim_mode","dimension":"mode","label":"不确定","description":"Synthetic uncertain mode.","certainty":"uncertain","evidence":[]},
            {"id":"claim_paradigm","dimension":"paradigm","label":"不确定","description":"Synthetic uncertain paradigm.","certainty":"uncertain","evidence":[]}
        ],"template":[{"id":"fixture_question","title":"Synthetic question","question":"What is available in this synthetic source?","claimIds":["claim_genre"]}]
    });
    let request = format!("context-metadata-fixture-{}", uuid::Uuid::new_v4());
    let written = service
        .write_resource(
            store.clone(),
            WriteDraft {
                resource_id: portrait.id.clone(),
                expected_revision: portrait.revision,
                content: serde_json::to_string(&content)?,
                request_id: request,
                conversation_id: None,
                expected_draft_revision: None,
                source_snapshot_id: None,
                submission_id: None,
                editor_kind: "human".into(),
            },
        )
        .await
        .map_err(|failure| serde_json::to_string(&failure).unwrap_or(failure.message))?;
    if written.status != "applied"
        || !written.errors.is_empty()
        || written.resource.content.as_ref() != Some(&content)
    {
        return Err("synthetic portrait was not applied by the original structured writer".into());
    }
    let after = empty_agent_and_binary_state(store)?;
    let db = store.connection()?;
    let linked: bool = db.query_row("select exists(select 1 from note_links where source_page_id=?1 and source_block_id is null and target_kind='paper' and target_id=?2 and relation_type='derived_from')",rusqlite::params![note_id,paper_id],|row|row.get(0))?;
    let note_after = serde_json::to_value(crate::notes::load_document_value(&db, &note_id)?)?;
    let portrait_after = structured_resources::get(&db, &portrait.id)?;
    if !linked
        || note_after != note
        || !note.to_string().contains(NOTE_BODY)
        || portrait_after.content.as_ref() != Some(&content)
    {
        return Err("prepared business identity or read-back content differs".into());
    }
    Ok(
        json!({"seededBusinessFixture":true,"paperId":paper_id,"paperTitle":TITLE,"paper":paper,
        "noteId":note_id,"noteDocument":note,"portraitId":portrait.id,"portrait":portrait_after,
        "query":"fixture_marker","agentAndBinaryBefore":before,"agentAndBinaryAfter":after,
        "preparationOwners":{"catalog":"original selected Native catalog save","note":"Core UUID create/load/save with paper link",
            "portrait":"Core ensure and original human structured writer; seeded content"},
        "expectedGuiScope":"Authorize metadata, paper Note and current portrait only; start the real ordinary Agent session through the UI."}),
    )
}
fn empty_agent_and_binary_state(store: &AppStore) -> Result<Value, Error> {
    let db = store.connection()?;
    let mut values = serde_json::Map::new();
    for table in [
        "agent_conversations",
        "agent_submissions",
        "agent_items",
        "agent_context_configurations",
        "agent_text_read_scopes",
        "docling_parse_runs",
        "paper_context_snapshots",
        "attachments",
        "content_objects",
    ] {
        let count: i64 = db.query_row(&format!("select count(*) from {table}"), [], |row| {
            row.get(0)
        })?;
        if count != 0 {
            return Err(
                format!("fresh business preparation unexpectedly contains {table} rows").into(),
            );
        }
        values.insert(table.into(), json!(count));
    }
    Ok(Value::Object(values))
}
