use super::*;

pub(crate) fn accept_duplicate_review(
    catalog: &RuntimePluginCatalog,
    key: &str,
) -> NativeResult<RuntimePluginPackage> {
    let package = accept(catalog, key, NativeProfile::NoteTraceDecision)?;
    if !package.manifest["provides"]
        .as_array()
        .is_some_and(|values| {
            values.iter().any(|value| {
                value["id"] == research_workbench_native_sdk::duplicate_review::CAPABILITY_ID
            })
        })
    {
        return Err(NativeHostError::for_profile(
            NativeProfile::NoteTraceDecision,
            "plugin_host_capability_unavailable",
            "所选文献包未声明重复候选审核／历史方法。",
        )
        .package(key));
    }
    Ok(package)
}
