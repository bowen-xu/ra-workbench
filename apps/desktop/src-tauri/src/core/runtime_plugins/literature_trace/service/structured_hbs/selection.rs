use super::*;
use crate::runtime_plugins::literature_hbs::HbsReadingEvidenceResources;

pub(super) fn select(
    catalog: &RuntimePluginCatalog,
    explicit: Option<&str>,
) -> NativeResult<HbsReadingEvidenceResources> {
    let key = selected_key(catalog, explicit)?;
    let package = package::accept_hbs_validation(catalog, &key)?;
    HbsReadingEvidenceResources::load(catalog, &package)
        .map_err(|error| LiteratureTraceService::error(error.code, error.message).package(&key))
}

pub(super) fn select_unreviewed(
    catalog: &RuntimePluginCatalog,
    explicit: Option<&str>,
) -> NativeResult<String> {
    let key = selected_key(catalog, explicit)?;
    package::accept_unreviewed_selection(catalog, &key)?;
    Ok(key)
}

fn selected_key(catalog: &RuntimePluginCatalog, explicit: Option<&str>) -> NativeResult<String> {
    let key = if let Some(key) = explicit {
        key.to_owned()
    } else {
        let listing = catalog
            .discover()
            .map_err(|error| LiteratureTraceService::error(error.code, error.message))?;
        let providers = listing
            .packages
            .iter()
            .filter(|package| package.manifest["id"] == "literature_library")
            .collect::<Vec<_>>();
        match providers.as_slice() {
            [package] => package.package_key.clone(),
            [] => {
                return Err(LiteratureTraceService::error(
                    "plugin_host_capability_unavailable",
                    "结构化校验的文献提供者不可用。",
                ));
            }
            _ => {
                return Err(LiteratureTraceService::error(
                    "plugin_host_package_ambiguous",
                    "受控目录存在多个文献包，结构化校验未选择提供者。",
                ));
            }
        }
    };
    Ok(key)
}

pub(super) fn select_editable_template(
    catalog: &RuntimePluginCatalog,
    explicit: Option<&str>,
) -> NativeResult<String> {
    let key = selected_key(catalog, explicit)?;
    package::accept_editable_template(catalog, &key)?;
    Ok(key)
}
