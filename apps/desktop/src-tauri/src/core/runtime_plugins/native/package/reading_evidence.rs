use super::*;
use crate::runtime_plugins::reading_evidence::CAPABILITY_ID;

pub(crate) fn accept_reading_evidence(
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
            "所选文献包未声明显式冻结来源读取方法。",
        )
        .package(key));
    }
    Ok(package)
}

pub(crate) fn accept_hbs_validation(
    catalog: &RuntimePluginCatalog,
    key: &str,
) -> NativeResult<RuntimePluginPackage> {
    let package = accept(catalog, key, NativeProfile::NoteTraceDecision)?;
    for capability in [
        research_workbench_native_sdk::structured_validation::CAPABILITY_ID,
        CAPABILITY_ID,
    ] {
        if !package.manifest["provides"]
            .as_array()
            .is_some_and(|values| {
                values
                    .iter()
                    .any(|value| value["id"] == capability && value["version"] == 1)
            })
        {
            return Err(NativeHostError::for_profile(
                NativeProfile::NoteTraceDecision,
                "plugin_host_capability_unavailable",
                "所选文献包未声明结构化校验或其冻结来源读取能力。",
            )
            .package(key));
        }
    }
    Ok(package)
}

pub(crate) fn accept_unreviewed_selection(
    catalog: &RuntimePluginCatalog,
    key: &str,
) -> NativeResult<RuntimePluginPackage> {
    let package = accept(catalog, key, NativeProfile::NoteTraceDecision)?;
    if !package.manifest["provides"]
        .as_array()
        .is_some_and(|values| {
            values.iter().any(|value| {
                value["id"] == research_workbench_native_sdk::review_selection::CAPABILITY_ID
                    && value["version"] == 1
            })
        })
    {
        return Err(NativeHostError::for_profile(
            NativeProfile::NoteTraceDecision,
            "plugin_host_capability_unavailable",
            "所选文献包未声明未校验范围的纯选择能力。",
        )
        .package(key));
    }
    Ok(package)
}

pub(crate) fn accept_editable_template(
    catalog: &RuntimePluginCatalog,
    key: &str,
) -> NativeResult<RuntimePluginPackage> {
    let package = accept(catalog, key, NativeProfile::NoteTraceDecision)?;
    if !package.manifest["provides"]
        .as_array()
        .is_some_and(|values| {
            values.iter().any(|value| {
                value["id"] == research_workbench_native_sdk::editable_template::CAPABILITY_ID
                    && value["version"]
                        == research_workbench_native_sdk::editable_template::CAPABILITY_VERSION
            })
        })
    {
        return Err(NativeHostError::for_profile(
            NativeProfile::NoteTraceDecision,
            "plugin_host_capability_unavailable",
            "所选文献包未声明可编辑阅读模板的纯校验能力。",
        )
        .package(key));
    }
    Ok(package)
}

#[cfg(test)]
mod tests;

pub(crate) fn accept_paper_context(
    catalog: &RuntimePluginCatalog,
    key: &str,
) -> NativeResult<RuntimePluginPackage> {
    use research_workbench_native_sdk::paper_context_wire::{CAPABILITY_ID, CAPABILITY_VERSION};
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
            "所选文献包未声明论文上下文的纯准备能力。",
        )
        .package(key));
    }
    Ok(package)
}

/// Existing pure preparation does not imply the new ordered preview profile.
pub(crate) fn accept_paper_context_preview(
    catalog: &RuntimePluginCatalog,
    key: &str,
) -> NativeResult<RuntimePluginPackage> {
    use research_workbench_native_sdk::paper_context_preview::{CAPABILITY_ID, CAPABILITY_VERSION};
    let package = accept_paper_context(catalog, key)?;
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
            "所选文献包未声明论文上下文的有序预览能力。",
        )
        .package(key));
    }
    Ok(package)
}

/// The original build capability alone does not imply complete preparation policy.
pub(crate) fn accept_paper_context_prepare(
    catalog: &RuntimePluginCatalog,
    key: &str,
) -> NativeResult<RuntimePluginPackage> {
    use research_workbench_native_sdk::paper_context_prepare::{CAPABILITY_ID, CAPABILITY_VERSION};
    let package = accept_paper_context(catalog, key)?;
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
            "所选文献包未声明阅读资料与截图的准备能力。",
        )
        .package(key));
    }
    Ok(package)
}

/// Paper trace policy is explicit; ordinary Note scope remains the original None.
pub(crate) fn accept_paper_retrieval_trace(
    catalog: &RuntimePluginCatalog,
    key: &str,
) -> NativeResult<RuntimePluginPackage> {
    use research_workbench_native_sdk::paper_retrieval::{CAPABILITY_ID, CAPABILITY_VERSION};
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
            "所选文献包未声明论文范围检索预览的纯校验能力。",
        )
        .package(key));
    }
    Ok(package)
}

pub(crate) fn accept_concept_retrieval(
    catalog: &RuntimePluginCatalog,
    key: &str,
) -> NativeResult<RuntimePluginPackage> {
    use research_workbench_native_sdk::concept_retrieval::{CAPABILITY_ID, CAPABILITY_VERSION};
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
            "所选文献包未声明概念检索预览的有限规则。",
        )
        .package(key));
    }
    Ok(package)
}

pub(crate) fn accept_semantic_index(
    catalog: &RuntimePluginCatalog,
    key: &str,
) -> NativeResult<RuntimePluginPackage> {
    use research_workbench_native_sdk::semantic_index::rules::{CAPABILITY_ID, CAPABILITY_VERSION};
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
            "所选文献包未声明语义索引的有限规则。",
        )
        .package(key));
    }
    Ok(package)
}
