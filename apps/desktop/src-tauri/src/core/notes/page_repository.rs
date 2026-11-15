use std::collections::HashSet;

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use uuid::Uuid;

use super::{
    error::NotesError,
    model::{NoteListQuery, NotePage, NotePageMetadataUpdate, validate_status, validate_title},
};

fn string_list(connection: &Connection, sql: &str, id: &str) -> Result<Vec<String>, NotesError> {
    let mut statement = connection.prepare(sql)?;
    let rows = statement.query_map([id], |row| row.get(0))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub(crate) fn page_from(connection: &Connection, id: &str) -> Result<NotePage, NotesError> {
    let mut page = connection
        .query_row(
            "select id, title, parent_page_id, position, status, is_favorite, revision,
                    schema_version, created_at, updated_at, trashed_at,
                    (select target_id from note_links where source_page_id = note_pages.id
                     and source_block_id is null and target_kind = 'paper'
                     and relation_type = 'derived_from' order by created_at, id limit 1)
             from note_pages where id = ?1",
            [id],
            |row| {
                Ok(NotePage {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    literature_paper_id: row.get(11)?,
                    parent_page_id: row.get(2)?,
                    position: row.get(3)?,
                    status: row.get(4)?,
                    is_favorite: row.get::<_, i64>(5)? != 0,
                    revision: row.get(6)?,
                    schema_version: row.get(7)?,
                    project_ids: Vec::new(),
                    tag_ids: Vec::new(),
                    created_at: row.get(8)?,
                    updated_at: row.get(9)?,
                    trashed_at: row.get(10)?,
                })
            },
        )
        .optional()?
        .ok_or(NotesError::NotFound)?;
    page.project_ids = string_list(
        connection,
        "select project_id from project_note_pages where page_id = ?1 order by project_id",
        id,
    )?;
    page.tag_ids = string_list(
        connection,
        "select tag_id from note_page_tags where page_id = ?1 order by tag_id",
        id,
    )?;
    Ok(page)
}

pub(crate) fn list_pages_from(
    connection: &Connection,
    query: Option<&NoteListQuery>,
) -> Result<Vec<NotePage>, NotesError> {
    let status = query
        .and_then(|value| value.status.as_deref())
        .unwrap_or("active");
    if !validate_status(status) {
        return Err(NotesError::InvalidInput("笔记状态无效".to_owned()));
    }
    let parent = query.and_then(|value| value.parent_page_id.as_deref());
    let project = query.and_then(|value| value.project_id.as_deref());
    let tag = query.and_then(|value| value.tag_id.as_deref());
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
        "select p.id from note_pages p
         where p.status = ?1
           and (?8 = 'all' or not exists(select 1 from reading_note_sources s where s.page_id=p.id))
           and (?2 is null or p.parent_page_id = ?2)
           and (?3 is null or exists(select 1 from project_note_pages x where x.page_id = p.id and x.project_id = ?3))
           and (?4 is null or exists(select 1 from note_page_tags x where x.page_id = p.id and x.tag_id = ?4))
           and (?5 = '' or p.title like '%' || ?5 || '%' or exists(
                select 1 from note_blocks b where b.page_id = p.id and b.plain_text like '%' || ?5 || '%'))
           and (?6 = '' or p.id > ?6)
         order by case when ?8 = 'all' then p.id else coalesce(p.parent_page_id, '') end,
           case when ?8 = 'all' then 0 else p.position end, p.id limit ?7",
    )?;
    let ids = statement
        .query_map(
            params![
                status,
                parent,
                project,
                tag,
                search,
                cursor,
                limit,
                query.and_then(|q| q.kind.as_deref()).unwrap_or("ordinary")
            ],
            |row| row.get::<_, String>(0),
        )?
        .collect::<Result<Vec<_>, _>>()?;
    ids.iter().map(|id| page_from(connection, id)).collect()
}

pub(super) fn next_position(
    transaction: &Transaction<'_>,
    parent_page_id: Option<&str>,
) -> Result<i64, NotesError> {
    Ok(transaction.query_row(
        "select coalesce(max(position), -1) + 1 from note_pages
         where (parent_page_id = ?1 or (parent_page_id is null and ?1 is null))",
        [parent_page_id],
        |row| row.get(0),
    )?)
}

