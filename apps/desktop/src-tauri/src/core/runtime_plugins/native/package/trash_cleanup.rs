//! Expiry cleanup uses the same closed catalog host and a separate declaration.
use super::*;

pub(crate) fn accept_trash_cleanup(
    catalog: &RuntimePluginCatalog,
    key: &str,
) -> NativeResult<RuntimePluginPackage> {
    let package = accept(catalog, key, NativeProfile::NoteTraceDecision)?;
    if !package.manifest["provides"]
        .as_array()
        .is_some_and(|values| {
            values.iter().any(|value| {
                value["id"] == research_workbench_native_sdk::trash_cleanup::CAPABILITY_ID
            })
        })
    {
        return Err(NativeHostError::for_profile(
            NativeProfile::NoteTraceDecision,
            "plugin_host_capability_unavailable",
            "所选文献包未声明回收站到期清理方法。",
        )
        .package(key));
    }
    Ok(package)
}
