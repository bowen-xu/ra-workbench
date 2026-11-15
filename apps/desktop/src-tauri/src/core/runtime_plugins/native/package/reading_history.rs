use super::*;
use research_workbench_native_sdk::reading_history::{CAPABILITY_ID, CAPABILITY_VERSION};

pub(crate) fn accept_reading_history(
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
            "所选文献包未声明旧阅读历史兼容能力。",
        )
        .package(key));
    }
    Ok(package)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn reading_history_requires_original_foundations_and_exact_version() {
        let (root, catalog, _) = super::super::tests::literature_fixture(b"host");
        let file = root.path().join("writing/manifest.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        for id in [
            "literature.paper-context",
            "literature.paper-context-prepare",
            "literature.analysis-history",
            "literature.reading-history",
        ] {
            manifest["provides"]
                .as_array_mut()
                .unwrap()
                .push(json!({"id":id,"version":1}));
        }
        std::fs::write(&file, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let key = catalog.discover().unwrap().packages[0].package_key.clone();
        accept_reading_history(&catalog, &key).unwrap();
        for id in [
            "literature.paper-context",
            "literature.paper-context-prepare",
            "literature.analysis-history",
            "literature.reading-history",
        ] {
            let mut missing = manifest.clone();
            missing["provides"]
                .as_array_mut()
                .unwrap()
                .retain(|value| value["id"] != id);
            std::fs::write(&file, serde_json::to_vec(&missing).unwrap()).unwrap();
            let key = catalog.discover().unwrap().packages[0].package_key.clone();
            assert!(accept_reading_history(&catalog, &key).is_err());
        }
        let mut version = manifest;
        version["provides"]
            .as_array_mut()
            .unwrap()
            .last_mut()
            .unwrap()["version"] = json!(2);
        std::fs::write(&file, serde_json::to_vec(&version).unwrap()).unwrap();
        let listing = catalog.discover().unwrap();
        if let Some(package) = listing.packages.first() {
            assert!(accept_reading_history(&catalog, &package.package_key).is_err());
        }
    }
}