fn normalize_parent_blocks(
    transaction: &Transaction<'_>,
    parent_id: &str,
) -> Result<(), NotesError> {
    let roots = {
        let mut statement = transaction.prepare(
            "select id, block_type from note_blocks
             where page_id = ?1 and parent_block_id is null and status = 'active'
             order by position, id",
        )?;
        statement
            .query_map([parent_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?
    };
    let children = {
        let mut statement = transaction.prepare(
            "select b.id from note_blocks b
             join note_pages p on p.id = json_extract(b.content_json, '$.targetId')
             where b.page_id = ?1 and b.parent_block_id is null
               and b.block_type = 'childPage' and b.status = 'active'
               and p.parent_page_id = ?1 and p.status = 'active'
             order by p.position, p.id",
        )?;
        statement
            .query_map([parent_id], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?
    };
    let mut ordered = Vec::with_capacity(roots.len());
    let mut children = children.into_iter();
    for (id, block_type) in roots {
        if block_type == "childPage" {
            if let Some(child) = children.next() {
                ordered.push(child);
            }
        } else {
            ordered.push(id);
        }
    }
    ordered.extend(children);
    for (position, id) in ordered.iter().enumerate() {
        transaction.execute(
            "update note_blocks set position = ?2, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') where id = ?1",
            params![id, position as i64],
        )?;
    }
    Ok(())
}

fn touch_parent(transaction: &Transaction<'_>, parent_id: &str) -> Result<(), NotesError> {
    transaction.execute(
        "update note_pages set revision = revision + 1,
         updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') where id = ?1",
        [parent_id],
    )?;
    Ok(())
}

pub(super) fn position_child_page_block(
    transaction: &Transaction<'_>,
    parent_id: &str,
    child_id: &str,
    requested_position: i64,
) -> Result<(), NotesError> {
    if requested_position < 0 {
        return Err(NotesError::InvalidInput("子页面块位置无效".to_owned()));
    }
    let child_block_id: String = transaction.query_row(
        "select id from note_blocks where page_id = ?1 and block_type = 'childPage'
         and status = 'active' and json_extract(content_json, '$.targetId') = ?2",
        params![parent_id, child_id],
        |row| row.get(0),
    )?;
    let mut block_ids = {
        let mut statement = transaction.prepare(
            "select id from note_blocks where page_id = ?1 and parent_block_id is null
             and status = 'active' order by position, id",
        )?;
        statement
            .query_map([parent_id], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?
    };
    block_ids.retain(|id| id != &child_block_id);
    let insertion = usize::try_from(requested_position)
        .unwrap_or(usize::MAX)
        .min(block_ids.len());
    block_ids.insert(insertion, child_block_id);
    for (position, block_id) in block_ids.iter().enumerate() {
        transaction.execute(
            "update note_blocks set position = ?2,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') where id = ?1",
            params![block_id, position as i64],
        )?;
    }
    let child_ids = {
        let mut statement = transaction.prepare(
            "select json_extract(content_json, '$.targetId') from note_blocks
             where page_id = ?1 and parent_block_id is null and block_type = 'childPage'
               and status = 'active' order by position, id",
        )?;
        statement
            .query_map([parent_id], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?
    };
    for (position, id) in child_ids.iter().enumerate() {
        transaction.execute(
            "update note_pages set position = ?2,
             revision = revision + case when id <> ?3 and position <> ?2 then 1 else 0 end,
             updated_at = case when id <> ?3 and position <> ?2 then strftime('%Y-%m-%dT%H:%M:%fZ', 'now') else updated_at end
             where id = ?1",
            params![id, position as i64, child_id],
        )?;
    }
    Ok(())
}

pub(crate) fn sync_child_page_block(
    transaction: &Transaction<'_>,
    child_id: &str,
) -> Result<HashSet<String>, NotesError> {
    let (parent_id, title, status): (Option<String>, String, String) = transaction
        .query_row(
            "select parent_page_id, title, status from note_pages where id = ?1",
            [child_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?
        .ok_or(NotesError::NotFound)?;
    let existing: Option<(String, String)> = transaction
        .query_row(
            "select id, page_id from note_blocks
             where block_type = 'childPage' and json_extract(content_json, '$.targetId') = ?1
             order by status = 'active' desc limit 1",
            [child_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let mut affected = HashSet::new();
    if let Some((_, old_parent)) = &existing {
        affected.insert(old_parent.clone());
    }
    if let Some(parent) = &parent_id {
        affected.insert(parent.clone());
        let content = serde_json::json!({
            "kind": "reference",
            "targetKind": "note_page",
            "targetId": child_id,
            "labelSnapshot": title,
            "quoteSnapshot": "",
        })
        .to_string();
        if let Some((block_id, _)) = existing {
            transaction.execute(
                "update note_blocks set
                 position = case when page_id <> ?2 then coalesce((select max(position) + 1 from note_blocks where page_id = ?2 and parent_block_id is null), 0) else position end,
                 page_id = ?2, parent_block_id = null,
                 block_type = 'childPage', content_json = ?3, plain_text = ?4,
                 status = ?5, trashed_at = case when ?5 = 'trashed'
                   then coalesce(trashed_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')) else null end,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') where id = ?1",
                params![block_id, parent, content, title, status],
            )?;
        } else {
            transaction.execute(
                "insert into note_blocks
                 (id, scope_kind, page_id, parent_block_id, position, block_type, props_json,
                  content_json, plain_text, schema_version, revision, status, is_favorite,
                  created_at, updated_at, trashed_at)
                 values (?1, 'page', ?2, null,
                   coalesce((select max(position) + 1 from note_blocks where page_id = ?2 and parent_block_id is null), 0),
                   'childPage', '{}', ?3, ?4, 1, 0, ?5, 0,
                   strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                   case when ?5 = 'trashed' then strftime('%Y-%m-%dT%H:%M:%fZ', 'now') else null end)",
                params![Uuid::new_v4().to_string(), parent, content, title, status],
            )?;
        }
    } else if let Some((block_id, _)) = existing {
        transaction.execute(
            "update note_blocks set status = 'trashed',
             trashed_at = coalesce(trashed_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') where id = ?1",
            [block_id],
        )?;
    }
    for parent in &affected {
        normalize_parent_blocks(transaction, parent)?;
    }
    Ok(affected)
}

pub(super) fn touch_parents(
    transaction: &Transaction<'_>,
    parents: HashSet<String>,
) -> Result<(), NotesError> {
    for parent in parents {
        touch_parent(transaction, &parent)?;
    }
    Ok(())
}

pub(crate) use super::page_creation::{create_page_from, create_page_with_paper_from};

fn replace_relations(
    transaction: &Transaction<'_>,
    delete_sql: &str,
    insert_sql: &str,
    page_id: &str,
    ids: &[String],
) -> Result<(), NotesError> {
    transaction.execute(delete_sql, [page_id])?;
    for id in ids {
        transaction.execute(insert_sql, params![page_id, id])?;
    }
    Ok(())
}

pub(crate) fn update_page_from(
    connection: &mut Connection,
    request: NotePageMetadataUpdate,
) -> Result<NotePage, NotesError> {
    let transaction = connection.transaction()?;
    let page = update_page_in(&transaction, request)?;
    transaction.commit()?;
    Ok(page)
}

pub(super) fn update_page_in(
    transaction: &Transaction<'_>,
    request: NotePageMetadataUpdate,
) -> Result<NotePage, NotesError> {
    let before = super::text_change_set::read(transaction, &request.page_id)?;
    let current_revision: i64 = transaction
        .query_row(
            "select revision from note_pages where id = ?1",
            [&request.page_id],
            |row| row.get(0),
        )
        .optional()?
        .ok_or(NotesError::NotFound)?;
    if current_revision != request.expected_revision {
        return Err(NotesError::RevisionConflict { current_revision });
    }
    if request.is_favorite == Some(true) {
        let dependent: bool = transaction.query_row(
            "select exists(select 1 from reading_note_sources where page_id=?1
             and (note_type='highlight' or (note_type='annotation' and trim(quote_exact)<>'')))",
            [&request.page_id],
            |row| row.get(0),
        )?;
        if dependent {
            return Err(NotesError::InvalidInput("附属笔记不支持收藏".to_owned()));
        }
    }
    let previous_title: String = transaction.query_row(
        "select title from note_pages where id=?1",
        [&request.page_id],
        |row| row.get(0),
    )?;
    let title_changed = request
        .title
        .as_ref()
        .is_some_and(|title| validate_title(title).as_deref() != Some(previous_title.as_str()));
    if let Some(title) = request.title {
        let title = validate_title(&title)
            .ok_or_else(|| NotesError::InvalidInput("笔记标题过长".to_owned()))?;
        transaction.execute(
            "update note_pages set title = ?2, document_title = ?2 where id = ?1",
            params![request.page_id, title],
        )?;
        let parents = sync_child_page_block(transaction, &request.page_id)?;
        touch_parents(transaction, parents)?;
    }
    if let Some(value) = request.is_favorite {
        transaction.execute(
            "update note_pages set is_favorite = ?2 where id = ?1",
            params![request.page_id, value],
        )?;
    }
    if let Some(ids) = request.project_ids {
        replace_relations(
            transaction,
            "delete from project_note_pages where page_id = ?1",
            "insert into project_note_pages(page_id, project_id, added_at) values (?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
            &request.page_id,
            &ids,
        )?;
    }
    if let Some(ids) = request.tag_ids {
        replace_relations(
            transaction,
            "delete from note_page_tags where page_id = ?1",
            "insert into note_page_tags(page_id, tag_id) values (?1, ?2)",
            &request.page_id,
            &ids,
        )?;
    }
    transaction.execute("update note_pages set revision = revision + 1, updated_at = case when ?2 then strftime('%Y-%m-%dT%H:%M:%fZ', 'now') else updated_at end where id = ?1", params![request.page_id, title_changed])?;
    let after = super::text_change_set::read(transaction, &request.page_id)?;
    crate::agent::text_tools::note_review::synchronize_metadata(transaction, &before, &after)
        .map_err(NotesError::InvalidInput)?;
    if title_changed {
        super::hbs::synchronize_title(transaction, &request.page_id, &after.title)
            .map_err(NotesError::InvalidInput)?;
    }
    transaction.execute(
        "update note_index_state set state = 'dirty' where id = 1",
        [],
    )?;
    page_from(transaction, &request.page_id)
}

pub(crate) use super::page_mutation::{move_page_from, set_page_status_from};
