//! Exact original SQL bytes; Core selects operations, binds values and owns transactions.
use crate::runtime_plugins::{
    RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data,
};
use std::collections::BTreeMap;
const ASSETS: &[(&str, &str, &str)] = &[
    (
        "ids",
        "evidence-matrix-ids.v1.sql",
        "8fb349d0ceed36bde6648c62f973e882f371b3288e7d6cc872e6c831ad1dbeb2",
    ),
    (
        "cell-header",
        "evidence-matrix-cell-header.v1.sql",
        "94da4be46480de58e6b12d92da2ddeb4f8b31595489c020b784bd9942e437290",
    ),
    (
        "cell-keys",
        "evidence-matrix-cell-keys.v1.sql",
        "1797a2109e93b4e2632a061da06e746f18db2a03926309e1a3a5943567145f4a",
    ),
    (
        "insert",
        "evidence-matrix-insert.v1.sql",
        "8706a32abf278cdb29051846fa677054d87fa7cd61f7c11db5b84218f56b4c9b",
    ),
    (
        "header",
        "evidence-matrix-header.v1.sql",
        "62a827b2a6bd734614428d2f9c352581c6af4baf720f7c7d885842c05e1fc23c",
    ),
    (
        "update",
        "evidence-matrix-update.v1.sql",
        "f2c891ef824454841d30afc1bf41bf8623067945af5be24e5a3308d0f3d3fd42",
    ),
    (
        "revision",
        "evidence-matrix-revision.v1.sql",
        "b860ac706ba7a1fffd35c9ce1ce690e0e30294dae44a7b6592e76bb55d833791",
    ),
    (
        "cell-insert",
        "evidence-matrix-cell-insert.v1.sql",
        "57d0652608e0150720225b713175685cc5ecd9f843b40dd0848847b196b0a0d2",
    ),
    (
        "cell-update",
        "evidence-matrix-cell-update.v1.sql",
        "07b843dbbaef451c555cb6819acc79f7c6dd3937e6693f997c7e877ddb1e5024",
    ),
    (
        "cell-revision",
        "evidence-matrix-cell-revision.v1.sql",
        "4f372e343ad30c9a28fa86a08ae2a8afc23114d210c7b16fe26a373adcffa841",
    ),
    (
        "matrix",
        "evidence-matrix-matrix.v1.sql",
        "56f3234aba25b70c5c1aac437048a1dac6fc796e7e38a22997e8257f0264adf6",
    ),
    (
        "cell",
        "evidence-matrix-cell.v1.sql",
        "5ac4e16b0550c16694934e836f8301c36767cb14bf2e4176ee05ae23dbf15780",
    ),
    (
        "project-belongs",
        "evidence-matrix-project-belongs.v1.sql",
        "ca10e2afb070fa6b290100644cbe2886a6ed135e3bbc129559defe58f619b736",
    ),
    (
        "anchor-belongs",
        "evidence-matrix-anchor-belongs.v1.sql",
        "bdb15893e7346797517a3eab447a4486ac44f24d1f69f89020437c0ca04442f4",
    ),
];
pub(crate) struct MatrixSql(BTreeMap<&'static str, String>);
impl MatrixSql {
    #[cfg(test)]
    pub(crate) fn for_test(statements: &[&str; 14]) -> Self {
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
            .expect("fixed matrix statement")
            .trim_end_matches('\n')
    }
}
