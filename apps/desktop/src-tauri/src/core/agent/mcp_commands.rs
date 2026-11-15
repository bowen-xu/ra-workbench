//! Explicit user-approved MCP runtime installation.
//!
//! The frontend supplies only server IDs that the user approved. Configuration
//! and process details stay native; the response contains namespaced tool names
//! only.

use super::{
    mcp_config::{self, ConfiguredServer},
    mcp_runtime::{JsonMcpCredentialResolver, McpRuntime},
    repository::mcp_approvals::Approval,
    service::AgentSessionService,
};
use crate::ai::JsonCredentialStore;
use crate::storage::AppStore;
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, sync::Arc, time::Duration};
use tauri::State;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstallRequest {
    pub approved_server_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallResponse {
    pub schema_version: u8,
    pub server_ids: Vec<String>,
    pub tool_names: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerSummary {
    pub server_id: String,
    pub environment_id: String,
    pub revision: String,
    pub enabled: bool,
    pub credential_configured: bool,
    pub approved: bool,
}

fn approval_matches_server(server: &ConfiguredServer, receipt: Option<&Approval>) -> bool {
    server.enabled
        && receipt.is_some_and(|receipt| {
            receipt.environment_id == server.environment_id && receipt.revision == server.revision
        })
}

fn runtime_is_stale(
    runtime: Option<&[(String, String, String)]>,
    active_approved: &[(String, String, String)],
) -> bool {
    runtime.is_some_and(|identities| identities != active_approved)
}

fn select_approved_servers(
    configured: Vec<ConfiguredServer>,
    approved: &HashSet<String>,
) -> Result<Vec<ConfiguredServer>, Error> {
    let mut selected = configured
        .into_iter()
        .filter(|server| approved.contains(&server.server_id))
        .collect::<Vec<_>>();
    if selected.len() != approved.len() || selected.iter().any(|server| !server.enabled) {
        return Err(Error::ConfigurationInvalid);
    }
    selected.sort_by(|left, right| left.server_id.cmp(&right.server_id));
    Ok(selected)
}

#[tauri::command]
#[allow(private_interfaces)]
pub async fn agent_list_mcp_servers(
    store: State<'_, AppStore>,
    credentials: State<'_, JsonCredentialStore>,
    sessions: State<'_, Arc<AgentSessionService>>,
) -> Result<Vec<ServerSummary>, Error> {
    let root = store.agent_configuration_directory().to_path_buf();
    let configured = tokio::task::spawn_blocking(move || mcp_config::read_from_root(&root))
        .await
        .map_err(|_| Error::ConfigurationUnavailable)?
        .map_err(|error| match error {
            mcp_config::Error::Unavailable => Error::ConfigurationUnavailable,
            _ => Error::ConfigurationInvalid,
        })?
        .unwrap_or_default();
    let runtime = sessions.mcp_runtime().await;
    let runtime_stale = {
        let db = store.connection().map_err(|_| Error::StorageUnavailable)?;
        let mut active_approved = Vec::new();
        for server in configured.iter().filter(|server| server.enabled) {
            let receipt = super::repository::mcp_approvals::read(&db, &server.server_id)
                .map_err(|_| Error::StorageUnavailable)?;
            if approval_matches_server(server, receipt.as_ref()) {
                active_approved.push((
                    server.server_id.clone(),
                    server.environment_id.clone(),
                    server.revision.clone(),
                ));
            }
        }
        active_approved.sort();
        runtime_is_stale(
            runtime.as_ref().map(|runtime| runtime.identities()),
            &active_approved,
        )
    };
    if runtime_stale {
        sessions.clear_mcp_runtime().await;
    }
    let db = store.connection().map_err(|_| Error::StorageUnavailable)?;
    configured
        .into_iter()
        .map(|server| {
            let receipt = super::repository::mcp_approvals::read(&db, &server.server_id)
                .map_err(|_| Error::StorageUnavailable)?;
            let approved = approval_matches_server(&server, receipt.as_ref());
            // A server without a credential handle is intentionally usable
            // without credentials. Only an explicitly configured opaque
            // handle needs to resolve successfully before the UI can approve
            // the server.
            let credential_configured =
                credential_is_configured(&credentials, server.credential_handle.as_deref());
            Ok(ServerSummary {
                server_id: server.server_id,
                environment_id: server.environment_id,
                revision: server.revision,
                enabled: server.enabled,
                credential_configured,
                approved,
            })
        })
        .collect()
}

fn credential_is_configured(credentials: &JsonCredentialStore, handle: Option<&str>) -> bool {
    handle.is_none_or(|handle| credentials.resolve_mcp(handle).is_ok())
}

#[cfg(test)]
mod tests {
    use super::{
        Error, InstallRequest, approval_matches_server, credential_is_configured,
        install_mcp_runtime_from_config, runtime_is_stale, select_approved_servers,
    };
    use crate::agent::mcp_config::{ConfiguredServer, Transport};
    use crate::{
        agent::{repository::mcp_approvals, service::AgentSessionService},
        ai::JsonCredentialStore,
        storage::{self, AppStore},
    };
    use rusqlite::Connection;
    use std::{collections::HashSet, fs, path::PathBuf, sync::Arc};

    #[test]
    fn no_handle_means_no_credentials_are_required() {
        let store = JsonCredentialStore::from_explicit_development_path(
            tempfile::tempdir().unwrap().path().join("credentials.json"),
        );
        assert!(credential_is_configured(&store, None));
        assert!(!credential_is_configured(&store, Some("mcp:missing")));
    }

    fn server(id: &str, enabled: bool) -> ConfiguredServer {
        ConfiguredServer {
            server_id: id.into(),
            environment_id: "test".into(),
            revision: "r1".into(),
            transport: Transport::Stdio {
                program: PathBuf::from("/bin/sh"),
                arguments: vec![],
                directory: std::env::temp_dir(),
            },
            environment: Default::default(),
            credential_handle: None,
            enabled,
        }
    }

    #[test]
    fn rejects_unknown_or_disabled_approved_servers() {
        let unknown = HashSet::from(["missing".to_owned()]);
        assert!(matches!(
            select_approved_servers(vec![server("known", true)], &unknown),
            Err(Error::ConfigurationInvalid)
        ));
        let disabled = HashSet::from(["known".to_owned()]);
        assert!(matches!(
            select_approved_servers(vec![server("known", false)], &disabled),
            Err(Error::ConfigurationInvalid)
        ));
    }

    #[test]
    fn returns_only_configured_servers_in_stable_order() {
        let approved = HashSet::from(["zeta".to_owned(), "alpha".to_owned()]);
        let selected =
            select_approved_servers(vec![server("zeta", true), server("alpha", true)], &approved)
                .unwrap();
        assert_eq!(
            selected
                .iter()
                .map(|server| server.server_id.as_str())
                .collect::<Vec<_>>(),
            ["alpha", "zeta"]
        );
    }

    #[test]
    fn disabled_server_receipt_is_not_reported_as_approved() {
        let server = server("known", false);
        let receipt = super::Approval {
            server_id: "known".into(),
            environment_id: "test".into(),
            revision: "r1".into(),
        };
        assert!(!approval_matches_server(&server, Some(&receipt)));
    }

    #[test]
    fn runtime_refresh_is_required_only_for_an_installed_identity_mismatch() {
        let active = vec![("known".into(), "test".into(), "r2".into())];
        assert!(!runtime_is_stale(None, &active));
        assert!(!runtime_is_stale(Some(&active), &active));
        assert!(runtime_is_stale(
            Some(&[("known".into(), "test".into(), "r1".into())]),
            &active,
        ));
    }

    #[tokio::test]
    async fn installs_app_configured_runtime_into_shared_session_service() {
        let temporary = tempfile::tempdir().unwrap();
        let data_root = temporary.path().canonicalize().unwrap();
        let mut database = Connection::open_in_memory().unwrap();
        storage::migrate(&mut database).unwrap();
        let store = AppStore::for_test(database, data_root.join("objects"));
        let configuration_root = store.agent_configuration_directory();
        fs::create_dir_all(configuration_root.join("agent")).unwrap();
        let server_script = data_root.join("mcp-fixture.sh");
        fs::write(
            &server_script,
            r#"while IFS= read line; do
case "$line" in
  *initialize*) printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{}}' ;;
  *tools/list*) printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"search","description":"search","inputSchema":{"type":"object"}}]}}' ;;
