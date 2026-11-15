use super::*;
use research_workbench_native_sdk::reading_units::CAPABILITY_ID;

pub(crate) fn accept_reading_units(
    catalog: &RuntimePluginCatalog,
    key: &str,
) -> NativeResult<RuntimePluginPackage> {
    let package = accept(catalog, key, NativeProfile::NoteTraceDecision)?;
    if !package.manifest["provides"]
        .as_array()
        .is_some_and(|values| {
            values
                .iter()
                .any(|value| value["id"] == CAPABILITY_ID && value["version"] == 1)
        })
    {
        return Err(NativeHostError::for_profile(
            NativeProfile::NoteTraceDecision,
            "plugin_host_capability_unavailable",
            "所选文献包未声明阅读单元列表／保存方法。",
        )
        .package(key));
    }
    Ok(package)
}

#[cfg(test)]
mod tests;
