use super::*;

impl DoclingServiceManager {
    pub(crate) async fn delete_models(
        &self,
    ) -> Result<super::super::models::ModelStorageReport, DoclingError> {
        let _connect = self.inner.connect_lock.lock().await;
        let _download = self.inner.models.begin_removal()?;
        self.disconnect().await?;
        let instances = self.inner.service_directory.join("instances");
        match fs::read_dir(instances) {
            Ok(entries) => {
                for entry in entries {
                    let entry = entry?;
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    if !entry.file_type()?.is_dir()
                        || name.len() != 64
                        || !name.bytes().all(|b| b.is_ascii_hexdigit())
                    {
                        continue;
                    }
                    let registry = entry.path().join(REGISTRY_FILE_NAME);
                    if !registry.exists() {
                        continue;
                    }
                    let Some(record) = read_registry(&registry)? else {
                        continue;
                    };
                    if !process_is_alive(record.pid) {
                        continue;
                    }
                    validate_registry(&record)?;
                    let token = fs::read_to_string(entry.path().join(TOKEN_FILE_NAME))?;
                    let response = self
                        .inner
                        .client
                        .post(format!(
                            "{}/local-service/v1/shutdown-idle",
                            record.http_base_url
                        ))
                        .timeout(Duration::from_secs(3))
                        .header("X-Api-Key", token.trim())
                        .send()
                        .await?;
                    if response.status() == reqwest::StatusCode::CONFLICT {
                        return Err(DoclingError::Models(
                            "其他客户端或解析任务正在使用模型，未删除任何模型".into(),
                        ));
                    }
                    response.error_for_status()?;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        // Graceful shutdown releases native sessions and their shared file lock.
        // A concurrent new client is safe: its shared lock prevents removal.
        for attempt in 0..40 {
            match self.inner.models.remove_models() {
                Ok(report) => {
                    if let Ok(mut selected) = self.inner.selected_models.lock() {
                        *selected = None;
                    }
                    return Ok(report);
                }
                Err(DoclingError::Models(_)) if attempt < 39 => {
                    tokio::time::sleep(Duration::from_millis(100)).await
                }
                Err(error) => return Err(error),
            }
        }
        unreachable!()
    }
}
