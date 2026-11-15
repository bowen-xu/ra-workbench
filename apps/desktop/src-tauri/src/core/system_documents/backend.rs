use super::*;
use crate::storage::AppStore;
use research_agent_core::{submission::SubmissionRecord, text_patch::backend::*};
pub struct Backend {
    pub store: AppStore,
    pub record: SubmissionRecord,
    pub cancellation: crate::agent::cancellation::AgentCancellation,
}
impl TextBackend for Backend {
    fn read<'a>(
        &'a self,
        path: &'a str,
        _saved: bool,
        remember: bool,
    ) -> BackendFuture<'a, TextSnapshot> {
        Box::pin(async move {
            let id = registry::id_from_path(path)?;
            let root = self.store.agent_configuration_directory().to_path_buf();
            let document = tokio::task::spawn_blocking(move || read(&root, id))
                .await
                .map_err(|_| "无法读取内置文档")??;
            let snapshot = TextSnapshot {
                path: document.path,
                name: Some(document.name),
                text: document.content,
                version: document.revision,
            };
            let db = self.store.connection().map_err(|e| e.to_string())?;
            if remember {
                crate::agent::text_tools::journal::remember(&db, &self.record, &snapshot)?;
            }
            Ok(snapshot)
        })
    }
    fn preflight<'a>(&'a self, changes: &'a [TextChange]) -> BackendFuture<'a, ()> {
        Box::pin(async move {
            if changes.len() != 1 {
                return Err("内置文档每次仅修改一个文件".into());
            }
            let change = &changes[0];
            let id = registry::id_from_path(&change.path)?;
            if change.destination.is_some() || change.before.is_none() || change.after.is_none() {
                return Err("内置文档只支持 Update File，不能创建、删除或移动".into());
            }
            registry::validate(id, change.after.as_deref().ok_or("缺少修改内容")?)
        })
    }
    fn commit<'a>(
        &'a self,
        request_id: &'a str,
        changes: Vec<TextChange>,
    ) -> BackendFuture<'a, Vec<TargetResult>> {
        Box::pin(async move {
            if let Some(result) = self.query(request_id).await? {
                return Ok(result);
            }
            self.preflight(&changes).await?;
            let change = &changes[0];
            let id = registry::id_from_path(&change.path)?;
            let root = self.store.agent_configuration_directory().to_path_buf();
            let current = tokio::task::spawn_blocking(move || read(&root, id))
                .await
                .map_err(|_| "无法读取内置文档")??;
            let before = change.before.as_ref().ok_or("请先读取目标")?;
            if current.revision != before.version || current.content != before.text {
                return Err("文档已变化，请重新读取后修改".into());
            }
            self.store
                .system_document_approvals()
                .ask(
                    self.record.conversation_id,
                    &current,
                    change.after.as_deref().unwrap(),
                    &self.cancellation,
                )
                .await?;
            let root = self.store.agent_configuration_directory().to_path_buf();
            let expected = current.revision;
            let content = change.after.clone().unwrap();
            let baseline = current.baseline;
            let request = format!("{}:{request_id}", self.record.conversation_id);
            let cancellation = self.cancellation.clone();
            let saved = tokio::task::spawn_blocking(move || {
                cancellation
                    .commit_if_active(|| {
                        storage::commit(&root, id, &expected, &content, &baseline, &request)
                    })
                    .map_err(|_| "用户已取消本次内置文档写入".to_string())?
            })
            .await
            .map_err(|_| "无法保存内置文档")??;
            Ok(vec![result(saved)])
        })
    }
    fn query<'a>(&'a self, request_id: &'a str) -> BackendFuture<'a, Option<Vec<TargetResult>>> {
        Box::pin(async move {
            let root = self.store.agent_configuration_directory().to_path_buf();
            let request = format!("{}:{request_id}", self.record.conversation_id);
            let document = tokio::task::spawn_blocking(move || storage::receipt(&root, &request))
                .await
                .map_err(|_| "无法读取写入回执")??;
            Ok(document.map(|document| vec![result(document)]))
        })
    }
}
fn result(document: Document) -> TargetResult {
    TargetResult {
        path: document.path,
        name: Some(document.name),
        status: TargetStatus::Applied,
        message: "修改已保存并立即生效；Diff 仅用于 Undo/Keep 审阅".into(),
        change_id: None,
        generation: None,
    }
}