esac
done
"#,
        )
        .unwrap();
        let configuration = format!(
            "schema_version=1\n[servers.literature]\nenvironment_id='project'\nrevision='r1'\nprogram='/bin/sh'\narguments=['{}']\ndirectory='{}'\n",
            server_script.display(),
            data_root.display()
        );
        fs::write(configuration_root.join("agent/mcp.toml"), configuration).unwrap();
        let credentials =
            JsonCredentialStore::from_explicit_development_path(data_root.join("credentials.json"));
        let sessions = Arc::new(AgentSessionService::from_app_store(&store).unwrap());

        let response = install_mcp_runtime_from_config(
            &store,
            &credentials,
            &sessions,
            InstallRequest {
                approved_server_ids: vec!["literature".into()],
            },
        )
        .await
        .unwrap();

        assert_eq!(response.server_ids, ["literature"]);
        assert_eq!(response.tool_names, ["literature__search"]);
        let runtime = sessions.mcp_runtime().await.expect("installed runtime");
        assert_eq!(runtime.definitions().len(), 1);
        assert_eq!(
            runtime.identities(),
            &[("literature".into(), "project".into(), "r1".into())]
        );
        let database = store.connection().unwrap();
        assert_eq!(
            mcp_approvals::read(&database, "literature").unwrap(),
            Some(mcp_approvals::Approval {
                server_id: "literature".into(),
                environment_id: "project".into(),
                revision: "r1".into(),
            })
        );
    }
}

