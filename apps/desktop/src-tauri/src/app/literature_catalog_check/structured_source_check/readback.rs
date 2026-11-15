use super::*;
use crate::ai::paper_reading_contracts::PaperContextSnapshot;
use research_agent_core::text_patch::address::{SourceRoot, object};

#[derive(Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct State {
    pub snapshot: PaperContextSnapshot,
    proof_path: String,
    proof_version: String,
    proof_json: String,
}

pub(super) fn verify_prefix(store: &AppStore, seed: &fixture::Seed) -> Result<(), Error> {
    let db = store.connection()?;
    let note = serde_json::to_value(crate::notes::load_document_value(&db, &seed.note_id)?)?;
    if note != seed.note_document {
        return Err("structured source reading changed the pre-existing nonempty Core Note".into());
    }
    for attachment in [&seed.selected, &seed.unselected] {
        let (paper, sha, relative, size): (String, String, String, u64) = db.query_row(
            "select a.item_id,a.content_sha256,o.relative_path,o.byte_size from attachments a join content_objects o on o.sha256=a.content_sha256 where a.id=?1",
            [&attachment.id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        if paper != seed.paper_id
            || sha != attachment.sha256
            || relative != attachment.relative_path
            || size != attachment.byte_size
        {
            return Err(
                "structured source reading changed original attachment/CAS metadata".into(),
            );
        }
        let path = crate::content::StoredObjectRead::resolve(store.content_directory(), &relative)?
            .into_legacy_path();
        let bytes = fs::read(path)?;
        if bytes.len() as u64 != size || format!("{:x}", Sha256::digest(&bytes)) != sha {
            return Err("original CAS PDF bytes changed".into());
        }
        let (parse_paper, parse_attachment, parse_sha, status, raw): (String, String, String, String, String) = db.query_row(
            "select item_id,attachment_id,content_sha256,parse_status,json_content from docling_parse_runs where id=?1",
            [&attachment.parse_run_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )?;
        if parse_paper != seed.paper_id
            || parse_attachment != attachment.id
            || parse_sha != attachment.sha256
            || status != "success"
            || format!("{:x}", Sha256::digest(raw.as_bytes())) != attachment.parse_json_sha256
        {
            return Err("saved synthetic Docling cache identity or bytes changed".into());
        }
    }
    Ok(())
}

pub(super) fn capture(
    store: &AppStore,
    resources: &PaperContextResources,
    seed: &fixture::Seed,
    record: &SubmissionRecord,
    output: &Value,
) -> Result<State, Error> {
    verify_prefix(store, seed)?;
    if output["nextChar"].is_number() || output["turnId"] != record.turn_id.to_string() {
        return Err("source tool output was truncated or bound to another turn".into());
    }
    let text = output["text"]
        .as_str()
        .ok_or("source tool returned no text")?;
    if !text.contains(fixture::AUTHORIZED)
        || text.contains(fixture::OTHER_PAGE)
        || text.contains(fixture::OTHER_FILE)
    {
        return Err("source tool output widened or lost the authorized first page".into());
    }
    let frozen: Value = serde_json::from_str(text)?;
    if frozen["kind"] != "frozen_structured_evidence_v1" {
        return Err("source tool did not return its original frozen evidence view".into());
    }
    let snapshot_id = frozen["snapshotId"]
        .as_str()
        .ok_or("frozen snapshot identity missing")?;
    let db = store.connection()?;
    let snapshot = resources
        .load_snapshot(&db, snapshot_id)
        .map_err(|error| format!("original snapshot readback failed: {}", error.failure.error))?;
    if snapshot.paper_id != seed.paper_id
        || snapshot.attachments.len() != 1
        || snapshot.items.len() != 2
        || snapshot.context_schema_version != 1
        || snapshot.id != format!("paper-context-{}", snapshot.context_hash)
    {
        return Err("frozen snapshot paper/schema/item identities differ".into());
    }
    let attachment = &snapshot.attachments[0];
    if attachment.attachment_id != seed.selected.id
        || attachment.content_sha256 != seed.selected.sha256
        || attachment.parse_run_id != seed.selected.parse_run_id
        || attachment.ordinal != 0
    {
        return Err("snapshot did not bind the exact authorized PDF/parse".into());
    }
    let items = frozen["items"]
        .as_array()
        .ok_or("frozen evidence items missing")?;
    if items.len() != snapshot.items.len() {
        return Err("returned evidence and stored snapshot item sets differ".into());
    }
    for item in &snapshot.items {
        if item.attachment_id != seed.selected.id
            || item.content_sha256 != seed.selected.sha256
            || item.parse_run_id != seed.selected.parse_run_id
            || item.page_index != Some(0)
            || item.regions.is_empty()
            || item.regions.iter().any(|region| region.page_index != 0)
        {
            return Err("snapshot item widened its PDF, parse or page scope".into());
        }
        let matches = items
            .iter()
            .filter(|value| value["evidenceItemId"] == item.id)
            .collect::<Vec<_>>();
        let [returned] = matches.as_slice() else {
            return Err("returned evidence item identity is missing or repeated".into());
        };
        let quote = item
            .text
            .clone()
            .or_else(|| item.table_json.as_ref().map(Value::to_string))
            .or_else(|| item.label.clone())
            .unwrap_or_default();
        if returned["attachmentId"] != item.attachment_id
            || returned["attachmentSha256"] != item.content_sha256
            || returned["parseRunId"] != item.parse_run_id
            || returned["rawRef"] != item.raw_ref
            || returned["pageIndex"] != 0
            || returned["regions"] != serde_json::to_value(&item.regions)?
            || returned["kind"] != item.kind
            || returned["quoteExact"] != quote
        {
            return Err(
                "returned evidence identity, geometry or text differs from the snapshot".into(),
            );
        }
    }
    let mut statement = db.prepare("select path,version,snapshot_json from agent_text_read_scopes where conversation_id=?1 and scope_request_id=?2 order by path")?;
    let rows = statement
        .query_map(
            rusqlite::params![
                record.conversation_id.to_string(),
                record.request_id.to_string()
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;
    let [(proof_path, proof_version, proof_json)] = rows.as_slice() else {
        return Err("original structured read proof is missing or duplicated".into());
    };
    let proof: Value = serde_json::from_str(proof_json)?;
    if proof_path != &object(SourceRoot::Library, std::slice::from_ref(&seed.paper_id))
        || proof_version != snapshot_id
        || proof
            != json!({"kind":"structuredEvidenceRead",
            "paperId":seed.paper_id,"snapshotId":snapshot_id,"scopes":[[seed.selected.id,0]]})
    {
        return Err(
            "original read proof does not match this submission, scope and returned snapshot"
                .into(),
        );
    }
    let count: i64 = db.query_row(
        "select count(*) from paper_context_snapshots where paper_id=?1",
        [&seed.paper_id],
        |row| row.get(0),
    )?;
    if count != 1 {
        return Err("repeated source reading created another snapshot".into());
    }
    Ok(State {
        snapshot,
        proof_path: proof_path.clone(),
        proof_version: proof_version.clone(),
        proof_json: proof_json.clone(),
    })
}
