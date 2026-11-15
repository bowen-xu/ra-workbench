//! Existing attachment and PDF package admission; the parent retains its named entries.
use super::{
    NativeHostError, NativeProfile, NativeResult, RuntimePluginCatalog, RuntimePluginPackage,
    accept,
};

pub(crate) fn accept_attachments(
    catalog: &RuntimePluginCatalog,
    key: &str,
) -> NativeResult<RuntimePluginPackage> {
    let package = accept(catalog, key, NativeProfile::NoteTraceDecision)?;
    if !package.manifest["provides"]
        .as_array()
        .is_some_and(|values| {
            values
                .iter()
                .any(|value| value["id"] == "literature.attachments" && value["version"] == 1)
        })
    {
        return Err(NativeHostError::for_profile(
            NativeProfile::NoteTraceDecision,
            "plugin_host_capability_unavailable",
            "所选文献包未声明附件及阅读状态方法。",
        )
        .package(key));
    }
    Ok(package)
}

pub(crate) fn accept_pdf_import(
    catalog: &RuntimePluginCatalog,
    key: &str,
) -> NativeResult<RuntimePluginPackage> {
    let package = accept_attachments(catalog, key)?;
    let provides = package.manifest["provides"].as_array();
    for id in [
        "literature.pdf-import",
        "literature.import-jobs",
        "literature.catalog-read",
        "literature.catalog-save",
    ] {
        if !provides.is_some_and(|values| {
            values
                .iter()
                .any(|value| value["id"] == id && value["version"] == 1)
        }) {
            return Err(NativeHostError::for_profile(
                NativeProfile::NoteTraceDecision,
                "plugin_host_capability_unavailable",
                format!("所选文献包未声明PDF导入依赖：{id}。"),
            )
            .package(key));
        }
    }
    Ok(package)
}
