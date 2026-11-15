//! Closed footprint of the existing writing frontend plus its finite v2 host.
use super::*;

pub(super) const REQUIRED: [&str; 15] = [
    "core.workbench",
    "core.workbench.writing-frontend",
    "core.workbench.writing-runtime",
    "core.notes",
    "core.editor",
    "core.hbs",
    "core.agent",
    "core.agent.tools",
    "core.agent.writing-dispatch-admission",
    "core.agent.writing-compaction-retirement",
    "core.resources",
    "core.pdf",
    "core.models",
    "core.git",
    "core.platform",
];

pub(super) fn validate(manifest: &CombinedManifest) -> NativeResult<()> {
    let profile = NativeProfile::WritingProjects;
    let optional_dependency = serde_json::json!({"id":"literature_library", "apiVersion":1});
    let optional = [
        ("literature.citations", "literature_library"),
        ("core.agent.skills", "core"),
        ("core.agent.mcp", "core"),
    ];
    let optional_valid = manifest.consumes.optional.len() == optional.len()
        && optional.iter().all(|(id, provider)| {
            manifest
                .consumes
                .optional
                .iter()
                .filter(|actual| {
                    actual.id == *id && actual.provider == *provider && actual.version == 1
                })
                .count()
                == 1
        });
    if manifest.format_version != 2
        || manifest.id != profile.plugin_id()
        || manifest.api_version != 1
        || !version_valid(&manifest.version)
        || !manifest.dependencies.required.is_empty()
        || manifest.dependencies.optional != [optional_dependency]
        || !capabilities_match(
            &manifest.provides,
            &[
                "writing.projects",
                "writing.note-catalog",
                "writing.project-files",
                "writing.import-preview",
            ],
        )
        || !core_requirements_match(&manifest.consumes.required, &REQUIRED)
        || !optional_valid
        || manifest.entry != "entry.js"
        || manifest.styles != ["style.css"]
        || manifest.ui_runtime.is_empty()
        || manifest.ui_runtime.trim() != manifest.ui_runtime
    {
        return Err(profile_error(profile));
    }
    // Web compatibility remains with the SDK. These declarations do not grant
    // a Native method, generic SQL execution or a second domain provider.
    validate_native(&manifest.native, profile)
}
