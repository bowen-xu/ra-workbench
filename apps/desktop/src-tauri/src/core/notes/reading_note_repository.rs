use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use super::{
    error::NotesError,
    model::{
        NoteDocument, NoteDocumentBlock, NotePageMetadataUpdate, NoteStatusUpdate,
        ReadingNoteBlock, ReadingNoteDraft, ReadingNoteListQuery, ReadingNoteSource,
        ReadingNoteUpdate, reading_note_type_to_block_type, validate_status,
    },
};

fn string_list(connection: &Connection, sql: &str, id: &str) -> Result<Vec<String>, NotesError> {
    let mut statement = connection.prepare(sql)?;
    Ok(statement
        .query_map([id], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?)
}

fn source_state(
    note_type: &str,
    anchor_id: Option<&str>,
    paper_exists: bool,
    paper_trashed: bool,
    current_sha256: Option<&str>,
    stored_sha256: &str,
) -> &'static str {
    if !paper_exists {
        return "unavailable";
    }
    if paper_trashed {
        return "paper_in_trash";
    }
    if anchor_id.is_none() {
        return "page_only";
    }
    if current_sha256.is_some_and(|value| value != stored_sha256) {
        return "needs_relocation";
    }
    if note_type == "page_note" {
        "page_only"
    } else {
        "exact"
    }
}

pub(crate) fn reading_note_from(
    connection: &Connection,
    id: &str,
) -> Result<ReadingNoteBlock, NotesError> {
    let mut note = reading_note_metadata_from(connection, id)?;
    let document = super::document_repository::load_document_value(connection, &note.page_id)?;
    fn collect(blocks: &[NoteDocumentBlock], output: &mut Vec<String>) {
        for block in blocks {
            output.push(super::document_repository::plain_text(&block.content));
            collect(&block.children, output);
        }
    }
    let mut lines = Vec::new();
    collect(&document.blocks, &mut lines);
    note.body = lines.join("\n");
    Ok(note)
}

/// Source/lifecycle projection for search results; never opens canonical blocks.
pub(crate) fn reading_note_metadata_from(
    connection: &Connection,
    id: &str,
) -> Result<ReadingNoteBlock, NotesError> {
    let mut note = connection.query_row(
        "select s.block_id, s.note_type, b.id, s.color, b.status, b.is_favorite,
                b.revision, b.created_at, b.updated_at, b.trashed_at,
                s.current_anchor_id, s.paper_id_snapshot, s.paper_title_snapshot,
                s.attachment_id_snapshot, s.content_sha256, s.page_index, s.page_label,
                s.rects_json, s.quote_exact, s.quote_prefix, s.quote_suffix, s.selector_json,
                exists(select 1 from bibliographic_items p where p.id = s.paper_id_snapshot),
                exists(select 1 from bibliographic_items p where p.id = s.paper_id_snapshot and p.trashed_at is not null),
                (select a.content_sha256 from attachments a where a.id = s.attachment_id_snapshot)
         from note_pages b join reading_note_sources s on s.page_id = b.id
         where s.block_id = ?1 or s.page_id = ?1",
        [id],
        |row| {
            let note_type: String = row.get(1)?;
            let page_id: String = row.get(2)?;
            let anchor_id: Option<String> = row.get(10)?;
            let selector = row.get::<_, Option<String>>(21)?
                .and_then(|value| serde_json::from_str(&value).ok());
            let paper_exists: bool = row.get(22)?;
            let paper_trashed: bool = row.get(23)?;
            let current_sha256: Option<String> = row.get(24)?;
            let stored_sha256: String = row.get(14)?;
            Ok(ReadingNoteBlock {
                id: row.get(0)?,
                note_type: note_type.clone(),
                page_id,
                body: String::new(),
                color: row.get(3)?,
                status: row.get(4)?,
                is_favorite: row.get::<_, i64>(5)? != 0,
                revision: row.get(6)?,
                source: ReadingNoteSource {
                    anchor_id: anchor_id.clone(), paper_id: row.get(11)?, paper_title: row.get(12)?,
                    attachment_id: row.get(13)?, content_sha256: stored_sha256.clone(), page_index: row.get(15)?,
                    page_label: row.get(16)?, rects: serde_json::from_str(&row.get::<_, String>(17)?).unwrap_or_else(|_| serde_json::json!([])),
                    quote_exact: row.get(18)?, quote_prefix: row.get(19)?, quote_suffix: row.get(20)?,
                    state: source_state(&note_type, anchor_id.as_deref(), paper_exists, paper_trashed, current_sha256.as_deref(), &stored_sha256).to_owned(),
                    selector,
                },
                project_ids: Vec::new(), tag_ids: Vec::new(), created_at: row.get(7)?, updated_at: row.get(8)?, trashed_at: row.get(9)?,
            })
        },
    ).optional()?.ok_or(NotesError::NotFound)?;
    note.project_ids = string_list(
        connection,
        "select project_id from project_note_pages where page_id = ?1 order by project_id",
        &note.page_id,
    )?;
    note.tag_ids = string_list(
        connection,
        "select tag_id from note_page_tags where page_id = ?1 order by tag_id",
        &note.page_id,
    )?;
    Ok(note)
}

