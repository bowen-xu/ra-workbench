//! Codex-style exact pending write decisions, owned by one AppStore instance.
//! Closing/canceling the tool drops its receiver: stale UI requests cannot write.
use serde::Serialize;
use std::{collections::HashMap, sync::Mutex};
use tokio::sync::oneshot;
use uuid::Uuid;
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    pub id: String,
    pub conversation_id: String,
    pub document_id: String,
    pub name: String,
    pub path: String,
    pub before: String,
    pub after: String,
}
#[derive(Default)]
pub struct Approvals {
    requests: Mutex<HashMap<Uuid, (Request, oneshot::Sender<bool>)>>,
}
impl Approvals {
    pub fn pending(&self, conversation: Uuid) -> Result<Vec<Request>, String> {
        let mut values = self.requests.lock().map_err(|_| "无法读取写入授权")?;
        values.retain(|_, (_, sender)| !sender.is_closed());
        Ok(values
            .values()
            .filter(|(request, _)| request.conversation_id == conversation.to_string())
            .map(|(r, _)| r.clone())
            .collect())
    }
    pub fn decide(&self, conversation: Uuid, id: Uuid, allow: bool) -> Result<(), String> {
        let mut values = self.requests.lock().map_err(|_| "无法处理写入授权")?;
        let (request, _) = values.get(&id).ok_or("写入请求已结束")?;
        if request.conversation_id != conversation.to_string() {
            return Err("写入请求不属于当前对话".into());
        }
        let (_, sender) = values.remove(&id).ok_or("写入请求已结束")?;
        sender.send(allow).map_err(|_| "写入请求已取消".into())
    }
    pub async fn ask(
        &self,
        conversation: Uuid,
        document: &super::Document,
        after: &str,
        cancellation: &crate::agent::cancellation::AgentCancellation,
    ) -> Result<(), String> {
        if cancellation.is_cancelled() {
            return Err("本次内置文档写入已取消".into());
        }
        let id = Uuid::new_v4();
        let (sender, receiver) = oneshot::channel();
        {
            let mut values = self.requests.lock().map_err(|_| "无法请求写入授权")?;
            values.retain(|_, (_, sender)| !sender.is_closed());
            if values.len() >= 16 {
                return Err("等待授权的写入过多，请先处理现有请求".into());
            }
            values.insert(
                id,
                (
                    Request {
                        id: id.to_string(),
                        conversation_id: conversation.to_string(),
                        document_id: document.id.clone(),
                        name: document.name.clone(),
                        path: document.path.clone(),
                        before: document.content.clone(),
                        after: after.into(),
                    },
                    sender,
                ),
            );
        }
        let result = tokio::select! { biased;
            _ = cancellation.cancelled() => None,
            result = tokio::time::timeout(std::time::Duration::from_secs(1800), receiver) => Some(result),
        };
        if let Ok(mut values) = self.requests.lock() {
            values.remove(&id);
        }
        match result {
            Some(Ok(Ok(true))) => Ok(()),
            Some(Ok(Ok(false))) => Err(serde_json::json!({
                "code": "edit_declined",
                "message": "用户拒绝本次内置文档写入；原内容保持不变",
                "operationId": id,
                "conversationId": conversation,
            })
            .to_string()),
            _ => Err("用户未授权本次内置文档写入；原内容保持不变".into()),
        }
    }
}
