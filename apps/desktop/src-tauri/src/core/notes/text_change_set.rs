//! Atomic application of validated page, document and lifecycle changes.
use super::{
    NoteDocument, NoteDocumentSaveRequest, document_repository::save_preserving_in,
    error::NotesError, model::NotePageDraft, page_creation::create_page_with_paper_in,
};
use rusqlite::{Transaction, params};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", from = "PageStateWire")]
pub(crate) struct PageState {
    pub page_id: String,
    pub title: String,
    pub parent_id: Option<String>,
    pub position: i64,
    pub revision: i64,
    pub status: String,
    pub document: NoteDocument,
}
/// Old proposal snapshots stored title beside a body-only document. Decode
/// them once into the same canonical document as current native snapshots.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PageStateWire {
    page_id: String,
    title: String,
    parent_id: Option<String>,
    position: i64,
    revision: i64,
    status: String,
    document: NoteDocument,
}
impl From<PageStateWire> for PageState {
    fn from(mut value: PageStateWire) -> Self {
        if value.document.title.is_none() {
            value.document.title = Some(value.title.clone());
        }
        Self {
            page_id: value.page_id,
            title: value.title,
            parent_id: value.parent_id,
            position: value.position,
            revision: value.revision,
            status: value.status,
            document: value.document,
        }
    }
}
#[derive(Clone)]
pub(crate) struct PageChange {
    pub before: Option<PageState>,
    pub after: Option<PageState>,
}
pub(crate) fn read(db: &rusqlite::Connection, page: &str) -> Result<PageState, NotesError> {
    let (count,bytes,invalid):(usize,usize,usize)=db.query_row("select count(*),coalesce(sum(length(cast(props_json as blob))+length(cast(content_json as blob))),0),coalesce(sum(case when json_valid(props_json)=0 or json_valid(content_json)=0 or schema_version<>1 then 1 else 0 end),0) from note_blocks where page_id=?1 and scope_kind='page' and status='active'",[page],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?)))?;
    if count > 20_000 || bytes > 8 * 1024 * 1024 || invalid != 0 {
        return Err(NotesError::InvalidInput(
            "note snapshot is damaged or exceeds limits".into(),
        ));
    }
    let (title, parent_id, position, revision, status) = db.query_row(
        "select title,parent_page_id,position,revision,status from note_pages where id=?1",
        [page],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        },
    )?;
    let reachable:usize=db.query_row("with recursive tree(id,depth) as(select id,1 from note_blocks where page_id=?1 and scope_kind='page' and status='active' and parent_block_id is null union all select b.id,t.depth+1 from note_blocks b join tree t on b.parent_block_id=t.id where b.page_id=?1 and b.scope_kind='page' and b.status='active' and t.depth<64) select count(*) from tree",[page],|row|row.get(0))?;
    if reachable != count {
        return Err(NotesError::InvalidInput(
            "note snapshot has orphaned or cyclic blocks".into(),
        ));
    }
    let document = super::load_document_value(db, page)?;
    super::document_validation::validate_preserving(&document, &document)
        .map_err(NotesError::InvalidInput)?;
    Ok(PageState {
        page_id: page.into(),
        title,
        parent_id,
        position,
        revision,
        status,
        document,
    })
}
fn revision(tx: &Transaction<'_>, page: &str) -> Result<i64, NotesError> {
    Ok(tx.query_row(
        "select revision from note_pages where id=?1",
        [page],
        |row| row.get(0),
    )?)
}
pub(crate) fn apply(
    tx: &Transaction<'_>,
    changes: &[PageChange],
) -> Result<Vec<PageState>, NotesError> {
    let mapping = NoteDocument {
        title: None,
        schema_version: 1,
        page_id: "group".into(),
        blocks: changes
            .iter()
            .filter_map(|change| change.before.as_ref())
            .flat_map(|page| page.document.blocks.iter().cloned())
            .collect(),
    };
    for change in changes {
        if let Some(before) = &change.before {
            let current = read(tx, &before.page_id)?;
            if serde_json::to_value(current)
                .map_err(|error| NotesError::InvalidInput(error.to_string()))?
                != serde_json::to_value(before)
                    .map_err(|error| NotesError::InvalidInput(error.to_string()))?
            {
                return Err(NotesError::RevisionConflict {
                    current_revision: revision(tx, &before.page_id)?,
                });
            }
            let revision_id = uuid::Uuid::new_v4().to_string();
            tx.execute("insert or ignore into note_revisions(id,page_id,page_revision,schema_version,document_json,reason,created_at) values(?1,?2,?3,1,?4,'structure',strftime('%Y-%m-%dT%H:%M:%fZ','now'))", params![revision_id, before.page_id, before.revision, serde_json::to_string(&before.document).map_err(|error| NotesError::InvalidInput(error.to_string()))?])?;
            let revision_id: String = tx.query_row(
                "select id from note_revisions where page_id=?1 and page_revision=?2",
                params![before.page_id, before.revision],
                |row| row.get(0),
            )?;
            super::revision_assets::replace_revision_assets(tx, &revision_id, &before.document)?;
        } else if let Some(after) = &change.after
            && tx.query_row(
                "select exists(select 1 from note_pages where id=?1)",
                [&after.page_id],
                |row| row.get::<_, bool>(0),
            )?
        {
            return Err(NotesError::InvalidInput("新建页面身份已存在".into()));
        }
    }
    let mut new = changes
        .iter()
        .filter(|change| change.before.is_none())
        .filter_map(|change| change.after.as_ref())
        .collect::<Vec<_>>();
    while !new.is_empty() {
        let index = new
            .iter()
            .position(|page| {
                page.parent_id
                    .as_ref()
                    .is_none_or(|parent| !new.iter().any(|page| &page.page_id == parent))
            })
            .ok_or_else(|| NotesError::InvalidInput("页面树不能形成环路".into()))?;
        let page = new.remove(index);
        create_page_with_paper_in(
            tx,
            NotePageDraft {
                title: page.title.clone(),
                parent_page_id: page.parent_id.clone(),
                parent_block_position: None,
            },
            None,
            Some(&page.page_id),
        )?;
    }
    // Metadata changes precede parent-document saves, so child targets remain valid.
    fn owners(
        blocks: &[super::NoteDocumentBlock],
        page: &str,
        ids: &mut std::collections::BTreeMap<String, String>,
    ) {
        for block in blocks {
            ids.insert(block.id.clone(), page.into());
            owners(&block.children, page, ids);
        }
    }
    let mut before_owners = std::collections::BTreeMap::new();
    let mut after_owners = std::collections::BTreeMap::new();
    for change in changes {
        if let Some(page) = &change.before {
            owners(&page.document.blocks, &page.page_id, &mut before_owners);
        }
        if let Some(page) = &change.after {
            owners(&page.document.blocks, &page.page_id, &mut after_owners);
        }
    }
    for (block, page) in &after_owners {
        if let Some(original) = before_owners.get(block)
            && original != page
        {
            tx.execute("update note_links set source_page_id=?1 where source_page_id=?2 and coalesce(source_block_id,'')=?3",params![page,original,block])?;
        }
    }
    for change in changes {
        let Some(after) = &change.after else {
            continue;
        };
        if let Some(before) = &change.before {
            if before.parent_id != after.parent_id || before.position != after.position {
                tx.execute("update note_pages set parent_page_id=?2,position=?3,revision=revision+1,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') where id=?1",params![after.page_id,after.parent_id,after.position])?;
            }
            if before.status != after.status {
                status(tx, &after.page_id, &after.status)?;
            }
            if before.title != after.title {
                tx.execute("update note_pages set title=?2,document_title=?2,revision=revision+1,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') where id=?1",params![after.page_id,after.title])?;
            }
        }
    }
    for change in changes {
        if change.after.is_none()
            && let Some(before) = &change.before
            && read(tx, &before.page_id)?.status != "trashed"
        {
            status(tx, &before.page_id, "trashed")?;
        }
    }
    for change in changes {
        if let Some(after) = &change.after {
            let mut document = after.document.clone();
            // The text-tool envelope still carries a display title. Preserve
            // raw text for body-only edits; an explicit rename updates both.
            let renamed = change
                .before
                .as_ref()
                .is_none_or(|before| before.title != after.title);
            if renamed
                || document.title.as_ref().is_some_and(|title| {
                    super::model::validate_title(title).as_deref() != Some(after.title.as_str())
                })
            {
                document.title = Some(after.title.clone());
            }
            save_preserving_in(
                tx,
                NoteDocumentSaveRequest {
                    document,
                    expected_revision: revision(tx, &after.page_id)?,
                    save_reason: "structure".into(),
                },
                &mapping,
            )?;
        }
    }
    changes
        .iter()
        .filter_map(|change| change.after.as_ref().or(change.before.as_ref()))
        .map(|page| read(tx, &page.page_id))
        .collect()
}
fn status(tx: &Transaction<'_>, page: &str, status: &str) -> Result<(), NotesError> {
    if !super::model::validate_status(status) {
        return Err(NotesError::InvalidInput("Invalid note status".into()));
    }
    tx.execute("update note_pages set status=?2,trashed_at=case when ?2='trashed' then strftime('%Y-%m-%dT%H:%M:%fZ','now') else null end,revision=revision+1,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') where id=?1",params![page,status])?;
    tx.execute("update note_blocks set status=?2,trashed_at=case when ?2='trashed' then strftime('%Y-%m-%dT%H:%M:%fZ','now') else null end where block_type='childPage' and json_extract(content_json,'$.targetId')=?1",params![page,status])?;
    tx.execute("update note_index_state set state='dirty' where id=1", [])?;
    Ok(())
}