pub(crate) fn list_reading_notes_from(
    connection: &Connection,
    query: Option<&ReadingNoteListQuery>,
) -> Result<Vec<ReadingNoteBlock>, NotesError> {
    let status = query
        .and_then(|value| value.status.as_deref())
        .unwrap_or("active");
    if !validate_status(status) {
        return Err(NotesError::InvalidInput("阅读笔记状态无效".to_owned()));
    }
    let project = query.and_then(|value| value.project_id.as_deref());
    let tag = query.and_then(|value| value.tag_id.as_deref());
    let paper = query.and_then(|value| value.paper_id.as_deref());
    let note_type = query.and_then(|value| value.note_type.as_deref());
    let block_type = note_type.and_then(reading_note_type_to_block_type);
    if note_type.is_some() && block_type.is_none() {
        return Err(NotesError::InvalidInput("阅读笔记类型无效".to_owned()));
    }
    let search = query
        .and_then(|value| value.search.as_deref())
        .unwrap_or("")
        .trim();
    let limit = query
        .and_then(|value| value.limit)
        .unwrap_or(500)
        .clamp(1, 1_000) as i64;
    let cursor = query
        .and_then(|value| value.cursor.as_deref())
        .unwrap_or("");
    let mut statement = connection.prepare(
        "select s.block_id from note_pages b join reading_note_sources s on s.page_id = b.id
         where b.status = ?1
           and (?2 is null or exists(select 1 from project_reading_notes x where x.block_id = s.block_id and x.project_id = ?2))
           and (?3 is null or exists(select 1 from reading_note_tags x where x.block_id = s.block_id and x.tag_id = ?3))
           and (?4 is null or s.paper_id_snapshot = ?4)
           and (?5 is null or s.note_type = ?5)
           and (?6 = '' or exists(select 1 from note_blocks t where t.page_id=b.id and t.plain_text like '%' || ?6 || '%') or s.quote_exact like '%' || ?6 || '%' or s.paper_title_snapshot like '%' || ?6 || '%')
           and (?7 = '' or s.block_id > ?7)
         order by s.block_id limit ?8",
    )?;
    let ids = statement
        .query_map(
            params![
                status, project, tag, paper, note_type, search, cursor, limit
            ],
            |row| row.get::<_, String>(0),
        )?
        .collect::<Result<Vec<_>, _>>()?;
    let mut notes = ids
        .iter()
        .map(|id| reading_note_from(connection, id))
        .collect::<Result<Vec<_>, _>>()?;
    if let Some(state) = query.and_then(|value| value.source_state.as_deref()) {
        notes.retain(|note| note.source.state == state);
    }
    Ok(notes)
}

