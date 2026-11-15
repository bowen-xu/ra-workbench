//! Fixed synthetic data for this development CLI, outside production admission.
use super::*;
use crate::runtime_plugins::literature_attachments::{AttachmentRequest, AttachmentResult};
use research_workbench_native_sdk::{catalog::CatalogAttachment, catalog_save::CatalogDraft};
use uuid::Uuid;

pub(super) const AUTHORIZED: &str = "L09 authorized first-page evidence 汉字 α";
pub(super) const OTHER_PAGE: &str = "L09 ungranted second-page evidence";
pub(super) const OTHER_FILE: &str = "L09 ungranted attachment evidence";
const NOTE_BODY: &str = "L09 retained Core Note body before structured source reads.";

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Attachment {
    pub id: String,
    pub sha256: String,
    pub parse_run_id: String,
    pub parse_json_sha256: String,
    pub relative_path: String,
    pub byte_size: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Seed {
    pub paper_id: String,
    pub selected: Attachment,
    pub unselected: Attachment,
    pub note_id: String,
    pub note_document: Value,
}

pub(super) async fn prepare(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    directory: &Path,
) -> Result<Seed, Error> {
    let draft: CatalogDraft = serde_json::from_value(json!({
        "id":null,"itemType":"journalArticle","title":"L09 synthetic authorized reading",
        "issuedYear":2026,"creators":[],"tags":[]
    }))?;
    let paper = service
        .save_catalog(store.shared_agent_connection(), key.into(), draft)
        .await?;
    let selected = attach(store, service, key, directory, &paper.id, "selected").await?;
    let unselected = attach(store, service, key, directory, &paper.id, "unselected").await?;
    let note_id = Uuid::new_v4().to_string();
    let mut db = store.connection()?;
    let tx = db.transaction()?;
    crate::notes::create_page_with_paper_in(
        &tx,
        crate::notes::NotePageDraft {
            title: "L09 retained Core Note".into(),
            parent_page_id: None,
            parent_block_position: None,
        },
        Some(&paper.id),
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
    let note_document = serde_json::to_value(crate::notes::load_document_value(&db, &note_id)?)?;
    if !note_document.to_string().contains(NOTE_BODY) {
        return Err("synthetic Core Note body was not persisted".into());
    }
    Ok(Seed {
        paper_id: paper.id,
        selected,
        unselected,
        note_id,
        note_document,
    })
}

async fn attach(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    directory: &Path,
    paper: &str,
    label: &str,
) -> Result<Attachment, Error> {
    let source = directory.join(format!("{label}.pdf"));
    let bytes = pdf(label);
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&source)?;
    output.write_all(&bytes)?;
    output.sync_all()?;
    drop(output);
    let expected_hash = format!("{:x}", Sha256::digest(&bytes));
    let result = service
        .attachments(
            store.clone(),
            key.into(),
            AttachmentRequest::Attach {
                item_id: paper.into(),
                path: source
                    .to_str()
                    .ok_or("synthetic PDF path is not UTF-8")?
                    .into(),
            },
        )
        .await?;
    let AttachmentResult::Attachment(attachment) = result else {
        return Err("synthetic PDF attachment result kind differs".into());
    };
    if attachment.content_sha256 != expected_hash {
        return Err("real PDF CAS hash differs from its source bytes".into());
    }
    seed_parse(
        store,
        paper,
        &attachment,
        label == "selected",
        bytes.len() as u64,
    )
}

fn seed_parse(
    store: &AppStore,
    paper: &str,
    attachment: &CatalogAttachment,
    selected: bool,
    byte_size: u64,
) -> Result<Attachment, Error> {
    let db = store.connection()?;
    let (relative_path, stored_size): (String, u64) = db.query_row(
        "select relative_path,byte_size from content_objects where sha256=?1",
        [&attachment.content_sha256],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let path =
        crate::content::StoredObjectRead::resolve(store.content_directory(), &relative_path)?
            .into_legacy_path();
    let bytes = fs::read(path)?;
    if stored_size != byte_size
        || bytes.len() as u64 != byte_size
        || format!("{:x}", Sha256::digest(&bytes)) != attachment.content_sha256
    {
        return Err("real stored PDF bytes/metadata do not match their attachment".into());
    }
    let document = json!({
        "pages":{"1":{"page_no":1,"size":{"width":612,"height":792}},"2":{"page_no":2,"size":{"width":612,"height":792}}},
        "texts":[
            {"self_ref":"#/texts/0","text":if selected { AUTHORIZED } else { OTHER_FILE },
                "prov":[{"page_no":1,"bbox":{"l":20,"t":40,"r":120,"b":80},"charspan":[0,12]}]},
            {"self_ref":"#/texts/1","text":OTHER_PAGE,
                "prov":[{"page_no":2,"bbox":{"l":20,"t":40,"r":120,"b":80}}]}
        ],
        "tables":[{"self_ref":"#/tables/0","data":{"grid":[["合成表格","α"]]},
            "prov":[{"page_no":1,"bbox":{"l":13,"t":140,"r":501,"b":220}}]}]
    });
    let parse_run_id = Uuid::new_v4().to_string();
    let parse_json = serde_json::to_string(&document)?;
    let parse_json_sha256 = format!("{:x}", Sha256::digest(parse_json.as_bytes()));
    // This fresh-only development fixture seeds an already parsed cache. The
    // columns and JSON/UUID/time encoding mirror the original Docling writer.
    db.execute(
        "insert into docling_parse_runs (
             id, item_id, attachment_id, content_sha256, service_base_url,
             docling_status, parse_status, json_content, raw_response, error_message,
             processing_time_seconds, created_at, finished_at
         )
         values (
             ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
             strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
             strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         )",
        rusqlite::params![
            parse_run_id,
            paper,
            attachment.id,
            attachment.content_sha256,
            "synthetic-structured-source-cache",
            "success",
            "success",
            parse_json,
            serde_json::to_string(&json!({"status":"success","fixture":true}))?,
            Option::<String>::None,
            Option::<f64>::None
        ],
    )?;
    Ok(Attachment {
        id: attachment.id.clone(),
        sha256: attachment.content_sha256.clone(),
        parse_run_id,
        parse_json_sha256,
        relative_path,
        byte_size,
    })
}

/// Fixed two-page PDF, reusing the existing attachment fixture's object/xref layout.
fn pdf(label: &str) -> Vec<u8> {
    let streams = [
        format!("BT /F1 12 Tf 20 740 Td ({label} page one) Tj ET\n"),
        format!("BT /F1 12 Tf 20 740 Td ({label} page two) Tj ET\n"),
    ];
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 7 0 R >> >> /Contents 4 0 R >>".into(),
        format!("<< /Length {} >>\nstream\n{}endstream", streams[0].len(), streams[0]),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 7 0 R >> >> /Contents 6 0 R >>".into(),
        format!("<< /Length {} >>\nstream\n{}endstream", streams[1].len(), streams[1]),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
    ];
    let mut bytes = b"%PDF-1.4\n%synthetic\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
    }
    let xref = bytes.len();
    bytes.extend_from_slice(b"xref\n0 8\n0000000000 65535 f \n");
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!("trailer\n<< /Size 8 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    bytes
}
