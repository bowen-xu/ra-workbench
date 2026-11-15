//! Exact capability gate for the four original confirmation operations.
use super::*;
use research_workbench_native_sdk::paper_confirmation::wire::{CAPABILITY_ID, CAPABILITY_VERSION};

pub(crate) fn accept_paper_confirmation(
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
            "所选文献包未声明原字段与文献批次确认能力。",
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
    fn paper_confirmation_requires_its_exact_declared_capability() {
        let (root, catalog, _) = super::super::tests::literature_fixture(b"host");
        let file = root.path().join("writing/manifest.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        let key = catalog.discover().unwrap().packages[0].package_key.clone();
        assert!(accept_paper_confirmation(&catalog, &key).is_err());
        manifest["provides"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id": CAPABILITY_ID, "version": CAPABILITY_VERSION}));
        std::fs::write(&file, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let key = catalog.discover().unwrap().packages[0].package_key.clone();
        accept_paper_confirmation(&catalog, &key).unwrap();
        manifest["provides"]
            .as_array_mut()
            .unwrap()
            .last_mut()
            .unwrap()["version"] = json!(2);
        std::fs::write(&file, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let listing = catalog.discover().unwrap();
        if let Some(package) = listing.packages.first() {
            assert!(accept_paper_confirmation(&catalog, &package.package_key).is_err());
        }
    }
}
