use super::{error::NotesError, model::NotePage, page_repository::page_from};
use rusqlite::{Connection, params};
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiteratureNoteSummary {
    pub page: NotePage,
    pub preview: String,
}

pub(crate) fn list_from(
    connection: &Connection,
    paper_id: &str,
) -> Result<Vec<LiteratureNoteSummary>, NotesError> {
    let mut statement = connection.prepare(
        "select p.id from note_pages p where p.status = 'active'
         and not exists(select 1 from reading_note_sources s where s.page_id = p.id
             and (s.note_type = 'highlight' or (s.note_type = 'annotation' and trim(s.quote_exact) <> '')))
         and exists(select 1 from note_links l where l.source_page_id = p.id and l.source_block_id is null
             and l.target_kind = 'paper' and l.target_id = ?1 and l.relation_type = 'derived_from')
         order by p.created_at desc, p.position desc, p.id desc",
    )?;
    let ids = statement
        .query_map([paper_id], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    ids.iter()
        .map(|id| {
            let preview: String = connection.query_row(
                "select substr(coalesce(group_concat(plain_text, char(10)), ''), 1, 300) from
             (select plain_text from note_blocks where page_id = ?1 and status = 'active'
              order by position, id limit 20)",
                params![id],
                |row| row.get(0),
            )?;
            Ok(LiteratureNoteSummary {
                page: page_from(connection, id)?,
                preview,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notes::{
        document_repository::save_document_from,
        model::{NoteDocumentSaveRequest, NotePageDraft, NoteStatusUpdate},
        page_creation::create_page_with_paper_from,
        page_repository::set_page_status_from,
        tests::migrated_connection,
    };

    #[test]
    fn creates_paper_notes_without_pdf_locators_and_preserves_links_after_save() {
        let mut connection = migrated_connection();
        connection.execute("insert into bibliographic_items(id,item_type,title,created_at,updated_at) values('paper','journalArticle','Synthetic item','now','now')", []).unwrap();
        let draft = || NotePageDraft {
            title: "文献笔记".to_owned(),
            parent_page_id: None,
            parent_block_position: None,
        };
        assert!(create_page_with_paper_from(&mut connection, draft(), Some("missing")).is_err());
        assert_eq!(
            connection
                .query_row("select count(*) from note_pages", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        let mut first =
            create_page_with_paper_from(&mut connection, draft(), Some("paper")).unwrap();
        let second = create_page_with_paper_from(&mut connection, draft(), Some("paper")).unwrap();
        assert_eq!(first.page.literature_paper_id.as_deref(), Some("paper"));
        let independent = create_page_with_paper_from(&mut connection, draft(), None).unwrap();
        assert_eq!(independent.page.literature_paper_id, None);
        assert_eq!(
            list_from(&connection, "paper").unwrap()[0].page.id,
            second.page.id
        );
        assert_eq!(
            connection
                .query_row("select count(*) from source_anchors", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        first.document.blocks[0].content = serde_json::json!({ "kind": "inline", "items": [{ "type": "text", "text": "Whole paper note", "styles": [] }] });
        let saved = save_document_from(
            &mut connection,
            NoteDocumentSaveRequest {
                document: first.document,
                expected_revision: first.page.revision,
                save_reason: "manual".to_owned(),
            },
        )
        .unwrap();
        assert!(
            list_from(&connection, "paper")
                .unwrap()
                .iter()
                .any(|note| note.page.id == first.page.id && note.preview == "Whole paper note")
        );
        assert!(list_from(&connection, "other").unwrap().is_empty());
        let trashed = set_page_status_from(
            &mut connection,
            NoteStatusUpdate {
                id: first.page.id.clone(),
                status: "trashed".to_owned(),
                expected_revision: saved.page.revision,
            },
        )
        .unwrap();
        assert_eq!(list_from(&connection, "paper").unwrap().len(), 1);
        set_page_status_from(
            &mut connection,
            NoteStatusUpdate {
                id: first.page.id,
                status: "active".to_owned(),
                expected_revision: trashed.revision,
            },
        )
        .unwrap();
        assert_eq!(list_from(&connection, "paper").unwrap().len(), 2);
    }
}
