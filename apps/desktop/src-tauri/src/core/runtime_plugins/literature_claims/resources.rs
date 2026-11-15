//! Exact original SQL bytes; Core selects operations, binds values and owns transactions.
use crate::runtime_plugins::{
    RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data,
};
use std::collections::BTreeMap;
const ASSETS: &[(&str, &str, &str)] = &[
    (
        "claim",
        "evidence-write-claim.v1.sql",
        "94c41d22dfd850628f56cf4274d14e77970c8735736269a83b574d819f83c83e",
    ),
    (
        "revision",
        "evidence-write-revision.v1.sql",
        "86c635874cd1bfe5cfe6124d343dd66a0f1545b77923bd011c2293c28e8b7cee",
    ),
    (
        "update",
        "evidence-write-update.v1.sql",
        "f4abd031ffdb06e925fc879045bf5db72aff67584e2112752c71f56718c58624",
    ),
    (
        "insert",
        "evidence-write-insert.v1.sql",
        "ce693af857b843b6c632fb0836493525c5516fd632798cac22e28112d80997c7",
    ),
    (
        "history",
        "evidence-write-history.v1.sql",
        "9113a4efe5a3cc5f07a6fb92edac17d54c75e862ae489faa647128e37bc7a44f",
    ),
    (
        "link-upsert",
        "evidence-write-link-upsert.v1.sql",
        "9bff8cf2a62b1a199da2227eed2cd96ad691e95b9b074e1abf23cae89532fe65",
    ),
    (
        "link-lookup",
        "evidence-write-link-lookup.v1.sql",
        "a82983c7da7860f99c8f6b8cab1c44182770b6241977c92e409f960c9bb91829",
    ),
];
pub(crate) struct ClaimsSql {
    statements: BTreeMap<&'static str, String>,
    pub(crate) reads: super::super::literature_evidence::EvidenceReadSql,
}
impl ClaimsSql {
    #[cfg(test)]
    pub(crate) fn for_test(
        statements: &[&str; 7],
        reads: super::super::literature_evidence::EvidenceReadSql,
    ) -> Self {
        Self {
            reads,
            statements: ASSETS
                .iter()
                .zip(statements)
                .map(|((key, _, _), sql)| (*key, (*sql).to_owned()))
                .collect(),
        }
    }

    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        let mut statements = BTreeMap::new();
        for &(key, name, hash) in ASSETS {
            statements.insert(
                key,
                data::load_text(
                    catalog,
                    package,
                    "literature_library",
                    &format!("data/{name}"),
                    hash,
                )?,
            );
        }
        Ok(Self {
            statements,
            reads: super::super::literature_evidence::EvidenceReadSql::load(catalog, package)?,
        })
    }

    pub(crate) fn get(&self, key: &str) -> &str {
        self.statements
            .get(key)
            .expect("fixed claim statement")
            .trim_end_matches('\n')
    }
}
