use super::*;

pub(crate) fn accept_doi(
    catalog: &RuntimePluginCatalog,
    key: &str,
) -> NativeResult<RuntimePluginPackage> {
    let package = accept_import_jobs(catalog, key)?;
    for id in [
        research_workbench_native_sdk::doi_metadata::CAPABILITY_ID,
        research_workbench_native_sdk::catalog::CAPABILITY_ID,
    ] {
        if !package.manifest["provides"]
            .as_array()
            .is_some_and(|values| values.iter().any(|value| value["id"] == id))
        {
            return Err(NativeHostError::for_profile(
                NativeProfile::NoteTraceDecision,
                "plugin_host_capability_unavailable",
                "所选文献包未声明本次DOI预览所需的领域能力。",
            )
            .package(key));
        }
    }
    Ok(package)
}
