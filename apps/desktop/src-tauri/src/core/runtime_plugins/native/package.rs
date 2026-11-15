//! Manifest execution eligibility is narrower than graph resolution. The Web
//! SDK remains the only dependency resolver; execution uses only frozen profiles.
mod browser_capture;
mod reading_history;
mod staging;
pub(crate) use reading_history::accept_reading_history;
mod paper_confirmation;
pub(crate) use paper_confirmation::accept_paper_confirmation;
mod research_export;
use super::{
    NativeHostError, NativePreviewError, NativeProfile, NativeResult, RuntimePluginCatalog,
};
use crate::runtime_plugins::RuntimePluginPackage;
pub(crate) use browser_capture::accept_browser_capture;
pub(crate) use research_export::accept_research_export;
use serde::Deserialize;
#[cfg(test)]
use std::fs;
use std::{collections::BTreeSet, path::PathBuf};

mod analysis_history;
mod analysis_workflow;
#[cfg(test)]
mod attachment_tests;
mod attachments;
mod capabilities;
mod concepts;
mod context_metadata;
mod derived_views;
mod evidence_matrix;
mod evidence_read;
mod evidence_write;
mod reading_evidence;
mod reading_units;
mod reference_records;
mod writing_combined;
use capabilities::{capabilities_match, literature_matches};
pub(crate) use {
    analysis_history::accept_analysis_history,
    analysis_workflow::accept_analysis_workflow,
    attachments::{accept_attachments, accept_pdf_import},
    concepts::accept_concepts,
    context_metadata::accept_context_metadata,
    derived_views::accept_derived_views,
    evidence_matrix::accept_evidence_matrix,
    evidence_read::accept_evidence_read,
    evidence_write::accept_evidence_write,
    reading_evidence::{
        accept_concept_retrieval, accept_editable_template, accept_hbs_validation,
        accept_paper_context, accept_paper_context_prepare, accept_paper_context_preview,
        accept_paper_retrieval_trace, accept_reading_evidence, accept_semantic_index,
        accept_unreviewed_selection,
    },
    reading_units::accept_reading_units,
    reference_records::accept_reference_records,
};
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    format_version: u32,
    id: String,
    version: String,
    api_version: u32,
    dependencies: Requirements,
    provides: Vec<Capability>,
    consumes: Requirements,
    native: NativeEntry,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Requirements {
    required: Vec<serde_json::Value>,
    optional: Vec<serde_json::Value>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Capability {
    id: String,
    version: u32,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NativeEntry {
    protocol_version: u32,
    os: String,
    arch: String,
    entry: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum PackageManifest {
    NativeOnly(Manifest),
    Combined(CombinedManifest),
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CombinedManifest {
    format_version: u32,
    id: String,
    version: String,
    api_version: u32,
    dependencies: Requirements,
    provides: Vec<Capability>,
    consumes: CoreRequirements,
    native: NativeEntry,
    entry: String,
    styles: Vec<String>,
    ui_runtime: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CoreRequirements {
    required: Vec<CoreRequirement>,
    optional: Vec<CoreRequirement>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CoreRequirement {
    id: String,
    version: u32,
    provider: String,
}

// This is the current combined package's closed metadata footprint, not a
// Core capability registry or a native permission derived from Web metadata.
const LITERATURE_REQUIRED: [&str; 13] = [
    "core.workbench",
    "core.workbench.source-workspace-host",
    "core.sdk.literature-shared",
    "core.notes",
    "core.editor",
    "core.hbs",
    "core.agent",
    "core.agent.tools",
    "core.resources",
    "core.pdf",
    "core.models",
    "core.docling",
    "core.platform",
];
const LITERATURE_OPTIONAL: [&str; 2] = ["core.agent.skills", "core.agent.mcp"];

pub(crate) struct StagedHost {
    pub directory: tempfile::TempDir,
    pub executable: PathBuf,
    pub input_directory: Option<tempfile::TempDir>,
    pub profile: NativeProfile,
    #[cfg(unix)]
    _session: std::sync::Arc<super::staging::StagingSession>,
}

pub(super) fn stage(catalog: &RuntimePluginCatalog, key: &str) -> NativeResult<StagedHost> {
    stage_for(catalog, key, NativeProfile::WritingPreview)
}

pub(crate) fn accept(
    catalog: &RuntimePluginCatalog,
    key: &str,
    profile: NativeProfile,
) -> NativeResult<RuntimePluginPackage> {
    let listing = catalog.discover().map_err(|error| {
        NativeHostError::for_profile(profile, error.code, error.message).package(key)
    })?;
    let package = listing
        .packages
        .into_iter()
        .find(|package| package.package_key == key)
        .ok_or_else(|| {
            NativeHostError::for_profile(
                profile,
                "plugin_host_package_unavailable",
                "显式选择的原生插件包不可用。",
            )
            .package(key)
        })?;
    let manifest: PackageManifest = serde_json::from_value(package.manifest.clone())
        .map_err(|_| profile_error(profile).package(key))?;
    validate_package(manifest, profile).map_err(|error| error.package(key))?;
    Ok(package)
}

pub(crate) fn stage_for(
    catalog: &RuntimePluginCatalog,
    key: &str,
    profile: NativeProfile,
) -> NativeResult<StagedHost> {
    let resource = |path: &str| {
        catalog.resource(key, path).map_err(|error| {
            NativeHostError::for_profile(profile, error.code, error.message).package(key)
        })
    };
    let bytes = resource("manifest.json")?.bytes;
    let manifest: PackageManifest =
        serde_json::from_slice(&bytes).map_err(|_| profile_error(profile))?;
    let native = validate_package(manifest, profile)?;
    let executable_bytes = resource(&native.entry)?.bytes;
    staging::stage_bytes(catalog, &executable_bytes, profile)
}

fn version_valid(version: &str) -> bool {
    let (numeric, suffix) = version
        .split_once('-')
        .map_or((version, None), |(numeric, suffix)| (numeric, Some(suffix)));
    numeric.split('.').count() == 3
        && numeric
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
        && suffix.is_none_or(|suffix| {
            !suffix.is_empty()
                && suffix
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b".-".contains(&byte))
        })
}

fn validate_package(
    manifest: PackageManifest,
    profile: NativeProfile,
) -> NativeResult<NativeEntry> {
    match manifest {
        PackageManifest::NativeOnly(manifest) => {
            validate(&manifest, profile)?;
            Ok(manifest.native)
        }
        PackageManifest::Combined(manifest) if profile == NativeProfile::NoteTraceDecision => {
            validate_combined(&manifest)?;
            Ok(manifest.native)
        }
        PackageManifest::Combined(manifest) if profile == NativeProfile::WritingProjects => {
            writing_combined::validate(&manifest)?;
            Ok(manifest.native)
        }
        PackageManifest::Combined(_) => Err(profile_error(profile)),
    }
}

fn validate(manifest: &Manifest, profile: NativeProfile) -> NativeResult<()> {
    let empty = |value: &Requirements| value.required.is_empty() && value.optional.is_empty();
    let capabilities_valid = if profile == NativeProfile::NoteTraceDecision {
        literature_matches(&manifest.provides, false)
    } else if profile == NativeProfile::WritingProjects {
        capabilities_match(
            &manifest.provides,
            &[profile.capability(), "writing.import-preview"],
        )
    } else {
        capabilities_match(&manifest.provides, &[profile.capability()])
    };
    if manifest.format_version != 2
        || manifest.id != profile.plugin_id()
        || manifest.api_version != 1
        || !version_valid(&manifest.version)
        || !empty(&manifest.dependencies)
        || !empty(&manifest.consumes)
        || !capabilities_valid
    {
        return Err(profile_error(profile));
    }
    validate_native(&manifest.native, profile)
}

fn validate_combined(manifest: &CombinedManifest) -> NativeResult<()> {
    let profile = NativeProfile::NoteTraceDecision;
    let capabilities_valid = literature_matches(&manifest.provides, true);
    // Keep the original footprint and its confirmation extension; new writers
    // additionally require the exact diagnostics export. None grants Native authority.
    let mut with_confirmation = LITERATURE_REQUIRED.to_vec();
    with_confirmation.push("core.platform.confirmation");
    let mut with_write_diagnostics = with_confirmation.clone();
    with_write_diagnostics.push("core.structured-resources.write-diagnostics");
    let requirements_valid =
        core_requirements_match(&manifest.consumes.required, &LITERATURE_REQUIRED)
            || core_requirements_match(&manifest.consumes.required, &with_confirmation)
            || core_requirements_match(&manifest.consumes.required, &with_write_diagnostics);
    if manifest.format_version != 2
        || manifest.id != profile.plugin_id()
        || manifest.api_version != 1
        || !version_valid(&manifest.version)
        || !manifest.dependencies.required.is_empty()
        || !manifest.dependencies.optional.is_empty()
        || !capabilities_valid
        || !requirements_valid
        || !core_requirements_match(&manifest.consumes.optional, &LITERATURE_OPTIONAL)
        || manifest.entry != "entry.js"
        || manifest.styles != ["style.css"]
        || manifest.ui_runtime.is_empty()
        || manifest.ui_runtime.trim() != manifest.ui_runtime
    {
        return Err(profile_error(profile));
    }
    // React/UI compatibility is checked by the existing Web SDK before ESM
    // import. These fields never grant a native method or a data resource.
    validate_native(&manifest.native, profile)
}

/// Method capability is explicit; frontend catalog metadata alone grants none.
pub(crate) fn accept_catalog(
    catalog: &RuntimePluginCatalog,
    key: &str,
) -> NativeResult<RuntimePluginPackage> {
    let package = accept(catalog, key, NativeProfile::NoteTraceDecision)?;
    let declared = package
        .manifest
        .get("provides")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|values| {
            values.iter().any(|value| {
                value.get("id").and_then(serde_json::Value::as_str)
                    == Some(research_workbench_native_sdk::catalog::CAPABILITY_ID)
            })
        });
    if !declared {
        return Err(NativeHostError::for_profile(
            NativeProfile::NoteTraceDecision,
            "plugin_host_capability_unavailable",
            "所选文献包未声明完整题录读取方法。",
        )
        .package(key));
    }
    Ok(package)
}

pub(crate) fn accept_catalog_save(
    catalog: &RuntimePluginCatalog,
    key: &str,
) -> NativeResult<RuntimePluginPackage> {
    let package = accept_catalog(catalog, key)?;
    if !package.manifest["provides"]
        .as_array()
        .is_some_and(|values| {
            values.iter().any(|value| {
                value["id"] == research_workbench_native_sdk::catalog_save::CAPABILITY_ID
            })
        })
    {
        return Err(NativeHostError::for_profile(
            NativeProfile::NoteTraceDecision,
            "plugin_host_capability_unavailable",
            "所选文献包未声明题录保存方法。",
        )
        .package(key));
    }
    Ok(package)
}

pub(crate) fn accept_nal_queries(
    catalog: &RuntimePluginCatalog,
    key: &str,
) -> NativeResult<RuntimePluginPackage> {
    let package = accept(catalog, key, NativeProfile::NoteTraceDecision)?;
    if !package.manifest["provides"]
        .as_array()
        .is_some_and(|values| {
            values
                .iter()
                .any(|value| value["id"] == research_workbench_native_sdk::nal_query::CAPABILITY_ID)
        })
    {
        return Err(NativeHostError::for_profile(
            NativeProfile::NoteTraceDecision,
            "plugin_host_capability_unavailable",
            "所选文献包未声明查询集合读取／保存方法。",
        )
        .package(key));
    }
    Ok(package)
}

pub(crate) fn accept_nal_bootstrap(
    catalog: &RuntimePluginCatalog,
    key: &str,
) -> NativeResult<RuntimePluginPackage> {
    let package = accept(catalog, key, NativeProfile::NoteTraceDecision)?;
    if !package.manifest["provides"]
        .as_array()
        .is_some_and(|values| {
            values.iter().any(|value| {
                value["id"] == research_workbench_native_sdk::nal_bootstrap::CAPABILITY_ID
            })
        })
    {
        return Err(NativeHostError::for_profile(
            NativeProfile::NoteTraceDecision,
            "plugin_host_capability_unavailable",
            "所选文献包未声明关系网络启动／读取方法。",
        )
        .package(key));
    }
    Ok(package)
}

pub(crate) fn accept_nal_annotation(
    catalog: &RuntimePluginCatalog,
    key: &str,
) -> NativeResult<RuntimePluginPackage> {
    let package = accept(catalog, key, NativeProfile::NoteTraceDecision)?;
    if !package.manifest["provides"]
        .as_array()
        .is_some_and(|values| {
            values.iter().any(|value| {
                value["id"] == research_workbench_native_sdk::nal_annotation_prepare::CAPABILITY_ID
            })
        })
    {
        return Err(NativeHostError::for_profile(
            NativeProfile::NoteTraceDecision,
            "plugin_host_capability_unavailable",
            "所选文献包未声明单份批注笔记方法。",
        )
        .package(key));
    }
    Ok(package)
}

pub(crate) fn accept_nal_mutation(
    catalog: &RuntimePluginCatalog,
    key: &str,
) -> NativeResult<RuntimePluginPackage> {
    let package = accept(catalog, key, NativeProfile::NoteTraceDecision)?;
    if !package.manifest["provides"]
        .as_array()
        .is_some_and(|values| {
            values.iter().any(|value| {
                value["id"] == research_workbench_native_sdk::nal_mutation::CAPABILITY_ID
            })
        })
    {
        return Err(NativeHostError::for_profile(
            NativeProfile::NoteTraceDecision,
            "plugin_host_capability_unavailable",
            "所选文献包未声明关系网络修改方法。",
        )
        .package(key));
    }
    Ok(package)
}

fn core_requirements_match(actual: &[CoreRequirement], expected: &[&str]) -> bool {
    actual.len() == expected.len()
        && actual.iter().all(|requirement| {
            requirement.version == 1
                && requirement.provider == "core"
                && expected.contains(&requirement.id.as_str())
        })
        && actual
            .iter()
            .map(|requirement| &requirement.id)
            .collect::<BTreeSet<_>>()
            .len()
            == expected.len()
}

fn validate_native(native: &NativeEntry, profile: NativeProfile) -> NativeResult<()> {
    if native.protocol_version != profile.protocol_version() {
        return Err(NativeHostError::for_profile(
            profile,
            "plugin_host_protocol_incompatible",
            "原生插件协议版本不兼容。",
        ));
    }
    if !cfg!(unix) || native.os != std::env::consts::OS || native.arch != std::env::consts::ARCH {
        return Err(NativeHostError::for_profile(
            profile,
            "plugin_host_platform_unsupported",
            "原生插件包与当前操作系统／架构不兼容。",
        ));
    }
    Ok(())
}

fn profile_error(profile: NativeProfile) -> Box<NativePreviewError> {
    NativeHostError::for_profile(
        profile,
        "plugin_host_profile_unsupported",
        match profile {
            NativeProfile::WritingPreview => {
                "原生试点包必须只提供 writing.import-preview v1，且无其他依赖或入口。"
            }
            NativeProfile::WritingProjects => {
                "写作工程入口仅支持 writing.project-files v1 的已知独立包或组合包，且无额外执行权限。"
            }
            NativeProfile::NoteTraceDecision => {
                "文献原生入口仅支持 literature.note-trace-decision v1 的独立包或已知组合包，且无额外执行权限。"
            }
        },
    )
}

#[cfg(test)]
pub(crate) mod tests;

#[cfg(test)]
mod combined_tests;

#[cfg(test)]
mod catalog_tests;

#[cfg(test)]
mod nal_annotation_tests;
#[cfg(test)]
mod nal_bootstrap_tests;
#[cfg(test)]
mod nal_mutation_tests;

mod duplicate_review;
pub(crate) use duplicate_review::accept_duplicate_review;
mod duplicate_merge;
#[cfg(test)]
mod duplicate_review_tests;
pub(crate) use duplicate_merge::accept_duplicate_merge;
mod literature_lifecycle;
pub(crate) use literature_lifecycle::accept_literature_lifecycle;
mod trash_cleanup;
pub(crate) use trash_cleanup::accept_trash_cleanup;
mod metadata;
pub(crate) use metadata::accept_metadata;
mod import_jobs;
pub(crate) use import_jobs::{accept_bibtex, accept_import_jobs};
mod doi;
pub(crate) use doi::accept_doi;
#[cfg(test)]
mod duplicate_merge_tests;
mod duplicate_undo;
#[cfg(test)]
mod import_jobs_tests;
#[cfg(test)]
mod literature_lifecycle_tests;
#[cfg(test)]
mod metadata_tests;
#[cfg(test)]
mod trash_cleanup_tests;
pub(crate) use duplicate_undo::accept_duplicate_undo;
#[cfg(test)]
mod duplicate_undo_tests;
#[cfg(test)]
mod nal_query_tests;

#[cfg(test)]
mod writing_combined_tests;
