//! Native settings adapter. Selection never modifies these global resources.
use super::{
    service::AgentSessionService,
    skills::import::{self, Error, Preview, Selection, Source},
};
use crate::{ai::JsonCredentialStore, storage::AppStore};
use std::sync::Arc;
use tauri::State;
use uuid::Uuid;
#[tauri::command]
pub(crate) async fn agent_scan_skill_import(
    store: State<'_, AppStore>,
    source: Source,
) -> Result<Preview, Error> {
    import::scan(store.agent_configuration_directory().to_owned(), source).await
}
#[tauri::command]
pub(crate) async fn agent_install_skill_import(
    store: State<'_, AppStore>,
    import_id: Uuid,
    selections: Vec<Selection>,
) -> Result<(), Error> {
    let root = store.agent_configuration_directory().to_owned();
    tokio::task::spawn_blocking(move || import::install(&root, import_id, selections))
        .await
        .map_err(|_| Error::Unavailable)?
}
#[tauri::command]
pub(crate) async fn agent_remove_skill(
    store: State<'_, AppStore>,
    source_id: String,
    revision: String,
) -> Result<(), Error> {
    let root = store.agent_configuration_directory().to_owned();
    tokio::task::spawn_blocking(move || import::remove(&root, &source_id, &revision))
        .await
        .map_err(|_| Error::Unavailable)?
}
#[tauri::command]
pub async fn agent_cancel_skill_import(import_id: Uuid) -> Result<(), Error> {
    import::cancel(import_id)
}
#[tauri::command]
pub(crate) async fn agent_read_mcp_configuration(
    store: State<'_, AppStore>,
) -> Result<String, Error> {
    super::mcp_config::read_text_from_root(store.agent_configuration_directory())
        .map(|text| text.unwrap_or_else(|| "schema_version = 1\n".into()))
        .map_err(|_| Error::Unavailable)
}
#[tauri::command]
pub(crate) async fn agent_save_mcp_configuration(
    store: State<'_, AppStore>,
    sessions: State<'_, Arc<AgentSessionService>>,
    text: String,
    expected_text: String,
) -> Result<(), Error> {
    let root = store.agent_configuration_directory().to_owned();
    tokio::task::spawn_blocking(move || save_mcp(&root, &text, &expected_text))
        .await
        .map_err(|_| Error::Unavailable)??;
    sessions.clear_mcp_runtime().await;
    Ok(())
}
fn save_mcp(configuration: &std::path::Path, text: &str, expected_text: &str) -> Result<(), Error> {
    use std::io::Write;
    if text.len() > 256 * 1024 {
        return Err(Error::Invalid);
    }
    static WRITES: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let write_guard = WRITES.lock().map_err(|_| Error::Unavailable)?;
    let current = super::mcp_config::read_text_from_root(configuration)
        .map_err(|_| Error::Unavailable)?
        .unwrap_or_else(|| "schema_version = 1\n".into());
    if current != expected_text {
        return Err(Error::Conflict);
    }
    let root = import::checked_child(configuration, std::path::Path::new("agent"))?;
    let mut document = text
        .parse::<toml_edit::DocumentMut>()
        .map_err(|_| Error::Invalid)?;
    // Any changed server gets a new configuration identity; saved receipts cannot
    // accidentally authorize a new command, URL or credential handle.
    if let Some(servers) = document.get_mut("servers") {
        let servers = servers.as_table_mut().ok_or(Error::Invalid)?;
        for (_, item) in servers.iter_mut() {
            let table = item.as_table_mut().ok_or(Error::Invalid)?;
            table.remove("revision");
            use sha2::{Digest, Sha256};
            let hash = format!("{:x}", Sha256::digest(table.to_string().as_bytes()));
            table.insert("revision", toml_edit::value(hash));
        }
    }
    let text = document.to_string();
    super::mcp_config::parse(&text).map_err(|_| Error::Invalid)?;
    let mut temp = tempfile::NamedTempFile::new_in(&root).map_err(|_| Error::Unavailable)?;
    temp.write_all(text.as_bytes())
        .map_err(|_| Error::Unavailable)?;
    temp.as_file().sync_all().map_err(|_| Error::Unavailable)?;
    temp.persist(root.join("mcp.toml"))
        .map_err(|_| Error::Unavailable)?;
    std::fs::File::open(&root)
        .and_then(|file| file.sync_all())
        .map_err(|_| Error::Unavailable)?;
    drop(write_guard);
    Ok(())
}

#[tauri::command]
#[allow(private_interfaces)]
pub async fn agent_set_mcp_credential(
    credentials: State<'_, JsonCredentialStore>,
    handle: String,
    secret: Option<String>,
) -> Result<(), Error> {
    credentials
        .set_mcp(&handle, secret.as_deref())
        .map_err(|_| Error::Unavailable)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn saves_configuration_by_original_version_and_rotates_changed_server_identity() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path().canonicalize().unwrap();
        let initial = "schema_version = 1\n";
        let text = "schema_version=1\n[servers.test]\nenvironment_id='application'\ntransport='streamable_http'\nurl='https://example.test/mcp'\n";
        save_mcp(&root, text, initial).unwrap();
        let saved = super::super::mcp_config::read_text_from_root(&root)
            .unwrap()
            .unwrap();
        let first = super::super::mcp_config::parse(&saved).unwrap().remove(0);
        assert_ne!(first.revision, "initial");
        assert!(matches!(
            save_mcp(&root, text, initial),
            Err(Error::Conflict)
        ));
        save_mcp(&root, &saved, &saved).unwrap();
        assert_eq!(
            super::super::mcp_config::read_from_root(&root)
                .unwrap()
                .unwrap()[0]
                .revision,
            first.revision
        );
        let changed = saved.replace("example.test/mcp", "example.test/changed");
        save_mcp(&root, &changed, &saved).unwrap();
        assert_ne!(
            super::super::mcp_config::read_from_root(&root)
                .unwrap()
                .unwrap()[0]
                .revision,
            first.revision
        );
    }
    #[test]
    fn refuses_configuration_links_without_overwriting_the_target() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        std::fs::write(root.join("outside.toml"), "schema_version=1\n").unwrap();
        std::fs::create_dir(root.join("agent")).unwrap();
        std::os::unix::fs::symlink("../outside.toml", root.join("agent/mcp.toml")).unwrap();
        assert!(save_mcp(&root, "schema_version=1\n", "schema_version=1\n").is_err());
        assert_eq!(
            std::fs::read_to_string(root.join("outside.toml")).unwrap(),
            "schema_version=1\n"
        );
    }
}
