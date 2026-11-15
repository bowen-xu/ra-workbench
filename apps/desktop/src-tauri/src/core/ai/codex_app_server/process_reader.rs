//! A reader may publish or clean up only its own process generation.
use super::*;

impl CodexAppServerManager {
    pub(super) fn spawn_reader(&self, stdout: tokio::process::ChildStdout, generation: Uuid) {
        let bridge = self.clone();
        tauri::async_runtime::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(message) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if !bridge.read_message(generation, message).await {
                    return;
                }
            }
            bridge.reader_closed(generation).await;
        });
    }

    async fn read_message(&self, generation: Uuid, message: Value) -> bool {
        let state = self.process.lock().await;
        if state.generation != generation {
            return false;
        }
        if let Some(id) = message.get("id").and_then(Value::as_u64)
            && let Ok(mut entries) = self.pending.lock()
            && let Some(sender) = entries.remove(&id)
        {
            let result = if message.get("error").is_some() {
                Err(AiError::Provider("invalid_response"))
            } else {
                Ok(message.get("result").cloned().unwrap_or(Value::Null))
            };
            let _ = sender.send(result);
        } else if message.get("method").is_some() {
            if message.get("method").and_then(Value::as_str) == Some("account/updated") {
                self.auth_epoch.send_replace(Uuid::new_v4());
            }
            let _ = self.notifications.send(message);
        }
        true
    }

    async fn reader_closed(&self, generation: Uuid) {
        let mut state = self.process.lock().await;
        if state.generation != generation {
            return;
        }
        state.generation = Uuid::new_v4();
        self.auth_epoch.send_replace(Uuid::new_v4());
        self.fail_pending();
        state.initialized = false;
        state.stdin = None;
        state.child = None;
    }

    pub(super) fn fail_pending(&self) {
        if let Ok(mut entries) = self.pending.lock() {
            for (_, sender) in entries.drain() {
                let _ = sender.send(Err(AiError::TemporaryUnavailable));
            }
        }
    }
}

#[cfg(test)]
#[path = "process_reader/tests.rs"]
mod tests;
