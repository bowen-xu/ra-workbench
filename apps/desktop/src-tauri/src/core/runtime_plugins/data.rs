//! Immutable SQL bytes accepted against a Core-owned metadata grant.
//! This layer grants no typed resource scope and never executes SQL or a host.
use super::{RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage};
use research_workbench_native_sdk::data::DataResourceGrant;
use sha2::{Digest, Sha256};

/// Constructed only after the catalog's package/resource snapshot checks.
/// No Debug/Serialize implementation exposes the accepted SQL bytes.
#[derive(Clone)]
pub(crate) struct AcceptedDataResource {
    grant: DataResourceGrant,
    sql: String,
    package_key: String,
    owner_id: String,
}

impl AcceptedDataResource {
    /// `grant` is explicit Core authorization, never a native request or a
    /// permission inferred from package metadata. SQL remains a package resource.
    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
        owner_id: &str,
        path: &str,
        grant: DataResourceGrant,
    ) -> Result<Self, RuntimePluginError> {
        let error = |code: &'static str, message: String| {
            RuntimePluginError::new(code, message)
                .package(&package.package_key)
                .resource(path)
        };
        grant.validate().map_err(|cause| {
            error(
                "runtime_plugin_data_grant_invalid",
                format!("SQL资源的Core授权元数据无效：{cause}。"),
            )
        })?;
        let sql = load_text(catalog, package, owner_id, path, &grant.sha256)?;
        Ok(Self {
            grant,
            sql,
            package_key: package.package_key.clone(),
            owner_id: owner_id.to_owned(),
        })
    }

    pub(crate) fn grant(&self) -> &DataResourceGrant {
        &self.grant
    }

    pub(crate) fn sql(&self) -> &str {
        &self.sql
    }

    pub(crate) fn package_key(&self) -> &str {
        &self.package_key
    }

    pub(crate) fn owner_id(&self) -> &str {
        &self.owner_id
    }
}

/// Fixed package text for reviewed statement families with numeric-only fragments.
/// This does not authorize execution or accept text from a native request.
pub(super) fn load_text(
    catalog: &RuntimePluginCatalog,
    package: &RuntimePluginPackage,
    owner_id: &str,
    path: &str,
    sha256: &str,
) -> Result<String, RuntimePluginError> {
    let error = |code: &'static str, message: String| {
        RuntimePluginError::new(code, message)
            .package(&package.package_key)
            .resource(path)
    };
    if package.manifest.get("id").and_then(|id| id.as_str()) != Some(owner_id) {
        return Err(error(
            "runtime_plugin_data_owner_mismatch",
            "SQL资源的插件包拥有者与Core授权不符。".into(),
        ));
    }
    let resource = |relative| {
        catalog
            .resource(&package.package_key, relative)
            .map_err(|failure| failure.package(&package.package_key).resource(relative))
    };
    let manifest = resource("manifest.json")?;
    let manifest: serde_json::Value = serde_json::from_slice(&manifest.bytes).map_err(|_| {
        RuntimePluginError::new(
            "runtime_plugin_manifest_invalid",
            "SQL资源所属插件包的manifest不是有效JSON。",
        )
        .package(&package.package_key)
        .resource("manifest.json")
    })?;
    if manifest != package.manifest {
        return Err(error(
            "runtime_plugin_data_manifest_mismatch",
            "SQL资源的插件包声明与已发现的内容快照不符。".into(),
        ));
    }
    let bytes = resource(path)?.bytes;
    if format!("{:x}", Sha256::digest(&bytes)) != sha256 {
        return Err(error(
            "runtime_plugin_data_digest_mismatch",
            "SQL资源字节与Core授权的固定摘要不符。".into(),
        ));
    }
    let sql = String::from_utf8(bytes).map_err(|_| {
        error(
            "runtime_plugin_data_sql_invalid",
            "SQL资源必须是非空且不含NUL的UTF-8文本。".into(),
        )
    })?;
    if sql.trim().is_empty() || sql.contains('\0') {
        return Err(error(
            "runtime_plugin_data_sql_invalid",
            "SQL资源必须是非空且不含NUL的UTF-8文本。".into(),
        ));
    }
    Ok(sql)
}

#[cfg(test)]
mod tests;
