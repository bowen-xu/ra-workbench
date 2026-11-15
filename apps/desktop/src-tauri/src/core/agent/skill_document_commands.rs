//! Read-only skill links resolve catalog identities, never renderer file paths.
use super::{instructions::InstructionReadError, resource_catalog, skills};
use crate::storage::AppStore;
use research_agent_core::{skills::SkillIdentity, user_input::UserInput};
use serde::Serialize;
use std::path::Path;
use tauri::State;
use uuid::Uuid;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Error {
    Unavailable,
    InvalidSelection,
    #[serde(untagged)]
    Native(Box<crate::runtime_plugins::native::NativeHostError>),
}

#[tauri::command]
pub(crate) async fn agent_read_skill_document(
    store: State<'_, AppStore>,
    identity: SkillIdentity,
    scope: Option<resource_catalog::Request>,
) -> Result<String, Error> {
    read_current(store.inner().clone(), identity, scope).await
}

fn catalog_failure(error: resource_catalog::Error) -> Error {
    match error {
        resource_catalog::Error::Native(error) => Error::Native(error),
        _ => Error::InvalidSelection,
    }
}

pub(crate) async fn read_current(
    store: AppStore,
    identity: SkillIdentity,
    scope: Option<resource_catalog::Request>,
) -> Result<String, Error> {
    if identity.environment_id == "application" || store.native_writing_context().is_none() {
        return tokio::task::spawn_blocking(move || read(&store, &identity, scope.as_ref()))
            .await
            .map_err(|_| Error::Unavailable)?;
    }
    let scope = scope.ok_or(Error::InvalidSelection)?;
    if scope.kind != "manuscript" {
        return Err(Error::InvalidSelection);
    }
    let catalog = resource_catalog::list_current(store.clone(), scope.clone())
        .await
        .map_err(catalog_failure)?;
    tokio::task::spawn_blocking(move || {
        read_using(&store, &identity, Some(&scope), |request| {
            resource_catalog::authorize(&store, request).map_err(catalog_failure)?;
            Ok(catalog)
        })
    })
    .await
    .map_err(|_| Error::Unavailable)?
}

fn read(
    store: &AppStore,
    identity: &SkillIdentity,
    scope: Option<&resource_catalog::Request>,
) -> Result<String, Error> {
    read_using(store, identity, scope, |scope| {
        resource_catalog::list(store, scope).map_err(|_| Error::InvalidSelection)
    })
}

fn read_using(
    store: &AppStore,
    identity: &SkillIdentity,
    scope: Option<&resource_catalog::Request>,
    catalog: impl FnOnce(&resource_catalog::Request) -> Result<Vec<resource_catalog::Resource>, Error>,
) -> Result<String, Error> {
    let (root, manuscript) = if identity.environment_id == "application" {
        let catalog = skills::application::discover(store.agent_configuration_directory())
            .map_err(|_| Error::InvalidSelection)?;
        if !catalog
            .skills
            .iter()
            .any(|skill| &skill.identity == identity)
        {
            return Err(Error::InvalidSelection);
        }
        (store.agent_configuration_directory().to_path_buf(), None)
    } else {
        let scope = scope.ok_or(Error::InvalidSelection)?;
        if scope.kind != "manuscript" {
            return Err(Error::InvalidSelection);
        }
        let catalog = catalog(scope)?;
        if !catalog.iter().any(|resource| matches!(&resource.input, UserInput::Skill {identity: found, ..} if found == identity)) {
            return Err(Error::InvalidSelection);
        }
        let id = Uuid::parse_str(&scope.resource_id).map_err(|_| Error::InvalidSelection)?;
        (
            store
                .manuscript_directory()
                .join(id.to_string())
                .join("source"),
            Some(id),
        )
    };
    let root = root.canonicalize().map_err(|_| Error::Unavailable)?;
    let authorize = |opened: &super::instructions::InstructionRootIdentity<'_>| {
        if let Some(id) = manuscript {
            let db = store
                .connection()
                .map_err(|_| InstructionReadError::Unavailable)?;
            super::repository::instruction_trust::is_trusted(&db, id, opened)
                .map_err(|_| InstructionReadError::Unavailable)?
                .then_some(())
                .ok_or(InstructionReadError::Unauthorized)
        } else {
            (opened.canonical_path == root)
                .then_some(())
                .ok_or(InstructionReadError::Unauthorized)
        }
    };
    let descriptor = skills::files::read_descriptor(
        &root,
        Path::new(&identity.source_id),
        &identity.environment_id,
        authorize,
    )
    .map_err(|_| Error::InvalidSelection)?;
    if &descriptor.identity != identity {
        return Err(Error::InvalidSelection);
    }
    let loaded = skills::files::load_selected(&root, &descriptor, true, 1_048_576, authorize)
        .map_err(|_| Error::InvalidSelection)?;
    Ok(loaded.body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn opens_only_the_exact_registered_application_version() {
        let directory = tempfile::tempdir().unwrap();
        let store = AppStore::for_test(
            rusqlite::Connection::open_in_memory().unwrap(),
            directory.path().join("data/objects"),
        );
        let root = store.agent_configuration_directory();
        let package = root.join(".agents/skills/review");
        fs::create_dir_all(&package).unwrap();
        let path = package.join("SKILL.md");
        fs::write(
            &path,
            "---\nname: review\ndescription: Review prose\n---\nBound document",
        )
        .unwrap();
        let identity = skills::application::discover(root).unwrap().skills[0]
            .identity
            .clone();
        assert!(
            read(&store, &identity, None)
                .unwrap()
                .contains("Bound document")
        );
        let mut wrong = identity.clone();
        wrong.source_id = "../../unregistered/SKILL.md".into();
        assert!(matches!(
            read(&store, &wrong, None),
            Err(Error::InvalidSelection)
        ));
        wrong = identity.clone();
        wrong.environment_id = "manuscript:other".into();
        assert!(matches!(
            read(&store, &wrong, None),
            Err(Error::InvalidSelection)
        ));
        fs::write(
            &path,
            "---\nname: review\ndescription: Review prose\n---\nNew version",
        )
        .unwrap();
        assert!(matches!(
            read(&store, &identity, None),
            Err(Error::InvalidSelection)
        ));
    }
}
