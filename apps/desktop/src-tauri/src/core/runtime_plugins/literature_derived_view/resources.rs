//! Exact original SQL bytes; Core selects operations, binds values and owns transactions.
use crate::runtime_plugins::{
    RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data,
};
use std::collections::BTreeMap;
const ASSETS: &[(&str, &str, &str)] = &[
    (
        "ids",
        "derived-view-ids.v1.sql",
        "814fdf544c99c1de193dd9e173ba50a68fa6e86953df00a0dfc0ae905fe40069",
    ),
    (
        "insert",
        "derived-view-insert.v1.sql",
        "6f7429f043d94e738e6e46b96548217508c5ba116638ac430c9a7b514ce176ce",
    ),
    (
        "header",
        "derived-view-header.v1.sql",
        "f778b525c34600c30950d5bb5a7c0faf05f043a1f6300702327af67626d4623f",
    ),
    (
        "update",
        "derived-view-update.v1.sql",
        "604cdc3642d2baef90d831845b7b4aee601fc9d62eae71daa7523bc270aa9f3b",
    ),
    (
        "revision",
        "derived-view-revision.v1.sql",
        "31227960462c069acd3446cfce0dcfeefaea9821317d6f3ed8a776017448836a",
    ),
    (
        "view",
        "derived-view-view.v1.sql",
        "8f6d105a4e3d76da6de36f9d216ed5d8eefdeb4a883a920bd3909d374e1536cd",
    ),
    (
        "input-snapshot",
        "derived-view-input-snapshot.v1.sql",
        "f71f9c15f165e0b24845760fed2421310f22b4f6ed24a7808827f24a06ed18a5",
    ),
];
pub(crate) struct DerivedSql(BTreeMap<&'static str, String>);
impl DerivedSql {
    #[cfg(test)]
    pub(crate) fn for_test(statements: &[&str; 7]) -> Self {
        Self(
            ASSETS
                .iter()
                .zip(statements)
                .map(|((key, _, _), sql)| (*key, (*sql).to_owned()))
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
            .expect("fixed derived-view statement")
            .trim_end_matches('\n')
    }
}
