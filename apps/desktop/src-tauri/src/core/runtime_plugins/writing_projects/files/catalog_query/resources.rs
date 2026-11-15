//! Fixed reviewed SQL. Only search term positions/counts are mechanically filled.
use super::grants::ASSETS;
use crate::runtime_plugins::{
    RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data,
};
use research_workbench_native_sdk::writing_catalog_query::{QueryCursor, QueryPlan};
use std::collections::BTreeMap;

pub(crate) struct QuerySql {
    statements: BTreeMap<String, String>,
}
impl QuerySql {
    pub(super) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        let mut statements = BTreeMap::new();
        for (name, hash) in ASSETS {
            let text = data::load_text(
                catalog,
                package,
                "writing_projects",
                &format!("data/{name}"),
                hash,
            )?;
            statements.insert((*name).into(), text);
        }
        Ok(Self { statements })
    }
    /// Static App composition supplies the same canonical plugin-owned assets.

    pub(super) fn get(&self, name: &str) -> &str {
        self.statements
            .get(name)
            .expect("reviewed fixed catalog asset")
            .trim_end_matches('\n')
    }
    pub(super) fn scoped(&self, kind: &str, search: bool) -> &str {
        self.get(&format!(
            "catalog-query-{kind}-{}.v1.sql",
            if search { "all" } else { "status" }
        ))
    }
    pub(super) fn items(&self, plan: &QueryPlan) -> String {
        let search = !plan.terms.is_empty();
        let after = matches!(plan.cursor, QueryCursor::Ready { .. });
        let result = self.get(&format!(
            "catalog-query-{}-{}-{}.v1.sql",
            if search { "search" } else { "browse" },
            plan.order.asset_key(),
            u8::from(after)
        ));
        if !search {
            return result.into();
        }
        research_note_query::catalog_search_sql(
            research_note_query::CatalogSearchSql {
                scan_body: self.get("catalog-query-scan-body.v1.sql"),
                scan_metadata: self.get("catalog-query-scan-metadata.v1.sql"),
                fts_body: self.get("catalog-query-fts-body.v1.sql"),
                fts_metadata: self.get("catalog-query-fts-metadata.v1.sql"),
                result,
            },
            &plan.terms.iter().map(|term| term.scan).collect::<Vec<_>>(),
        )
    }
}
