use super::*;

pub(crate) fn accept_metadata(
    catalog: &RuntimePluginCatalog,
    key: &str,
) -> NativeResult<RuntimePluginPackage> {
    let package = accept(catalog, key, NativeProfile::NoteTraceDecision)?;
    let declared = |id: &str| {
        package.manifest["provides"]
            .as_array()
            .is_some_and(|values| values.iter().any(|value| value["id"] == id))
    };
    // Every operation may return the original catalog projection; full apply
    // additionally consumes the already reviewed save statements from this package.
    for id in [
        research_workbench_native_sdk::metadata::CAPABILITY_ID,
        research_workbench_native_sdk::catalog::CAPABILITY_ID,
        research_workbench_native_sdk::catalog_save::CAPABILITY_ID,
    ] {
        if !declared(id) {
            return Err(NativeHostError::for_profile(
                NativeProfile::NoteTraceDecision,
                "plugin_host_capability_unavailable",
                "所选文献包未声明完整的元数据准备、题录投影和保存方法。",
            )
            .package(key));
        }
    }
    Ok(package)
}
