use super::*;

pub(crate) fn accept_import_jobs(
    catalog: &RuntimePluginCatalog,
    key: &str,
) -> NativeResult<RuntimePluginPackage> {
    let package = accept(catalog, key, NativeProfile::NoteTraceDecision)?;
    require(
        &package,
        key,
        &[research_workbench_native_sdk::import_jobs::CAPABILITY_ID],
    )?;
    Ok(package)
}

pub(crate) fn accept_bibtex(
    catalog: &RuntimePluginCatalog,
    key: &str,
) -> NativeResult<RuntimePluginPackage> {
    let package = accept_import_jobs(catalog, key)?;
    require(
        &package,
        key,
        &[
            research_workbench_native_sdk::catalog::CAPABILITY_ID,
            research_workbench_native_sdk::catalog_save::CAPABILITY_ID,
        ],
    )?;
    Ok(package)
}

fn require(package: &RuntimePluginPackage, key: &str, capabilities: &[&str]) -> NativeResult<()> {
    for capability in capabilities {
        if !package.manifest["provides"]
            .as_array()
            .is_some_and(|values| values.iter().any(|value| value["id"] == *capability))
        {
            return Err(NativeHostError::for_profile(
                NativeProfile::NoteTraceDecision,
                "plugin_host_capability_unavailable",
                "所选文献包未声明本次导入任务所需的领域方法。",
            )
            .package(key));
        }
    }
    Ok(())
}
