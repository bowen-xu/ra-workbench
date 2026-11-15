use super::*;
use research_workbench_native_sdk::browser_capture::{CAPABILITY_ID, CAPABILITY_VERSION};

pub(crate) fn accept_browser_capture(
    catalog: &RuntimePluginCatalog,
    key: &str,
) -> NativeResult<RuntimePluginPackage> {
    let package = accept(catalog, key, NativeProfile::NoteTraceDecision)?;
    if !package.manifest["provides"]
        .as_array()
        .is_some_and(|values| {
            values
                .iter()
                .any(|value| value["id"] == CAPABILITY_ID && value["version"] == CAPABILITY_VERSION)
        })
    {
        return Err(NativeHostError::for_profile(
            NativeProfile::NoteTraceDecision,
            "plugin_host_capability_unavailable",
            "所选文献包未声明原浏览器采集能力。",
        )
        .package(key));
    }
    Ok(package)
}
