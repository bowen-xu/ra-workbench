//! Exact package texts; Native never submits SQL or chooses a statement.
#[path = "grants.rs"]
mod grants;
use crate::runtime_plugins::{
    RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data,
};
use std::collections::BTreeMap;
pub(crate) struct Sql {
    statements: BTreeMap<&'static str, String>,
    pub(super) checkpoints: super::super::checkpoints::Resources,
}
impl Sql {
    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        let mut statements = BTreeMap::new();
        for &(name, hash) in grants::ASSETS {
            statements.insert(
                name,
                data::load_text(
                    catalog,
                    package,
                    "writing_projects",
                    &format!("data/{name}"),
                    hash,
                )?,
            );
        }
        Ok(Self {
            statements,
            checkpoints: super::super::checkpoints::Resources::load(catalog, package)?,
        })
    }
    pub(super) fn get(&self, name: &str) -> &str {
        self.statements
            .get(name)
            .expect("finite reviewed bibliography statement")
            .trim_end_matches('\n')
    }
    pub(super) fn prepare<'a>(
        &self,
        db: &'a rusqlite::Connection,
        name: &str,
    ) -> rusqlite::Result<rusqlite::Statement<'a>> {
        let statement = db.prepare(self.get(name))?;
        if !statement.readonly() {
            return Err(rusqlite::Error::InvalidQuery);
        }
        Ok(statement)
    }
}
