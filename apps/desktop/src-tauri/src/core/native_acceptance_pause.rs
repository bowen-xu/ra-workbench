//! One-shot, local-only release points for isolated synthetic native tests.
use research_agent_core::model::ModelError;
use std::{
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::{io::AsyncReadExt, net::UnixListener};

pub(crate) enum EndpointName {
    ModelResponse,
}

impl EndpointName {
    pub(crate) fn file_name(&self) -> &'static str {
        match self {
            Self::ModelResponse => "model-release.sock",
        }
    }
}

pub(crate) struct Gate {
    listener: UnixListener,
    _endpoint: Endpoint,
}

struct Endpoint {
    path: PathBuf,
}

impl Gate {
    pub(crate) async fn bind(root: &Path, name: EndpointName) -> Result<Self, ModelError> {
        // The caller has validated the exact synthetic root and operation scope.
        // Keep access private and refuse an existing socket, file or symlink.
        let root = root.to_path_buf();
        let (listener, endpoint) = tokio::task::spawn_blocking(move || {
            if std::fs::metadata(&root)
                .map_err(|_| ModelError::InvalidRequest)?
                .permissions()
                .mode()
                & 0o077
                != 0
            {
                return Err(ModelError::InvalidRequest);
            }
            let path = root.join(name.file_name());
            let listener = std::os::unix::net::UnixListener::bind(&path)
                .map_err(|_| ModelError::InvalidRequest)?;
            let endpoint = Endpoint { path };
            if listener
                .set_nonblocking(true)
                .and_then(|()| {
                    std::fs::set_permissions(&endpoint.path, std::fs::Permissions::from_mode(0o600))
                })
                .is_err()
            {
                return Err(ModelError::Unavailable);
            }
            Ok((listener, endpoint))
        })
        .await
        .map_err(|_| ModelError::Unavailable)??;
        let listener = UnixListener::from_std(listener).map_err(|_| ModelError::Unavailable)?;
        Ok(Self {
            listener,
            _endpoint: endpoint,
        })
    }

    pub(crate) async fn wait(self, limit: Duration) -> Result<(), ModelError> {
        tokio::time::timeout(limit, async {
            let (mut connection, _) = self
                .listener
                .accept()
                .await
                .map_err(|_| ModelError::Unavailable)?;
            match connection.read_u8().await {
                Ok(1) => Ok(()),
                _ => Err(ModelError::InvalidRequest),
            }
        })
        .await
        .map_err(|_| ModelError::Unavailable)?
    }
}

impl Drop for Endpoint {
    fn drop(&mut self) {
        // Unlink this tiny disposable endpoint even when its future is
        // cancelled. No task or listener survives its owning operation.
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
#[path = "native_acceptance_pause/tests.rs"]
mod tests;