#[tauri::command]
#[allow(private_interfaces)]
pub async fn agent_revoke_mcp_server(
    store: State<'_, AppStore>,
    sessions: State<'_, Arc<AgentSessionService>>,
    server_id: String,
) -> Result<(), Error> {
    if server_id.trim().is_empty()
        || server_id.len() > 256
        || server_id.chars().any(char::is_control)
    {
        return Err(Error::InvalidInput);
    }
    {
        let db = store.connection().map_err(|_| Error::StorageUnavailable)?;
        super::repository::mcp_approvals::revoke(&db, &server_id)
            .map_err(|_| Error::StorageUnavailable)?;
    }
    sessions.clear_mcp_runtime().await;
    Ok(())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Error {
    InvalidInput,
    ConfigurationUnavailable,
    ConfigurationInvalid,
    RuntimeUnavailable,
    StorageUnavailable,
}

#[tauri::command]
#[allow(private_interfaces)]
pub async fn agent_install_mcp_runtime(
    store: State<'_, AppStore>,
    credentials: State<'_, JsonCredentialStore>,
    sessions: State<'_, Arc<AgentSessionService>>,
    request: InstallRequest,
) -> Result<InstallResponse, Error> {
    install_mcp_runtime_from_config(&store, &credentials, &sessions, request).await
}

pub(crate) async fn install_mcp_runtime_from_config(
    store: &AppStore,
    credentials: &JsonCredentialStore,
    sessions: &Arc<AgentSessionService>,
    request: InstallRequest,
) -> Result<InstallResponse, Error> {
    if request.approved_server_ids.len() > 64
        || request
            .approved_server_ids
            .iter()
            .any(|id| id.trim().is_empty() || id.len() > 256 || id.chars().any(char::is_control))
    {
        return Err(Error::InvalidInput);
    }
    let approved = request
        .approved_server_ids
        .into_iter()
        .collect::<HashSet<_>>();
    let root = store.agent_configuration_directory().to_path_buf();
    let configured = tokio::task::spawn_blocking(move || mcp_config::read_from_root(&root))
        .await
        .map_err(|_| Error::ConfigurationUnavailable)?
        .map_err(|error| match error {
            mcp_config::Error::Unavailable => Error::ConfigurationUnavailable,
            _ => Error::ConfigurationInvalid,
        })?
        .unwrap_or_default();
    let configured_server_ids = configured
        .iter()
        .filter(|server| server.enabled)
        .map(|server| server.server_id.clone())
        .collect::<Vec<_>>();
    let selected = select_approved_servers(configured, &approved)?;
    let approvals = selected.clone();
    let server_ids = selected
        .iter()
        .map(|server| server.server_id.clone())
        .collect::<Vec<_>>();
    let runtime = McpRuntime::build_with_credentials(
        selected,
        &approved,
        Duration::from_secs(10),
        Arc::new(JsonMcpCredentialResolver::new(credentials.clone())),
    )
    .await
    .map_err(|_| Error::RuntimeUnavailable)?;
    let tool_names = runtime
        .definitions()
        .into_iter()
        .map(|definition| definition.name)
        .collect::<Vec<_>>();
    {
        let mut db = store.connection().map_err(|_| Error::StorageUnavailable)?;
        let approvals = approvals
            .iter()
            .map(|server| super::repository::mcp_approvals::Approval {
                server_id: server.server_id.clone(),
                environment_id: server.environment_id.clone(),
                revision: server.revision.clone(),
            })
            .collect::<Vec<_>>();
        super::repository::mcp_approvals::reconcile(&mut db, &configured_server_ids, &approvals)
            .map_err(|_| Error::StorageUnavailable)?;
    }
    sessions.install_mcp_runtime(Arc::new(runtime)).await;
    Ok(InstallResponse {
        schema_version: 1,
        server_ids,
        tool_names,
    })
}
