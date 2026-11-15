//! Canonical Native decisions and original Core resource owners for test inputs.
use crate::runtime_plugins::{RuntimePluginCatalog, RuntimePluginPackage};

pub(crate) fn expected_preview(
    catalog: &RuntimePluginCatalog,
    package: &RuntimePluginPackage,
    db: &rusqlite::Connection,
    request: &crate::ai::paper_reading_contracts::PaperContextRequest,
) -> crate::ai::paper_reading_contracts::PaperContextPreview {
    use crate::runtime_plugins::literature_context::preview::PreviewResources;
    use research_workbench_native_sdk::paper_context_preview::PaperContextPreviewDecisionV1;
    let resources = PreviewResources::load(catalog, package).unwrap();
    for capture in [
        resources.header(db, request).unwrap(),
        resources.preflight(db, request).unwrap(),
    ] {
        assert!(matches!(
            research_literature_library_native::paper_context_preview::evaluate(capture.input),
            PaperContextPreviewDecisionV1::Validated {}
        ));
    }
    match research_literature_library_native::paper_context_preview::evaluate(
        resources.snapshot(db, request).unwrap().input,
    ) {
        PaperContextPreviewDecisionV1::Previewed { preview } => preview,
        decision => panic!("unexpected preview decision: {decision:?}"),
    }
}

pub(crate) fn seed_snapshot(
    catalog: &RuntimePluginCatalog,
    package: &RuntimePluginPackage,
    db: &mut rusqlite::Connection,
    paper: &str,
    prepared: &crate::ai::paper_reading_contracts::PaperAiReadingPrepareResult,
    image_hashes: std::collections::BTreeMap<String, String>,
) -> crate::ai::paper_reading_contracts::PaperContextSnapshot {
    use crate::runtime_plugins::literature_context::PaperContextResources;
    use research_workbench_native_sdk::paper_context::SnapshotProjectionInputV1;
    let resources = PaperContextResources::load(catalog, package).unwrap();
    let projection = research_literature_library_native::paper_context_snapshot::project(
        &SnapshotProjectionInputV1 {
            paper_id: paper.into(),
            attachment_statuses: prepared.attachment_statuses.clone(),
            context_items: prepared.context_items.clone(),
            image_hashes,
        },
    )
    .unwrap();
    let captured = resources.capture_snapshot(db, &projection).unwrap();
    let digest = captured
        .existing
        .as_ref()
        .map(|snapshot| snapshot.context_hash.as_str());
    resources
        .apply_snapshot(db, paper, &projection, &captured, digest)
        .unwrap()
}

/// A controlled package of the canonical context SQL, accepted by the original
/// Core resource loaders with their fixed hashes and finite grants.
pub(crate) fn controlled_package() -> (
    tempfile::TempDir,
    std::sync::Arc<RuntimePluginCatalog>,
    RuntimePluginPackage,
) {
    let (directory, catalog, _) =
        crate::runtime_plugins::native::package::tests::literature_fixture(b"#!/bin/sh\nexit 42\n");
    let package = directory.path().join("writing");
    let manifest = package.join("manifest.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
    value["provides"].as_array_mut().unwrap().extend([
        serde_json::json!({"id":"literature.paper-context","version":1}),
        serde_json::json!({"id":"literature.paper-context-preview","version":1}),
        serde_json::json!({"id":"literature.paper-context-prepare","version":1}),
        serde_json::json!({"id":"literature.reading-evidence","version":1}),
    ]);
    std::fs::write(&manifest, serde_json::to_vec(&value).unwrap()).unwrap();
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../plugins/literature_library/native/data");
    for name in [
        "attachments-paper-exists",
        "attachments-all",
        "attachments-exact",
        "snapshot-load-header",
        "snapshot-load-attachments",
        "snapshot-load-items",
        "snapshot-persist-find-hash",
        "snapshot-membership",
        "snapshot-item-geometry",
        "snapshot-repair-delete-items",
        "snapshot-repair-delete-attachments",
        "snapshot-repair-delete-header",
        "snapshot-persist-insert-header",
        "snapshot-persist-insert-attachment",
        "snapshot-persist-insert-item",
        "preview-header",
        "preview-items-budget",
        "preview-attachments-budget",
        "session-latest",
        "session-summary",
    ] {
        let file = format!("paper-context-{name}.v1.sql");
        std::fs::copy(source.join(&file), package.join("data").join(file)).unwrap();
    }
    for name in ["snapshot-hash", "snapshot-rows"] {
        let file = format!("reading-evidence-{name}.v1.sql");
        std::fs::copy(source.join(&file), package.join("data").join(file)).unwrap();
    }
    let package = catalog.discover().unwrap().packages.remove(0);
    super::PaperContextResources::load(&catalog, &package).unwrap();
    super::preview::PreviewResources::load(&catalog, &package).unwrap();
    (directory, catalog, package)
}
