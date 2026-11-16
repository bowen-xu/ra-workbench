//! Finite public wire fixtures and original Core resource owners for test inputs.
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
            preview_for(catalog, package, capture.input),
            PaperContextPreviewDecisionV1::Validated {}
        ));
    }
    match preview_for(
        catalog,
        package,
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
    let input = SnapshotProjectionInputV1 {
        paper_id: paper.into(),
        attachment_statuses: prepared.attachment_statuses.clone(),
        context_items: prepared.context_items.clone(),
        image_hashes,
    };
    use research_workbench_native_sdk::paper_context_wire::{
        PaperContextDecisionV1, PaperContextInputV1, decode_payload, encode_payload,
    };
    let decision = context_reply(
        catalog,
        package,
        PaperContextInputV1::Project {
            cbor_base64: encode_payload(&input).unwrap(),
        },
    );
    let PaperContextDecisionV1::Projected {
        projection_cbor_base64,
    } = decision
    else {
        panic!("snapshot projection response")
    };
    let projection = decode_payload(&projection_cbor_base64).unwrap();
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
        .join("../../../tests/fixtures/native-resources-v1/literature");
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

/// Fixed wire responses keep Core transport and storage tests self-contained.
pub(crate) fn policy_catalog(
    input: research_workbench_native_sdk::catalog::CatalogInput,
) -> research_workbench_native_sdk::catalog::CatalogOutput {
    use research_workbench_native_sdk::catalog::{CatalogInput, CatalogOutput};
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/fixtures/paper-context-policy-v1/cases.json");
    let cases: Vec<(CatalogInput, CatalogOutput)> =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    cases
        .into_iter()
        .find_map(|(expected, output)| (input == expected).then_some(output))
        .unwrap_or_else(|| panic!("missing fixed Core context policy response: {input:?}"))
}

fn context_reply(
    catalog: &RuntimePluginCatalog,
    package: &RuntimePluginPackage,
    input: research_workbench_native_sdk::paper_context_wire::PaperContextInputV1,
) -> research_workbench_native_sdk::paper_context_wire::PaperContextDecisionV1 {
    use research_workbench_native_sdk::catalog::{CatalogInput, CatalogOutput};
    let request = CatalogInput::PreparePaperContext {
        input: Box::new(input),
    };
    let entry = package.manifest["native"]["entry"].as_str().unwrap();
    let executable = catalog.resource(&package.package_key, entry).unwrap();
    let output = if executable.bytes.starts_with(b"#!") {
        policy_catalog(request)
    } else {
        crate::runtime_plugins::native::package::tests::catalog_oracle(
            catalog,
            &package.package_key,
            request,
        )
    };
    let CatalogOutput::PaperContext { outcome } = output else {
        panic!("context response")
    };
    outcome.decision
}

pub(crate) fn build(
    input: &research_workbench_native_sdk::paper_context::PaperContextBuildInputV1,
) -> research_workbench_native_sdk::paper_context::PreparedPaperContextV1 {
    use research_workbench_native_sdk::{catalog::*, paper_context_wire::*};
    let CatalogOutput::PaperContext { outcome } =
        policy_catalog(CatalogInput::PreparePaperContext {
            input: Box::new(PaperContextInputV1::Build {
                cbor_base64: encode_payload(input).unwrap(),
            }),
        })
    else {
        panic!("context response")
    };
    let PaperContextDecisionV1::Built {
        prepared_cbor_base64,
    } = outcome.decision
    else {
        panic!("built response")
    };
    decode_payload(&prepared_cbor_base64).unwrap()
}

pub(crate) fn project(
    input: &research_workbench_native_sdk::paper_context::SnapshotProjectionInputV1,
) -> Result<research_workbench_native_sdk::paper_context::SnapshotProjectionV1, String> {
    use research_workbench_native_sdk::{catalog::*, paper_context_wire::*};
    let CatalogOutput::PaperContext { outcome } =
        policy_catalog(CatalogInput::PreparePaperContext {
            input: Box::new(PaperContextInputV1::Project {
                cbor_base64: encode_payload(input).unwrap(),
            }),
        })
    else {
        panic!("context response")
    };
    match outcome.decision {
        PaperContextDecisionV1::Projected {
            projection_cbor_base64,
        } => Ok(decode_payload(&projection_cbor_base64).unwrap()),
        PaperContextDecisionV1::Rejected { message } => Err(message),
        _ => panic!("projected response"),
    }
}

pub(crate) fn digest(
    input: &research_workbench_native_sdk::paper_context::SnapshotDigestInputV1,
) -> Result<String, String> {
    use research_workbench_native_sdk::{catalog::*, paper_context_wire::*};
    let CatalogOutput::PaperContext { outcome } =
        policy_catalog(CatalogInput::PreparePaperContext {
            input: Box::new(PaperContextInputV1::Digest {
                cbor_base64: encode_payload(input).unwrap(),
            }),
        })
    else {
        panic!("context response")
    };
    match outcome.decision {
        PaperContextDecisionV1::Digested { context_hash } => Ok(context_hash),
        PaperContextDecisionV1::Rejected { message } => Err(message),
        _ => panic!("digest response"),
    }
}

pub(crate) fn preview(
    input: research_workbench_native_sdk::paper_context_preview::PaperContextPreviewInputV1,
) -> research_workbench_native_sdk::paper_context_preview::PaperContextPreviewDecisionV1 {
    use research_workbench_native_sdk::{catalog::*, paper_context_wire::*};
    let CatalogOutput::PaperContext { outcome } =
        policy_catalog(CatalogInput::PreparePaperContext {
            input: Box::new(PaperContextInputV1::Preview {
                cbor_base64: encode_payload(&input).unwrap(),
            }),
        })
    else {
        panic!("context response")
    };
    let PaperContextDecisionV1::Previewed {
        decision_cbor_base64,
    } = outcome.decision
    else {
        panic!("preview response")
    };
    decode_payload(&decision_cbor_base64).unwrap()
}

pub(crate) fn retrieval(
    input: research_workbench_native_sdk::paper_retrieval::PaperRetrievalInputV1,
) -> research_workbench_native_sdk::paper_retrieval::PaperRetrievalOutcomeV1 {
    use research_workbench_native_sdk::catalog::*;
    let CatalogOutput::PaperRetrievalTrace { outcome } =
        policy_catalog(CatalogInput::DecidePaperRetrievalTrace {
            input: Box::new(input),
        })
    else {
        panic!("retrieval response")
    };
    *outcome
}

fn preview_for(
    catalog: &RuntimePluginCatalog,
    package: &RuntimePluginPackage,
    input: research_workbench_native_sdk::paper_context_preview::PaperContextPreviewInputV1,
) -> research_workbench_native_sdk::paper_context_preview::PaperContextPreviewDecisionV1 {
    use research_workbench_native_sdk::paper_context_wire::*;
    let PaperContextDecisionV1::Previewed {
        decision_cbor_base64,
    } = context_reply(
        catalog,
        package,
        PaperContextInputV1::Preview {
            cbor_base64: encode_payload(&input).unwrap(),
        },
    )
    else {
        panic!("preview response")
    };
    decode_payload(&decision_cbor_base64).unwrap()
}