pub(crate) fn create_reading_note_from(
    connection: &mut Connection,
    draft: ReadingNoteDraft,
) -> Result<ReadingNoteBlock, NotesError> {
    let _block_type = reading_note_type_to_block_type(&draft.note_type)
        .ok_or_else(|| NotesError::InvalidInput("阅读笔记类型无效".to_owned()))?;
    if draft.page_index < 0 {
        return Err(NotesError::InvalidInput("PDF 页码无效".to_owned()));
    }
    let transaction = connection.transaction()?;
    let (sha256, paper_title, attachment_name): (String, String, String) = transaction.query_row(
        "select a.content_sha256, b.title, a.display_name from attachments a join bibliographic_items b on b.id = a.item_id where a.id = ?1 and a.item_id = ?2",
        params![draft.attachment_id, draft.item_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).optional()?.ok_or(NotesError::NotFound)?;
    let block_id = draft.id.unwrap_or_else(|| Uuid::new_v4().to_string());
    let draft_rects = serde_json::to_string(&draft.rects)
        .map_err(|error| NotesError::InvalidInput(error.to_string()))?;
    let draft_quote_prefix = draft.quote_prefix.as_deref().unwrap_or("");
    let draft_quote_suffix = draft.quote_suffix.as_deref().unwrap_or("");
    let draft_selector_json = match draft.selector.as_ref() {
        Some(selector) if selector.is_valid_for(draft.page_index, &sha256, &draft.quote_exact) => {
            Some(
                serde_json::to_string(selector)
                    .map_err(|error| NotesError::InvalidInput(error.to_string()))?,
            )
        }
        Some(_) => {
            return Err(NotesError::InvalidInput(
                "SourceSelectorV2 与当前附件或页范围不匹配".to_owned(),
            ));
        }
        None => None,
    };
    let (anchor_id, page_label, rects, quote_exact, quote_prefix, quote_suffix, selector_json) =
        if let Some(anchor_id) = draft.source_anchor_id.as_deref() {
            let source = transaction
            .query_row(
                "select page_label, rects_json, quote_exact, quote_prefix, quote_suffix, selector_json
                 from source_anchors
                 where id = ?1 and item_id = ?2 and attachment_id = ?3
                   and content_sha256 = ?4 and page_index = ?5",
                params![
                    anchor_id,
                    draft.item_id,
                    draft.attachment_id,
                    sha256,
                    draft.page_index
                ],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .optional()?
            .ok_or_else(|| {
                NotesError::InvalidInput("所选来源锚点与当前附件版本或页码不匹配".to_owned())
            })?;
            (
                anchor_id.to_owned(),
                source.0,
                source.1,
                source.2,
                source.3,
                source.4,
                source.5,
            )
        } else {
            let anchor_id = Uuid::new_v4().to_string();
            transaction.execute(
                "insert into source_anchors (id, item_id, attachment_id, content_sha256, selector_version, page_index, page_label, rects_json, quote_exact, quote_prefix, quote_suffix, selector_json, created_at)
                 values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
                params![anchor_id, draft.item_id, draft.attachment_id, sha256, if draft_selector_json.is_some() { 2 } else { 1 }, draft.page_index, draft.page_label, draft_rects, draft.quote_exact, draft_quote_prefix, draft_quote_suffix, draft_selector_json],
            )?;
            (
                anchor_id,
                draft.page_label,
                draft_rects,
                draft.quote_exact,
                draft_quote_prefix.to_owned(),
                draft_quote_suffix.to_owned(),
                draft_selector_json,
            )
        };
    let color = draft.color.unwrap_or_else(|| "#f4cf65".to_owned());
    let page_id = Uuid::new_v4().to_string();
    let title: String = paper_title.chars().take(240).collect();
    transaction.execute("insert into note_pages(id,title,position,status,is_favorite,revision,schema_version,created_at,updated_at) values (?1,?2,(select coalesce(max(position),-1)+1 from note_pages), 'active',0,0,1,strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now'))", params![page_id, if title.is_empty() { "阅读笔记" } else { &title }])?;
    transaction.execute(
        "insert into reading_note_page_map(legacy_id,page_id) values (?1,?2)",
        params![block_id, page_id],
    )?;
    let document = NoteDocument { title: None, schema_version: 1, page_id: page_id.clone(), blocks: draft.body.split('\n').map(|text| NoteDocumentBlock {
        id: Uuid::new_v4().to_string(), block_type: "paragraph".to_owned(), props: serde_json::json!({}),
        content: serde_json::json!({"kind":"inline","items":[{"type":"text","text":text,"styles":[]}]}), children: Vec::new(),
    }).collect() };
    super::document_repository::write_blocks(
        &transaction,
        &page_id,
        &document.blocks,
        None,
        &mut std::collections::HashSet::new(),
    )?;
    transaction.execute(
        "insert into reading_note_sources
         (block_id, page_id, note_type, color, current_anchor_id, paper_id_snapshot, paper_title_snapshot, attachment_id_snapshot,
          attachment_name_snapshot, content_sha256, page_index, page_label, rects_json, quote_exact,
          quote_prefix, quote_suffix, selector_json, locator_version, created_at, updated_at)
         values (?1, ?16, ?17, ?18, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15,
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
        params![block_id, anchor_id, draft.item_id, paper_title, draft.attachment_id, attachment_name, sha256, draft.page_index, page_label, rects, quote_exact, quote_prefix, quote_suffix, selector_json, if selector_json.is_some() { 2 } else { 1 }, page_id, draft.note_type, color],
    )?;
    transaction.execute(
        "update note_index_state set state = 'dirty' where id = 1",
        [],
    )?;
    transaction.commit()?;
    reading_note_from(connection, &block_id)
}

pub(crate) fn update_reading_note_from(
    connection: &mut Connection,
    request: ReadingNoteUpdate,
) -> Result<ReadingNoteBlock, NotesError> {
    let current = reading_note_from(connection, &request.id)?;
    // Body remains in the ordinary document; locator and color are source metadata.
    let transaction = connection.transaction()?;
    let page = super::page_repository::update_page_in(
        &transaction,
        NotePageMetadataUpdate {
            page_id: current.page_id.clone(),
            title: None,
            is_favorite: request.is_favorite,
            project_ids: request.project_ids,
            tag_ids: request.tag_ids,
            expected_revision: request.expected_revision,
        },
    )?;
    if let Some(position) = request.position {
        super::position_note::move_in(&transaction, &current, position)?;
    }
    if let Some(color) = request.color {
        transaction.execute(
            "update reading_note_sources set color=?2 where page_id=?1",
            params![page.id, color],
        )?;
    }
    transaction.commit()?;
    reading_note_from(connection, &request.id)
}

pub(crate) fn set_reading_note_status_from(
    connection: &mut Connection,
    request: NoteStatusUpdate,
) -> Result<ReadingNoteBlock, NotesError> {
    let current = reading_note_from(connection, &request.id)?;
    super::page_repository::set_page_status_from(
        connection,
        NoteStatusUpdate {
            id: current.page_id,
            status: request.status,
            expected_revision: request.expected_revision,
        },
    )?;
    reading_note_from(connection, &request.id)
}
