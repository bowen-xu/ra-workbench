//! Finite original compile statements admitted from the selected writing package.
use crate::runtime_plugins::{
    RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data,
};

pub(crate) const ASSETS: &[(&str, &str)] = &[
    (
        "project-compile-cache-protected.v1.sql",
        "91e8b1286ddb71c4796148f0ffeb8e8ce2582bc85e1cb044b8af56d9c4a697a5",
    ),
    (
        "project-compile-root.v1.sql",
        "aab6db3e1c40dea6ca74eac8f8b35e4937e6631a7f2203bbc90863898bbb056d",
    ),
    (
        "project-compile-create.v1.sql",
        "840e7bbed7ab902a0eedf6d878acd8b46513386edbb78278052093bed823f42f",
    ),
    (
        "project-compile-running.v1.sql",
        "e08f5c845b1d6d226147b28a42b90ece262a26c303122f11a04c2505da68c63f",
    ),
    (
        "project-compile-cancel.v1.sql",
        "4f2f0f244404a16d840b105d62610e7acfa917401e321dbcab2985faacb9f361",
    ),
    (
        "project-compile-finish.v1.sql",
        "3bc8dad8b8f13fe6e3088cc9988597e78c7cc8801e1f23480bb17e9335849787",
    ),
    (
        "project-compile-diagnostic-insert.v1.sql",
        "7aa763659e16972b2150ce180f74f18ccab404e5b272d859b35d838a068cf39a",
    ),
    (
        "project-compile-object-insert.v1.sql",
        "f8d4d186bcfc2dab4d3c9fa9370eadf8da3389060e6090f13afff8288e2ccd87",
    ),
    (
        "project-compile-last-success.v1.sql",
        "419112cfc821bdcac2ad5cf0576b3bc0b83d7dd21cc28919953822dfd9ec7fde",
    ),
    (
        "project-compile-fail.v1.sql",
        "65ccac55c2ec656345f4d88fe5d417d13ac35a3eb2b3894a47ad3200033d6187",
    ),
    (
        "project-compile-get.v1.sql",
        "16d42459b8becf8a5b2fc1124040b2473d2473b0261dabe3839084f6d8602642",
    ),
    (
        "project-compile-latest.v1.sql",
        "0c3e2a74d24c3262e438cdb7a0f148cf63c595347eeae9b7d71108a96db8fef7",
    ),
    (
        "project-compile-latest-pdf.v1.sql",
        "faca8f99cbba2bde29cc33dd2ee87621405e136974c96c4a9d3cc565af98f59e",
    ),
    (
        "project-compile-pdf-path.v1.sql",
        "c2e5d8ebe6b95584e717bc59b5c5058d0b23e8ac5022b877da3d2aa72bbe6477",
    ),
    (
        "project-compile-synctex-context.v1.sql",
        "2c6d850b49805cc2150e0144adbfb4ea7b15f6613cc639ad22ccda823d353e9b",
    ),
    (
        "project-compile-input-context.v1.sql",
        "874045ddad3a1a3f008c8d65f6d8ab6a1a7ef5629e74620c65ca8b0f389604ee",
    ),
    (
        "project-compile-artifact-insert.v1.sql",
        "476f7c1ec0c0dc5f2bc47455d45055b16bcf69e4988616190ea296cacf7d8b83",
    ),
    (
        "project-compile-diagnostics.v1.sql",
        "909cb4332e5c0f5a437556bf65fd008708ba3674a37a5a51c081bc65885080d8",
    ),
    (
        "project-compile-artifacts.v1.sql",
        "97fa80f822bed01a6bfc49249d6909c9de5f11762a04289feef6b2d438ba640f",
    ),
];

pub(crate) struct Sql {
    statements: Vec<String>,
}
impl Sql {
    pub(super) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        let statements = ASSETS
            .iter()
            .map(|(name, hash)| {
                data::load_text(
                    catalog,
                    package,
                    "writing_projects",
                    &format!("data/{name}"),
                    hash,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { statements })
    }
    pub(super) fn get(&self, name: &str) -> &str {
        let index = ASSETS
            .iter()
            .position(|(expected, _)| *expected == name)
            .expect("finite reviewed compile statement");
        self.statements[index].trim_end_matches('\n')
    }
}
