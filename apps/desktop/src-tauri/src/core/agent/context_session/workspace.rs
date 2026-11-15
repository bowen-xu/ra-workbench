//! Managed editable directories are configuration on the existing conversation identity.
use crate::{
    agent::{
        catalog_commands::CatalogCommandError, repository::SqliteConversationCatalog,
        service::AgentSessionService,
    },
    storage::AppStore,
};
use research_agent_core::conversation::{ConversationCatalog, ConversationId};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use std::{path::Path, sync::Arc};
use tauri::State;
use uuid::Uuid;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Directory {
    pub manuscript_id: String,
    pub title: String,
    pub path: String,
}
fn directory(id: String, title: String, root: &Path) -> Directory {
    let path = root.join(&id).join("source").to_string_lossy().into_owned();
    Directory {
        manuscript_id: id,
        title,
        path,
    }
}
fn read(db: &Connection, id: Uuid, root: &Path) -> Result<Option<Directory>, CatalogCommandError> {
    db.query_row(
        "select m.id,m.title from agent_conversation_bindings b join manuscripts m on m.id=b.resource_id
         where b.conversation_id=?1 and b.resource_kind='manuscript' order by b.resource_id limit 1",
        [id.to_string()], |r| Ok(directory(r.get(0)?, r.get(1)?, root)),
    ).optional().map_err(|_| CatalogCommandError::Unavailable)
}
#[tauri::command]
pub(crate) async fn agent_read_context_directory(
    store: State<'_, AppStore>,
    conversation_id: Uuid,
) -> Result<Option<Directory>, CatalogCommandError> {
    read_directory(&store, conversation_id).await
}

pub(crate) async fn read_directory(
    store: &AppStore,
    conversation_id: Uuid,
) -> Result<Option<Directory>, CatalogCommandError> {
    let store = store.clone();
    tokio::task::spawn_blocking(move || {
        let db = store
            .connection()
            .map_err(|_| CatalogCommandError::Unavailable)?;
        read(&db, conversation_id, store.manuscript_directory())
    })
    .await
    .map_err(|_| CatalogCommandError::Unavailable)?
}

#[tauri::command]
pub(crate) async fn agent_list_context_directories(
    store: State<'_, AppStore>,
) -> Result<Vec<Directory>, CatalogCommandError> {
    list_directories(&store).await
}

pub(crate) async fn list_directories(
    store: &AppStore,
) -> Result<Vec<Directory>, CatalogCommandError> {
    let store = store.clone();
    tokio::task::spawn_blocking(move || {
        let db = store
            .connection()
            .map_err(|_| CatalogCommandError::Unavailable)?;
        let mut stmt = db
            .prepare("select id,title from manuscripts where status='active' order by title,id")
            .map_err(|_| CatalogCommandError::Unavailable)?;
        stmt.query_map([], |r| {
            Ok(directory(
                r.get(0)?,
                r.get(1)?,
                store.manuscript_directory(),
            ))
        })
        .map_err(|_| CatalogCommandError::Unavailable)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| CatalogCommandError::Unavailable)
    })
    .await
    .map_err(|_| CatalogCommandError::Unavailable)?
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Change {
    pub conversation_id: Uuid,
    pub expected_directory_id: Option<Uuid>,
    pub directory_id: Option<Uuid>,
}
fn change_in(
    db: &mut Connection,
    request: Change,
    watermark: u64,
    root: &Path,
) -> Result<Option<Directory>, CatalogCommandError> {
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| CatalogCommandError::Unavailable)?;
    let valid: bool = tx.query_row(
        "select exists(select 1 from agent_conversations where id=?1 and archived=0 and durable_count=?2)",
        params![request.conversation_id.to_string(), watermark], |r| r.get(0),
    ).map_err(|_| CatalogCommandError::Unavailable)?;
    let current = read(&tx, request.conversation_id, root)?;
    let expected = request.expected_directory_id.map(|id| id.to_string());
    if !valid || current.as_ref().map(|d| &d.manuscript_id) != expected.as_ref() {
        return Err(CatalogCommandError::Conflict);
    }
    if let Some(id) = request.directory_id {
        let active: bool = tx
            .query_row(
                "select exists(select 1 from manuscripts where id=?1 and status='active')",
                [id.to_string()],
                |r| r.get(0),
            )
            .map_err(|_| CatalogCommandError::Unavailable)?;
        if !active {
            return Err(CatalogCommandError::NotFound);
        }
    }
    tx.execute("delete from agent_conversation_bindings where conversation_id=?1 and resource_kind='manuscript'",
        [request.conversation_id.to_string()]).map_err(|_| CatalogCommandError::Unavailable)?;
    if let Some(id) = request.directory_id {
        tx.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values (?1,'manuscript',?2)",
            params![request.conversation_id.to_string(), id.to_string()]).map_err(|_| CatalogCommandError::Unavailable)?;
    }
    let result = read(&tx, request.conversation_id, root)?;
    tx.commit().map_err(|_| CatalogCommandError::Unavailable)?;
    Ok(result)
}
#[tauri::command]
pub(crate) async fn agent_set_context_directory(
    store: State<'_, AppStore>,
    catalog: State<'_, SqliteConversationCatalog>,
    sessions: State<'_, Arc<AgentSessionService>>,
    request: Change,
) -> Result<Option<Directory>, CatalogCommandError> {
    set_directory(&store, &catalog, sessions.inner().clone(), request).await
}

