//! Closed literature method set; declarations do not authorize a SQL resource.
use super::Capability;
use research_workbench_native_sdk::{
    catalog, catalog_save, duplicate_merge_prepare, duplicate_review, duplicate_undo, import_jobs,
    literature_lifecycle, literature_trace_protocol, metadata, nal_annotation_prepare,
    nal_bootstrap, nal_mutation, nal_query, trash_cleanup,
};
use std::collections::BTreeSet;

pub(super) fn capabilities_match(actual: &[Capability], expected: &[&str]) -> bool {
    actual.len() == expected.len()
        && actual
            .iter()
            .all(|value| value.version == 1 && expected.contains(&value.id.as_str()))
        && unique(actual)
}

pub(super) fn literature_matches(actual: &[Capability], frontend: bool) -> bool {
    let mut required = vec![literature_trace_protocol::CAPABILITY_ID];
    if frontend {
        required.extend(["literature.catalog", "literature.citations"]);
    }
    let mut allowed = required.clone();
    allowed.extend([
        catalog::CAPABILITY_ID,
        catalog_save::CAPABILITY_ID,
        nal_query::CAPABILITY_ID,
        nal_bootstrap::CAPABILITY_ID,
        nal_annotation_prepare::CAPABILITY_ID,
        nal_mutation::CAPABILITY_ID,
        duplicate_review::CAPABILITY_ID,
        duplicate_undo::CAPABILITY_ID,
        duplicate_merge_prepare::CAPABILITY_ID,
        literature_lifecycle::CAPABILITY_ID,
        trash_cleanup::CAPABILITY_ID,
        metadata::CAPABILITY_ID,
        import_jobs::CAPABILITY_ID,
        research_workbench_native_sdk::doi_metadata::CAPABILITY_ID,
        research_workbench_native_sdk::reference_records::CAPABILITY_ID,
        research_workbench_native_sdk::evidence_matrix::CAPABILITY_ID,
        research_workbench_native_sdk::derived_view::manual::CAPABILITY_ID,
        research_workbench_native_sdk::evidence::CAPABILITY_ID,
        research_workbench_native_sdk::evidence_write::CAPABILITY_ID,
        research_workbench_native_sdk::research_export::CAPABILITY_ID,
        research_workbench_native_sdk::browser_capture::CAPABILITY_ID,
        research_workbench_native_sdk::reading_history::CAPABILITY_ID,
        research_workbench_native_sdk::paper_confirmation::wire::CAPABILITY_ID,
        "literature.attachments",
        "literature.pdf-import",
        research_workbench_native_sdk::concepts::CAPABILITY_ID,
        research_workbench_native_sdk::reading_units::CAPABILITY_ID,
        "literature.reading-evidence",
        research_workbench_native_sdk::structured_validation::CAPABILITY_ID,
        research_workbench_native_sdk::review_selection::CAPABILITY_ID,
        research_workbench_native_sdk::editable_template::CAPABILITY_ID,
        research_workbench_native_sdk::paper_context_wire::CAPABILITY_ID,
        research_workbench_native_sdk::paper_context_preview::CAPABILITY_ID,
        research_workbench_native_sdk::paper_context_prepare::CAPABILITY_ID,
        research_workbench_native_sdk::context_metadata::CAPABILITY_ID,
        research_workbench_native_sdk::analysis_history::CAPABILITY_ID,
        research_workbench_native_sdk::analysis_workflow::wire::CAPABILITY_ID,
        research_workbench_native_sdk::paper_retrieval::CAPABILITY_ID,
        research_workbench_native_sdk::concept_retrieval::CAPABILITY_ID,
        research_workbench_native_sdk::semantic_index::rules::CAPABILITY_ID,
    ]);
    let declares = |id: &str| actual.iter().any(|value| value.id == id);
    unique(actual)
        && actual
            .iter()
            .all(|value| value.version == 1 && allowed.contains(&value.id.as_str()))
        && required.iter().all(|id| declares(id))
        && (!declares(catalog_save::CAPABILITY_ID) || declares(catalog::CAPABILITY_ID))
        && (!declares(research_workbench_native_sdk::paper_context_preview::CAPABILITY_ID)
            || declares(research_workbench_native_sdk::paper_context_wire::CAPABILITY_ID))
        && (!declares(research_workbench_native_sdk::paper_context_prepare::CAPABILITY_ID)
            || declares(research_workbench_native_sdk::paper_context_wire::CAPABILITY_ID))
        && (!declares(research_workbench_native_sdk::browser_capture::CAPABILITY_ID)
            || (declares(research_workbench_native_sdk::catalog_save::CAPABILITY_ID)
                && declares("literature.attachments")))
        && (!declares(research_workbench_native_sdk::reading_history::CAPABILITY_ID)
            || (declares(research_workbench_native_sdk::paper_context_prepare::CAPABILITY_ID)
                && declares(research_workbench_native_sdk::analysis_history::CAPABILITY_ID)))
        && (!declares(research_workbench_native_sdk::research_export::CAPABILITY_ID)
            || declares(research_workbench_native_sdk::analysis_workflow::wire::CAPABILITY_ID))
        && (!declares("literature.pdf-import")
            || [
                "literature.attachments",
                import_jobs::CAPABILITY_ID,
                catalog::CAPABILITY_ID,
                catalog_save::CAPABILITY_ID,
            ]
            .iter()
            .all(|id| declares(id)))
}

fn unique(actual: &[Capability]) -> bool {
    actual
        .iter()
        .map(|value| &value.id)
        .collect::<BTreeSet<_>>()
        .len()
        == actual.len()
}
