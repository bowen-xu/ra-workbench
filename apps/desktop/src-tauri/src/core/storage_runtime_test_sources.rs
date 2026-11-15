//! Explicit compiled package fixtures for Core tests. Production startup is unchanged.
use super::AppStore;
use crate::runtime_plugins::{
    RuntimePluginCatalog,
    literature_trace::{binding::NoteTraceBinding, service::LiteratureTraceService},
    native::NativePreviewService,
    writing_projects::context::Binding,
};
use std::{path::PathBuf, sync::Arc};

fn selected_directory(variable: &str) -> Option<PathBuf> {
    let selected = PathBuf::from(std::env::var_os(variable)?);
    assert!(
        std::fs::symlink_metadata(&selected).unwrap().is_dir(),
        "native test packages require a real directory"
    );
    let directory = selected.canonicalize().unwrap();
    let temporary = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../.tmp")
        .canonicalize()
        .unwrap();
    assert!(directory.starts_with(&temporary) && directory != temporary);
    Some(directory)
}

fn selected_package(catalog: &RuntimePluginCatalog, id: &str) -> String {
    let listing = catalog.discover().unwrap();
    let mut matching = listing
        .packages
        .iter()
        .filter(|package| package.manifest["id"] == id);
    let package = matching
        .next()
        .expect("the selected canonical native package");
    assert!(
        matching.next().is_none(),
        "native test package must be unique"
    );
    package.package_key.clone()
}

pub(super) fn attach(mut store: AppStore) -> AppStore {
    if let Some(directory) = selected_directory("RESEARCH_ASSISTANT_TEST_WRITING_V2_PACKAGES") {
        let catalog = Arc::new(RuntimePluginCatalog::new(Some(directory)));
        let package_key = selected_package(&catalog, "writing_projects");
        store = store.with_native_writing_context(Binding {
            service: Arc::new(NativePreviewService::new(Arc::clone(&catalog))),
            catalog,
            package_key,
        });
    }
    if let Some(directory) = selected_directory("RESEARCH_ASSISTANT_TEST_LITERATURE_PACKAGE") {
        let collection = directory.parent().unwrap().to_path_buf();
        let catalog = Arc::new(RuntimePluginCatalog::new(Some(collection)));
        let package_key = selected_package(&catalog, "literature_library");
        store = store.with_native_note_trace(NoteTraceBinding {
            service: Arc::new(LiteratureTraceService::new(catalog)),
            package_key,
        });
    }
    store
}