pub(crate) async fn set_directory(
    store: &AppStore,
    catalog: &SqliteConversationCatalog,
    sessions: Arc<AgentSessionService>,
    request: Change,
) -> Result<Option<Directory>, CatalogCommandError> {
    let store = store.clone();
    let catalog = catalog.clone();
    let sessions = sessions.clone();
    // Keep the existing idle admission reservation until the SQL task settles,
    // even if the IPC waiter is dropped. No separate runtime owner is introduced.
    tokio::spawn(async move {
        let id = ConversationId::from_uuid(request.conversation_id);
        let _reservation = sessions
            .tasks()
            .reserve_idle(id)
            .map_err(|_| CatalogCommandError::Busy)?;
        if catalog.read(id, None).await?.archived {
            return Err(CatalogCommandError::Conflict);
        }
        let snapshot = catalog
            .export_snapshot(id)
            .await
            .map_err(|_| CatalogCommandError::Unavailable)?;
        if snapshot.turns.iter().any(|t| t.outcome.is_none()) {
            return Err(CatalogCommandError::Busy);
        }
        tokio::task::spawn_blocking(move || {
            let mut db = store
                .connection()
                .map_err(|_| CatalogCommandError::Unavailable)?;
            change_in(
                &mut db,
                request,
                snapshot.watermark,
                store.manuscript_directory(),
            )
        })
        .await
        .map_err(|_| CatalogCommandError::Unavailable)?
    })
    .await
    .map_err(|_| CatalogCommandError::Unavailable)?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn directory_configuration_keeps_identity_and_history_and_checks_stale_changes() {
        let mut db = Connection::open_in_memory().unwrap();
        crate::storage::migrate(&mut db).unwrap();
        let chat = Uuid::new_v4();
        let folder = Uuid::new_v4();
        db.execute(
            "insert into research_projects(id,title,summary,created_at) values ('p','p','','now')",
            [],
        )
        .unwrap();
        db.execute("insert into manuscripts(id,project_id,title,created_at,updated_at) values (?1,'p','Folder','now','now')", [folder.to_string()]).unwrap();
        db.execute(
            "insert into agent_conversations(id,title,durable_count) values (?1,'Preserved',3)",
            [chat.to_string()],
        )
        .unwrap();
        let request = |expected, next| Change {
            conversation_id: chat,
            expected_directory_id: expected,
            directory_id: next,
        };
        let root = Path::new("managed");
        let selected = change_in(&mut db, request(None, Some(folder)), 3, root)
            .unwrap()
            .unwrap();
        assert_eq!(
            selected.path,
            root.join(folder.to_string())
                .join("source")
                .to_string_lossy()
        );
        assert!(matches!(
            change_in(&mut db, request(None, None), 3, root),
            Err(CatalogCommandError::Conflict)
        ));
        assert!(matches!(
            change_in(&mut db, request(Some(folder), None), 2, root),
            Err(CatalogCommandError::Conflict)
        ));
        assert!(
            change_in(&mut db, request(Some(folder), None), 3, root)
                .unwrap()
                .is_none()
        );
        let row: (String, u64) = db
            .query_row(
                "select title,durable_count from agent_conversations where id=?1",
                [chat.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(row, ("Preserved".into(), 3));
        db.execute("update agent_conversations set archived=1", [])
            .unwrap();
        assert!(matches!(
            change_in(&mut db, request(None, Some(folder)), 3, root),
            Err(CatalogCommandError::Conflict)
        ));
    }
}
