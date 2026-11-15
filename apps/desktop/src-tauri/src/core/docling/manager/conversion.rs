use super::*;
use crate::docling::task_control::{ParseControl, ParseProgress};
#[path = "conversion/wait.rs"]
mod wait;

/// If the last waiter disappears, request cancellation without stopping another lease's daemon.
struct RemoteGuard {
    client: Client,
    service: RunningDoclingService,
    id: String,
    armed: bool,
}
impl RemoteGuard {
    async fn cancel(&self) -> Result<(), DoclingError> {
        self.client
            .delete(format!(
                "{}/local-service/v1/conversions/{}",
                self.service.base_url, self.id
            ))
            .timeout(Duration::from_secs(5))
            .header("X-Api-Key", &self.service.token)
            .header(LEASE_HEADER, &self.service.lease_id)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }
}
impl Drop for RemoteGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let cancellation = Self {
            client: self.client.clone(),
            service: self.service.clone(),
            id: self.id.clone(),
            armed: false,
        };
        tauri::async_runtime::spawn(async move {
            let _ = cancellation.cancel().await;
        });
    }
}
impl DoclingServiceManager {
    pub(crate) async fn convert_pdf(
        &self,
        service: &RunningDoclingService,
        file_path: &Path,
        display_name: &str,
        timeout_ms: u64,
        control: Arc<ParseControl>,
    ) -> Result<Value, DoclingError> {
        control.stage("reading_file");
        let bytes = tokio::fs::read(file_path).await?;
        if control.is_cancelled() {
            return Err(DoclingError::Cancelled);
        }
        let payload = build_convert_pdf_payload(&bytes, display_name);
        let mut guard = RemoteGuard {
            client: self.inner.client.clone(),
            service: service.clone(),
            id: control.id.clone(),
            armed: true,
        };
        let response_finished = std::sync::atomic::AtomicBool::new(false);
        let request = async {
            let response = self
                .inner
                .client
                .post(format!("{}/v1/convert/source", service.base_url))
                .header("X-Api-Key", &service.token)
                .header(LEASE_HEADER, &service.lease_id)
                .header("X-Docling-Request-Id", &control.id)
                .json(&payload)
                .send()
                .await?;
            let status = response.status();
            let text = response.text().await?;
            response_finished.store(true, std::sync::atomic::Ordering::Release);
            if control.is_cancelled() || status == reqwest::StatusCode::GONE {
                return Err(DoclingError::Cancelled);
            }
            if !status.is_success() {
                return Err(DoclingError::Request(format!(
                    "Docling HTTP {}: {}",
                    status.as_u16(),
                    trim_for_message(&text)
                )));
            }
            serde_json::from_str(&text)
                .map_err(|error| DoclingError::InvalidResponse(error.to_string()))
        };
        control.stage("queued");
        let poll = || async {
            Ok(self
                .inner
                .client
                .get(format!(
                    "{}/local-service/v1/conversions/{}",
                    service.base_url, control.id
                ))
                .timeout(Duration::from_secs(2))
                .header("X-Api-Key", &service.token)
                .header(LEASE_HEADER, &service.lease_id)
                .send()
                .await?
                .error_for_status()?
                .json::<ParseProgress>()
                .await?)
        };
        let result =
            wait::for_response(request, poll, || guard.cancel(), &control, timeout_ms).await;
        guard.armed = !response_finished.load(std::sync::atomic::Ordering::Acquire);
        result
    }
}

fn trim_for_message(value: &str) -> String {
    const LIMIT: usize = 300;
    let trimmed = value.trim();
    if trimmed.len() <= LIMIT {
        trimmed.to_owned()
    } else {
        format!("{}…", &trimmed[..LIMIT])
    }
}

pub(crate) fn build_convert_pdf_payload(bytes: &[u8], display_name: &str) -> Value {
    json!({
        "sources": [{
            "kind": "file",
            "base64_string": STANDARD.encode(bytes),
            "filename": display_name,
        }],
        "options": {
            "from_formats": ["pdf"],
            "to_formats": ["json"],
        },
    })
}
