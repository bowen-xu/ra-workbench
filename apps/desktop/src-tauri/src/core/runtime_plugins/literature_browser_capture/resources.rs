//! Original fixed browser-capture SQL, accepted from the selected package snapshot.
use crate::runtime_plugins::{
    RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data,
};
use std::collections::HashMap;
const ASSETS: &[(&str, &str, &str)] = &[
    (
        "session-receipt",
        "browser-capture-session-receipt.v1.sql",
        "09f6ea20a5796a5591a92b2ff3f875b5d3ccfd2598fbf76487a742068744f92d",
    ),
    (
        "session-insert",
        "browser-capture-session-insert.v1.sql",
        "8d642d32ece5641aae273f4cb0fb93efd145d4ce0828f60cdc539b65bf1405c0",
    ),
    (
        "session-finish",
        "browser-capture-session-finish.v1.sql",
        "361c41c18f2a31b8386dc90cbb01da6f507ddd970b813225705eb1b8d21bbbcc",
    ),
    (
        "status-receipt",
        "browser-capture-status-receipt.v1.sql",
        "03ed47b47f0993e7731fd1cd7f0cf59c6e6dc6397c933c17dfb5204b3d2ac0f2",
    ),
    (
        "status-attachments",
        "browser-capture-status-attachments.v1.sql",
        "302a47188fa11c750b3997536ab8a269a65f9a1a68a3fcc59cd1cf703c33de91",
    ),
    (
        "item-insert",
        "browser-capture-item-insert.v1.sql",
        "a2068338d31edc3afa3bac513fc4d1b0e684cba9cb3837e97c7e821ca3dc59f3",
    ),
    (
        "provenance-source",
        "browser-capture-provenance-source.v1.sql",
        "f413b1385409ed1b622c62e87b3fbee25896c2dc8bdd7c8983eed2970aa22185",
    ),
    (
        "identifier-insert",
        "browser-capture-identifier-insert.v1.sql",
        "be319b32e54132c5ff0ecc0a8c320b472cf4a613b1ffa64cf7ff9dcdf3fb6a92",
    ),
    (
        "active-parent",
        "browser-capture-active-parent.v1.sql",
        "725afd4c968a9aecb7afcb3ba63bd484834e653e5c313b7a7061271bb0da7726",
    ),
    (
        "attachment-receipt",
        "browser-capture-attachment-receipt.v1.sql",
        "a4e9d1c23769716e7b8e91857364396a48b203365775051278b157969be3f45e",
    ),
    (
        "attachment-label",
        "browser-capture-attachment-label.v1.sql",
        "ac9b68d1bfafdb95b24d210d6b8f5e4c5480ff701671cce9bab73971ba111b55",
    ),
    (
        "attachment-insert",
        "browser-capture-attachment-insert.v1.sql",
        "2a09ba9e98e32ac90787a3ceccec165dacb84d5bfc9e99b8d954f9cbbb4417f6",
    ),
];
#[derive(Clone)]
pub(crate) struct CaptureResources {
    statements: HashMap<&'static str, String>,
}
impl CaptureResources {
    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        let statements = ASSETS
            .iter()
            .map(|&(key, file, sha)| {
                data::load_text(
                    catalog,
                    package,
                    "literature_library",
                    &format!("data/{file}"),
                    sha,
                )
                .map(|sql| (key, sql))
            })
            .collect::<Result<_, _>>()?;
        Ok(Self { statements })
    }
    pub(super) fn sql(&self, key: &str) -> &str {
        // Every key is a source-controlled literal; load captures every required statement.
        self.statements
            .get(key)
            .expect("fixed browser capture statement")
    }
}
