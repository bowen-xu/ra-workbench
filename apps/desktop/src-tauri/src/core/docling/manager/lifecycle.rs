use super::*;

impl DoclingServiceManager {
    /// A fresh desktop process gets one automatic attempt; later starts are explicit.
    pub(crate) async fn start_on_app_start(
        &self,
        settings: &DoclingSettings,
    ) -> Result<RunningDoclingService, DoclingError> {
        if self.inner.startup_attempted.swap(true, Ordering::AcqRel) {
            return self.require_connected(settings).await;
        }
        self.start_service(settings).await
    }

    pub(crate) async fn start_service(
        &self,
        settings: &DoclingSettings,
    ) -> Result<RunningDoclingService, DoclingError> {
        // An explicit user action also consumes any delayed startup attempt.
        self.inner.startup_attempted.store(true, Ordering::Release);
        self.connect(settings, true).await
    }

    pub(super) fn matches_previous_instance(&self, candidate: &str) -> bool {
        let previous = self
            .inner
            .state
            .lock()
            .ok()
            .and_then(|s| s.last_instance_id.clone());
        previous.as_deref() == Some(candidate)
    }

    pub(super) fn discovery_path(&self, base: &Path) -> PathBuf {
        if let Ok(selection) = self.inner.selected_models.lock()
            && let Some((manifest, _)) = selection.as_ref()
            && let Ok(name) = manifest.instance_directory_name()
        {
            return self
                .inner
                .service_directory
                .join("instances")
                .join(name)
                .join(base.file_name().unwrap_or_default());
        }
        base.to_owned()
    }

    pub(super) fn selected_model_identity(&self) -> Option<String> {
        self.inner
            .selected_models
            .lock()
            .ok()?
            .as_ref()?
            .0
            .execution_identity()
            .ok()
    }

    pub(super) fn current_connected(&self) -> Option<RunningDoclingService> {
        let state = self.inner.state.lock().ok()?;
        if state.state != "connected" || state.manually_disconnected {
            return None;
        }
        state
            .service
            .clone()
            .filter(|s| self.model_identity().ok().as_ref() == s.model_identity.as_ref())
    }

    pub(super) fn lease_finished(
        &self,
        generation: u64,
        failure: Option<String>,
        settings: DoclingSettings,
    ) {
        if let Ok(mut session) = self.inner.session.lock()
            && session
                .as_ref()
                .is_some_and(|value| value.generation == generation)
        {
            session.take();
        }
        let should_reconnect = if let Ok(mut state) = self.inner.state.lock() {
            if state.generation != generation {
                return;
            }
            state.service = None;
            state.active_client_count = None;
            state.active_request_count = None;
            if state.manually_disconnected || failure.is_none() {
                state.state = "disconnected".to_owned();
                false
            } else {
                state.state = "reconnecting".to_owned();
                state.last_error = failure;
                true
            }
        } else {
            false
        };
        if should_reconnect {
            let manager = self.clone();
            tokio::spawn(async move {
                for delay in [500, 1_000, 2_000] {
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                    if manager.connect(&settings, false).await.is_ok() {
                        return;
                    }
                }
            });
        }
    }

    pub(super) fn set_state(&self, state_name: &str, last_error: Option<String>) {
        if let Ok(mut state) = self.inner.state.lock() {
            state.state = state_name.to_owned();
            state.last_error = last_error;
        }
    }

    pub(super) fn update_counts(
        &self,
        generation: u64,
        clients: Option<u32>,
        requests: Option<u32>,
    ) {
        if let Ok(mut state) = self.inner.state.lock()
            && state.generation == generation
            && state.state == "connected"
        {
            if let Some(clients) = clients {
                state.active_client_count = Some(clients.max(1));
            }
            if let Some(requests) = requests {
                state.active_request_count = Some(requests);
            }
        }
    }

    pub(super) fn update_instance_counts(
        &self,
        instance_id: &str,
        clients: Option<u32>,
        requests: Option<u32>,
    ) {
        if let Ok(mut state) = self.inner.state.lock()
            && state.state == "connected"
            && state
                .service
                .as_ref()
                .is_some_and(|service| service.instance_id == instance_id)
        {
            if let Some(clients) = clients {
                state.active_client_count = Some(clients.max(1));
            }
            if let Some(requests) = requests {
                state.active_request_count = Some(requests);
            }
        }
    }

    pub(super) fn clear_stale_files(&self) {
        for path in [&self.inner.registry_path, &self.inner.token_path] {
            match fs::remove_file(self.discovery_path(path)) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => {}
            }
        }
    }
}
