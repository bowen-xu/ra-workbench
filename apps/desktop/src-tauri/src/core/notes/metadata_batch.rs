use super::{
    error::{NotesCommandError, NotesError},
    favorites,
    model::NotePage,
    page_repository::{page_from, sync_child_page_block, touch_parents},
};
use crate::storage::AppStore;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use tauri::State;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BatchItem {
    pub id: String,
    pub expected_revision: i64,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BatchRequest {
    pub operation_id: String,
    pub action: String,
    pub items: Vec<BatchItem>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchFailure {
    id: String,
    code: String,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    current_revision: Option<i64>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchReceipt {
    pub(super) operation_id: String,
    pub(super) succeeded_ids: Vec<String>,
    pub(super) failures: Vec<BatchFailure>,
    pub(super) pages: Vec<NotePage>,
    pub(super) body_changed_ids: Vec<String>,
    pub(super) favorite_page_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) history: Option<super::metadata_history::History>,
}
struct Metadata {
    parent: Option<String>,
    revision: i64,
    status: String,
    favorite: bool,
    dependent: bool,
}
fn root(parents: &mut [usize], mut index: usize) -> usize {
    while parents[index] != index {
        parents[index] = parents[parents[index]];
        index = parents[index];
    }
    index
}
fn failure(id: &str, code: &str, message: &str, revision: Option<i64>) -> BatchFailure {
    BatchFailure {
        id: id.into(),
        code: code.into(),
        message: message.into(),
        current_revision: revision,
    }
}

pub(crate) fn apply_from(
    connection: &mut Connection,
    request: BatchRequest,
) -> Result<BatchReceipt, NotesError> {
    let tx = connection.transaction()?;
    let receipt = apply_in(&tx, request, true)?;
    tx.commit()?;
    Ok(receipt)
}
pub(super) fn apply_in(
    tx: &rusqlite::Transaction<'_>,
    request: BatchRequest,
    cascade_ownership: bool,
) -> Result<BatchReceipt, NotesError> {
    if request.operation_id.is_empty()
        || request.operation_id.len() > 128
        || request.items.len() > 10000
        || !matches!(
            request.action.as_str(),
            "trash" | "restore" | "archive" | "favorite" | "unfavorite"
        )
        || request
            .items
            .iter()
            .any(|item| item.id.is_empty() || item.expected_revision < 0)
        || request
            .items
            .iter()
            .map(|item| &item.id)
            .collect::<HashSet<_>>()
            .len()
            != request.items.len()
    {
        return Err(NotesError::InvalidInput("批量笔记请求无效".into()));
    }
    let encoded =
        serde_json::to_string(&request).map_err(|e| NotesError::InvalidInput(e.to_string()))?;
    let prior: Option<(String, String)> = tx.query_row(
        "select request_json,receipt_json from note_metadata_batch_receipts where operation_id=?1",
        [&request.operation_id], |row| Ok((row.get(0)?, row.get(1)?))).optional()?;
    if let Some((previous, receipt)) = prior {
        if previous != encoded {
            return Err(NotesError::InvalidInput("操作 ID 已用于不同请求".into()));
        }
        return serde_json::from_str(&receipt).map_err(|e| NotesError::InvalidInput(e.to_string()));
    }
    let mut statement = tx.prepare("select p.id,p.parent_page_id,p.revision,p.status,p.is_favorite,
      exists(select 1 from reading_note_sources s where s.page_id=p.id and
        (s.note_type='highlight' or (s.note_type='annotation' and trim(s.quote_exact)<>''))) from note_pages p")?;
    let metadata: HashMap<String, Metadata> = statement
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                Metadata {
                    parent: row.get(1)?,
                    revision: row.get(2)?,
                    status: row.get(3)?,
                    favorite: row.get(4)?,
                    dependent: row.get(5)?,
                },
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    drop(statement);
    let mut children: HashMap<&str, Vec<&str>> = HashMap::new();
    for (id, page) in &metadata {
        if let Some(parent) = &page.parent {
            children.entry(parent).or_default().push(id);
        }
    }
    let cascade = cascade_ownership && matches!(request.action.as_str(), "trash" | "restore");
    let favorite = matches!(request.action.as_str(), "favorite" | "unfavorite");
    let mut parents: Vec<usize> = (0..request.items.len()).collect();
    let mut owner: HashMap<String, usize> = HashMap::new();
    let mut impacts = Vec::new();
    for (index, item) in request.items.iter().enumerate() {
        let mut ids = BTreeSet::new();
        let mut pending = vec![item.id.clone()];
        while let Some(id) = pending.pop() {
            if !ids.insert(id.clone()) {
                continue;
            }
            if cascade {
                pending.extend(
                    children
                        .get(id.as_str())
                        .into_iter()
                        .flatten()
                        .map(|id| id.to_string()),
                );
            }
        }
        for id in &ids {
            if let Some(previous) = owner.insert(id.clone(), index) {
                let from = root(&mut parents, index);
                let to = root(&mut parents, previous);
                parents[from] = to;
            }
        }
        impacts.push(ids);
    }
    let mut failures = BTreeMap::new();
    let mut blocked = HashSet::new();
    for (index, item) in request.items.iter().enumerate() {
        let error = match metadata.get(&item.id) {
            None => Some(failure(&item.id, "not_found", "找不到笔记", None)),
            Some(page) if page.revision != item.expected_revision => Some(failure(
                &item.id,
                "revision_conflict",
                "笔记已更新",
                Some(page.revision),
            )),
            Some(page) if favorite && (page.dependent || page.status != "active") => Some(failure(
                &item.id,
                "invalid_input",
                "只能调整活动独立笔记的收藏",
                None,
            )),
            Some(page) if request.action != "restore" && page.status == "trashed" => Some(failure(
                &item.id,
                "invalid_input",
                "请先恢复回收站中的笔记",
                None,
            )),
            _ => None,
        };
        if let Some(error) = error {
            failures.insert(item.id.clone(), error);
            blocked.insert(root(&mut parents, index));
        }
    }
    let mut succeeded_ids = Vec::new();
    let mut affected = BTreeSet::new();
    for (index, item) in request.items.iter().enumerate() {
        if blocked.contains(&root(&mut parents, index)) {
            failures.entry(item.id.clone()).or_insert_with(|| {
                failure(
                    &item.id,
                    "revision_conflict",
                    "同一归属组有未完成项，请刷新后重试",
                    None,
                )
            });
        } else {
            succeeded_ids.push(item.id.clone());
            affected.extend(impacts[index].clone());
        }
    }
    let status = match request.action.as_str() {
        "trash" => "trashed",
        "archive" => "archived",
        _ => "active",
    };
    let mut changed = BTreeSet::new();
    let mut update_status = tx.prepare("update note_pages set status=?2, trashed_at=case when ?2='trashed' then strftime('%Y-%m-%dT%H:%M:%fZ','now') else null end,
        revision=revision+1,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') where id=?1")?;
    let mut update_favorite =
        tx.prepare("update note_pages set is_favorite=?2,revision=revision+1 where id=?1")?;
    let updates: Vec<String> = if favorite {
        request
            .items
            .iter()
            .filter(|item| affected.contains(&item.id))
            .map(|item| item.id.clone())
            .collect()
    } else {
        affected.into_iter().collect()
    };
    for id in updates {
        let page = &metadata[&id];
        if favorite {
            let value = request.action == "favorite";
            if page.favorite == value {
                continue;
            }
            update_favorite.execute(params![id, value])?;
        } else {
            if page.status == status {
                continue;
            }
            update_status.execute(params![id, status])?;
        }
        changed.insert(id);
    }
    drop(update_status);
    drop(update_favorite);
    let targets: HashSet<String> = changed.iter().cloned().collect();
    let trashed = if request.action == "trash" {
        targets.clone()
    } else {
        HashSet::new()
    };
    let history = if trashed.is_empty() {
        None
    } else {
        Some(super::metadata_history::capture(
            tx,
            &trashed,
            targets
                .iter()
                .map(|id| (id.clone(), metadata[id].status.clone()))
                .collect(),
        )?)
    };
    let mut body_changed = HashSet::new();
    if !favorite {
        for id in &changed {
            if metadata[id].parent.is_some() {
                body_changed.extend(sync_child_page_block(tx, id)?);
            }
        }
    }
    let detached = super::page_entries::detach(tx, &trashed)?;
    body_changed.extend(detached.iter().cloned());
    touch_parents(
        tx,
        body_changed
            .iter()
            .filter(|id| !changed.contains(*id) && !detached.contains(*id))
            .cloned()
            .collect(),
    )?;
    super::page_entries::synchronize_baselines(tx, &body_changed, &targets, &trashed)?;
    let mut page_ids = changed;
    page_ids.extend(body_changed.iter().cloned());
    let receipt = BatchReceipt {
        operation_id: request.operation_id.clone(),
        succeeded_ids,
        failures: failures.into_values().collect(),
        pages: page_ids
            .iter()
            .map(|id| page_from(tx, id))
            .collect::<Result<_, _>>()?,
        body_changed_ids: {
            let mut ids: Vec<_> = body_changed.into_iter().collect();
            ids.sort();
            ids
        },
        favorite_page_ids: favorites::list_from(tx)?,
        history,
    };
    let receipt_json =
        serde_json::to_string(&receipt).map_err(|e| NotesError::InvalidInput(e.to_string()))?;
    tx.execute("insert into note_metadata_batch_receipts(operation_id,request_json,receipt_json,created_at)
      values(?1,?2,?3,strftime('%Y-%m-%dT%H:%M:%fZ','now'))", params![request.operation_id, encoded, receipt_json])?;
    Ok(receipt)
}

#[tauri::command]
pub fn apply_note_metadata_batch(
    store: State<'_, AppStore>,
    request: BatchRequest,
) -> Result<BatchReceipt, NotesCommandError> {
    let mut connection = store.connection().map_err(NotesError::from)?;
    apply_from(&mut connection, request).map_err(Into::into)
}
#[tauri::command]
pub fn list_note_page_metadata(
    store: State<'_, AppStore>,
    ids: Vec<String>,
) -> Result<Vec<NotePage>, NotesCommandError> {
    if ids.len() > 10000 {
        return Err(NotesError::InvalidInput("元数据请求过大".into()).into());
    }
    let connection = store.connection().map_err(NotesError::from)?;
    ids.into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter_map(|id| match page_from(&connection, &id) {
            Err(NotesError::NotFound) => None,
            value => Some(value),
        })
        .collect::<Result<_, _>>()
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn seed(db: &Connection, id: &str, parent: Option<&str>) {
        db.execute("insert into note_pages(id,title,parent_page_id,created_at,updated_at) values(?1,?1,?2,'old','old')", params![id, parent]).unwrap();
    }
    fn request(operation: &str, ids: &[(&str, i64)]) -> BatchRequest {
        BatchRequest {
            operation_id: operation.into(),
            action: "trash".into(),
            items: ids
                .iter()
                .map(|(id, revision)| BatchItem {
                    id: (*id).into(),
                    expected_revision: *revision,
                })
                .collect(),
        }
    }
    #[test]
    fn batch_updates_500_metadata_rows_and_replays_receipt_without_reading_bodies() {
        let mut db = super::super::tests::migrated_connection();
        let tx = db.transaction().unwrap();
        for i in 0..500 {
            seed(&tx, &format!("page-{i}"), None);
        }
        tx.execute_batch("insert into note_blocks(id,scope_kind,page_id,position,block_type,props_json,content_json,plain_text,schema_version,revision,status,is_favorite,created_at,updated_at)
          values('sentinel','page','page-0',0,'paragraph','{}','{\"sentinel\":\"intentionally not a document body\"}','Retained body',1,0,'active',0,'old','old')").unwrap();
        tx.execute_batch("insert into agent_note_patch_candidates(conversation_id,page_id,proposal_id,generation,baseline_json,candidate_json,state) values('conversation','page-0','pending',1,'{}','{\"retained\":true}','pending')").unwrap();
        tx.commit().unwrap();
        let ids: Vec<_> = (0..500).map(|i| format!("page-{i}")).collect();
        let pairs: Vec<_> = ids.iter().map(|id| (id.as_str(), 0)).collect();
        let receipt = apply_from(&mut db, request("first", &pairs)).unwrap();
        assert_eq!(receipt.succeeded_ids.len(), 500);
        assert_eq!(receipt.pages.len(), 500);
        assert!(receipt.body_changed_ids.is_empty());
        let replay = apply_from(&mut db, request("first", &pairs)).unwrap();
        assert_eq!(
            serde_json::to_value(receipt).unwrap(),
            serde_json::to_value(replay).unwrap()
        );
        assert_eq!(
            db.query_row(
                "select revision from note_pages where id='page-0'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        assert_eq!(
            db.query_row(
                "select plain_text from note_blocks where id='sentinel'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "Retained body"
        );
        assert_eq!(db.query_row("select candidate_json,state from agent_note_patch_candidates where proposal_id='pending'", [], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).unwrap(), ("{\"retained\":true}".into(), "pending".into()));
        let mut different = request("first", &pairs);
        different.action = "archive".into();
        assert!(matches!(
            apply_from(&mut db, different),
            Err(NotesError::InvalidInput(_))
        ));
    }
    #[test]
    fn overlapping_ownership_conflicts_cannot_be_bypassed_through_ancestor() {
        let mut db = super::super::tests::migrated_connection();
        seed(&db, "parent", None);
        seed(&db, "child", Some("parent"));
        seed(&db, "independent", None);
        let receipt = apply_from(
            &mut db,
            request("group", &[("parent", 0), ("child", 99), ("independent", 0)]),
        )
        .unwrap();
        assert_eq!(receipt.succeeded_ids, ["independent"]);
        assert_eq!(receipt.failures.len(), 2);
        assert_eq!(page_from(&db, "parent").unwrap().status, "active");
        assert_eq!(page_from(&db, "child").unwrap().status, "active");
    }
    #[test]
    fn database_failure_rolls_back_the_entire_valid_set_and_receipt() {
        let mut db = super::super::tests::migrated_connection();
        seed(&db, "a", None);
        seed(&db, "b", None);
        db.execute_batch("create trigger fixture_failure before update on note_pages when new.id='b' begin select raise(abort,'fixture disk failure'); end;").unwrap();
        assert!(apply_from(&mut db, request("failure", &[("a", 0), ("b", 0)])).is_err());
        assert_eq!(page_from(&db, "a").unwrap().status, "active");
        assert_eq!(
            db.query_row(
                "select count(*) from note_metadata_batch_receipts",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
    }
    #[test]
    fn favorites_preserve_edit_time_and_lifecycle_uses_the_same_batch() {
        let mut db = super::super::tests::migrated_connection();
        seed(&db, "a", None);
        seed(&db, "b", None);
        let mut favorite = request("favorite", &[("a", 0), ("b", 0)]);
        favorite.action = "favorite".into();
        assert_eq!(
            apply_from(&mut db, favorite).unwrap().favorite_page_ids,
            ["b", "a"]
        );
        assert_eq!(page_from(&db, "a").unwrap().updated_at, "old");
        let mut archive = request("archive", &[("a", 1)]);
        archive.action = "archive".into();
        apply_from(&mut db, archive).unwrap();
        assert_eq!(page_from(&db, "a").unwrap().status, "archived");
        let mut restore = request("restore", &[("a", 2)]);
        restore.action = "restore".into();
        apply_from(&mut db, restore).unwrap();
        let mut unfavorite = request("unfavorite", &[("b", 1)]);
        unfavorite.action = "unfavorite".into();
        let receipt = apply_from(&mut db, unfavorite).unwrap();
        assert_eq!(receipt.favorite_page_ids, ["a"]);
        assert_eq!(page_from(&db, "a").unwrap().status, "active");
        assert!(!page_from(&db, "b").unwrap().is_favorite);
    }
    #[test]
    fn overlapping_legacy_descendants_advance_once_and_restore_the_owner_block() {
        let mut db = super::super::tests::migrated_connection();
        seed(&db, "parent", None);
        seed(&db, "child", Some("parent"));
        seed(&db, "linked-independent", None);
        let receipt = apply_from(
            &mut db,
            request("trash-tree", &[("parent", 0), ("child", 0)]),
        )
        .unwrap();
        assert_eq!(receipt.body_changed_ids, ["parent"]);
        assert_eq!(page_from(&db, "parent").unwrap().revision, 1);
        assert_eq!(page_from(&db, "child").unwrap().revision, 1);
        assert_eq!(
            page_from(&db, "linked-independent").unwrap().status,
            "active"
        );
        let mut restore = request("restore-tree", &[("parent", 1), ("child", 1)]);
        restore.action = "restore".into();
        apply_from(&mut db, restore).unwrap();
        assert_eq!(page_from(&db, "parent").unwrap().revision, 2);
        assert_eq!(page_from(&db, "child").unwrap().revision, 2);
        assert_eq!(db.query_row("select count(*) from note_blocks where page_id='parent' and block_type='childPage' and status='active'", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    }
    #[test]
    fn upgrades_deployed_191_and_reopens_with_metadata_receipt_and_body_preserved() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let mut db = Connection::open(file.path()).unwrap();
        crate::storage_migration_tests::migrate_through(&mut db, 191);
        crate::storage_integrity::backfill_migration_checksums(&db).unwrap();
        seed(&db, "retained", None);
        db.execute_batch("insert into note_blocks(id,scope_kind,page_id,position,block_type,props_json,content_json,plain_text,schema_version,revision,status,is_favorite,created_at,updated_at)
          values('retained-body','page','retained',0,'paragraph','{}','[]','Upgrade body',1,0,'active',0,'old','old')").unwrap();
        crate::storage::migrate(&mut db).unwrap();
        let first = apply_from(&mut db, request("reopen", &[("retained", 0)])).unwrap();
        drop(db);
        let mut reopened = Connection::open(file.path()).unwrap();
        crate::storage::migrate(&mut reopened).unwrap();
        let second = apply_from(&mut reopened, request("reopen", &[("retained", 0)])).unwrap();
        assert_eq!(
            serde_json::to_value(first).unwrap(),
            serde_json::to_value(second).unwrap()
        );
        assert_eq!(page_from(&reopened, "retained").unwrap().revision, 1);
        assert_eq!(
            reopened
                .query_row(
                    "select plain_text from note_blocks where id='retained-body'",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            "Upgrade body"
        );
    }
}
