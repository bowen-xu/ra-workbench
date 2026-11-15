//! Shared shell adapter for non-manuscript hosts. Reuses the workspace lease,
//! Seatbelt policy, ShellTools and ProcessRegistry; no second execution loop.
use crate::storage::AppStore;
#[cfg(not(target_os = "macos"))]
use research_agent_core::tool_registry::OwnedToolRegistration;
use research_agent_core::{runner::RunError, submission::SubmissionRecord};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
#[cfg(not(target_os = "macos"))]
use std::sync::Arc;
use uuid::Uuid;
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostSummary {
    pub conversation_id: Uuid,
    pub title: String,
    pub kind: String,
    pub resource_id: String,
    pub approved: bool,
}
pub(crate) fn freeze_grant_sql(
    db: &Connection,
    record: &SubmissionRecord,
) -> Result<(), rusqlite::Error> {
    // Text-only legacy fixtures/database readers can operate before this migration.
    let exists: bool = db.query_row(
        "select exists(select 1 from sqlite_master where name='agent_host_shell_grants')",
        [],
        |r| r.get(0),
    )?;
    if !exists {
        return Ok(());
    }
    db.execute("insert or ignore into agent_turn_shell_grants(turn_id,grant_revision) select ?1,g.revision from agent_host_shell_grants g join agent_conversation_bindings b on b.conversation_id=g.conversation_id and b.resource_kind=g.host_kind and b.resource_id=g.resource_id where g.conversation_id=?2",params![record.turn_id.to_string(),record.conversation_id.to_string()])?;
    Ok(())
}
fn grant(db: &Connection, record: &SubmissionRecord) -> Result<Option<String>, RunError> {
    let exists: bool = db
        .query_row(
            "select exists(select 1 from sqlite_master where name='agent_host_shell_grants')",
            [],
            |r| r.get(0),
        )
        .map_err(|_| RunError::EventSink)?;
    if !exists {
        return Ok(None);
    }
    let terminal: bool = db
        .query_row(
            "select exists(select 1 from agent_items where conversation_id=?1 and json_extract(payload_json,'$.payload.turnId')=?2 and json_extract(payload_json,'$.payload.type')='turnFinished')",
            params![record.conversation_id.to_string(), record.turn_id.to_string()],
            |row| row.get(0),
        )
        .map_err(|_| RunError::EventSink)?;
    if terminal {
        return Ok(None);
    }
    let current: Option<(String,String,String)> = db.query_row("select g.revision,g.host_kind,g.resource_id from agent_host_shell_grants g join agent_turn_shell_grants t on t.turn_id=?1 and t.grant_revision=g.revision join agent_conversation_bindings b on b.conversation_id=g.conversation_id and b.resource_kind=g.host_kind and b.resource_id=g.resource_id join agent_conversations c on c.id=g.conversation_id and c.archived=0 where g.conversation_id=?2",params![record.turn_id.to_string(),record.conversation_id.to_string()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|_|RunError::EventSink)?;
    match current {
        Some((revision, kind, id)) if target_exists(db, &kind, &id)? => Ok(Some(revision)),
        _ => Ok(None),
    }
}
fn target_exists(db: &Connection, kind: &str, id: &str) -> Result<bool, RunError> {
    let query = match kind {
        "paper" => {
            "select exists(select 1 from bibliographic_items where id=?1 and trashed_at is null)"
        }
        "note" => {
            "select exists(select 1 from note_pages p where p.id=?1 and p.status='active' and not exists(select 1 from reading_note_sources s where s.page_id=p.id))"
        }
        "readingNote" => {
            "select exists(select 1 from reading_note_sources s join note_pages p on p.id=s.page_id where s.block_id=?1 and p.status='active')"
        }
        _ => return Ok(false),
    };
    db.query_row(query, [id], |row| row.get(0))
        .map_err(|_| RunError::EventSink)
}
#[tauri::command]
pub(crate) async fn agent_list_execution_hosts(
    store: tauri::State<'_, AppStore>,
) -> Result<Vec<HostSummary>, String> {
    let db = store.connection().map_err(|_| "无法读取授权")?;
    let mut stmt=db.prepare("select c.id,c.title,b.resource_kind,b.resource_id,exists(select 1 from agent_host_shell_grants g where g.conversation_id=c.id and g.host_kind=b.resource_kind and g.resource_id=b.resource_id) from agent_conversations c join agent_conversation_bindings b on b.conversation_id=c.id where c.archived=0 and b.resource_kind!='manuscript' order by c.updated_at desc limit 256").map_err(|_|"无法读取授权")?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
            ))
        })
        .map_err(|_| "无法读取授权")?;
    rows.map(|r| {
        let (id, title, kind, resource_id, approved) = r.map_err(|_| "无法读取授权")?;
        Ok(HostSummary {
            conversation_id: Uuid::parse_str(&id).map_err(|_| "授权身份无效")?,
            title,
            kind,
            resource_id,
            approved,
        })
    })
    .collect()
}
#[tauri::command]
pub(crate) async fn agent_set_host_shell_authorization(
    store: tauri::State<'_, AppStore>,
    conversation_id: Uuid,
    approved: bool,
) -> Result<(), String> {
    let mut db = store.connection().map_err(|_| "无法保存授权")?;
    let tx = db.transaction().map_err(|_| "无法保存授权")?;
    let(kind,id):(String,String)=tx.query_row("select b.resource_kind,b.resource_id from agent_conversation_bindings b join agent_conversations c on c.id=b.conversation_id where c.id=?1 and c.archived=0 and b.resource_kind!='manuscript'",[conversation_id.to_string()],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|"对话授权范围无效")?;
    if !target_exists(&tx, &kind, &id).map_err(|_| "对话宿主已不可用")? {
        return Err("对话宿主已不可用".into());
    }
    if approved {
        tx.execute("insert into agent_host_shell_grants(conversation_id,host_kind,resource_id,revision) values(?1,?2,?3,?4) on conflict(conversation_id) do update set host_kind=excluded.host_kind,resource_id=excluded.resource_id,revision=excluded.revision",params![conversation_id.to_string(),kind,id,Uuid::new_v4().to_string()]).map_err(|_|"无法保存授权")?;
    } else {
        tx.execute(
            "delete from agent_host_shell_grants where conversation_id=?1",
            [conversation_id.to_string()],
        )
        .map_err(|_| "无法保存授权")?;
    }
    tx.commit().map_err(|_| "无法保存授权".into())
}
#[cfg(target_os = "macos")]
#[path = "host_execution/execution.rs"]
mod execution;
#[cfg(target_os = "macos")]
pub(crate) use execution::registration;
#[cfg(target_os = "macos")]
pub(crate) use execution::registration_with_hooks;
pub(crate) struct HostTools {
    pub tools: research_agent_core::tool_registry::OwnedToolRegistration,
    pub workspace: super::compaction_hook_factory::HookWorkspaceHandle,
}
#[cfg(not(target_os = "macos"))]
pub(crate) async fn registration_with_hooks(
    store: AppStore,
    record: SubmissionRecord,
    sessions: Arc<super::service::AgentSessionService>,
    existing: OwnedToolRegistration,
) -> Result<HostTools, RunError> {
    Ok(HostTools {
        tools: registration(store, record, sessions, existing).await?,
        workspace: None,
    })
}
#[cfg(not(target_os = "macos"))]
pub(crate) async fn registration(
    store: AppStore,
    record: SubmissionRecord,
    _: Arc<super::service::AgentSessionService>,
    existing: OwnedToolRegistration,
) -> Result<OwnedToolRegistration, RunError> {
    let db = store.connection().map_err(|_| RunError::EventSink)?;
    if grant(&db, &record)?.is_some() {
        return Err(RunError::ToolUnavailable);
    }
    Ok(existing)
}

#[cfg(test)]
#[path = "host_execution/tests.rs"]
pub(crate) mod tests;
