//! Fixed original evidence reads, with no write or arbitrary query entry.
use crate::runtime_plugins::{
    RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data,
};
use std::collections::BTreeMap;
const ASSETS: &[(&str, &str, &str)] = &[
    (
        "claims",
        "evidence-read-claims.v1.sql",
        "8e15adc6e35150b2ca5c8296567d8d32ca56b2a37a662c2a6f19e2faa993ad33",
    ),
    (
        "source",
        "evidence-read-source.v1.sql",
        "e995ba215a620a7ceb895d45606e7b6d7c1921f7fcdd67f03f8b7a77d7743be3",
    ),
    (
        "sources",
        "evidence-read-sources.v1.sql",
        "eaba5530816a93c91aa355a9d6a64d96faec3abeaade1d06f8d346f2a7bb601f",
    ),
    (
        "links",
        "evidence-read-links.v1.sql",
        "1fffd1c60cb1d8a4b1751fb69e61b964d5eaf15fc60780a91ba68e6969712e3e",
    ),
];
pub(crate) struct EvidenceReadSql(BTreeMap<&'static str, String>);
impl EvidenceReadSql {
    #[cfg(test)]
    pub(crate) fn for_test(statements: [&str; 4]) -> Self {
        Self(
            ASSETS
                .iter()
                .zip(statements)
                .map(|((key, _, _), sql)| (*key, sql.to_owned()))
                .collect(),
        )
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
        Ok(Self(statements))
    }

    pub(crate) fn get(&self, key: &str) -> &str {
        self.0
            .get(key)
            .expect("fixed evidence read")
            .trim_end_matches('\n')
    }
}
