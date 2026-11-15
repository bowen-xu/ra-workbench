//! Fixed original legacy reading statements accepted from the selected package.
use crate::runtime_plugins::{
    RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data::load_text,
    literature_analysis::resources::HistoryResources,
    literature_context::preparation::PreparationResources,
};
pub(crate) struct ReadingHistoryResources {
    pub preparation: PreparationResources,
    pub history: HistoryResources,
    pub session_list: String,
    pub session_archive: String,
    pub session_turns: String,
    pub turn_debug: String,
    pub bootstrap_attachments: String,
}
impl ReadingHistoryResources {
    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        Ok(Self {
            preparation: PreparationResources::load(catalog, package)?,
            history: HistoryResources::load(catalog, package)?,
            session_list: load_text(
                catalog,
                package,
                "literature_library",
                "data/reading-history-session-list.v1.sql",
                "737ebb2ae7aaa0f3687e8738d6ce1c1429ed824b815409a35f90e761cce5946f",
            )?,
            session_archive: load_text(
                catalog,
                package,
                "literature_library",
                "data/reading-history-session-archive.v1.sql",
                "b6a7f0d96adc2ec49dd79da81c32e74ae32a93f962fce80b9e9e881e05120d54",
            )?,
            session_turns: load_text(
                catalog,
                package,
                "literature_library",
                "data/reading-history-session-turns.v1.sql",
                "d8026b7c5a3a422c5745f6aa0a494c264850a7a817917fa800e73128b38a3d98",
            )?,
            turn_debug: load_text(
                catalog,
                package,
                "literature_library",
                "data/reading-history-turn-debug.v1.sql",
                "6dabccc6539f7fc33f3d97b35efd5b3cb448b38aac89dbc1a7f39846e8778e83",
            )?,
            bootstrap_attachments: load_text(
                catalog,
                package,
                "literature_library",
                "data/reading-history-bootstrap-attachments.v1.sql",
                "15723de47b04405ef1dfc439fb2657d02d2d0e90feb9fbac017b68b9ccf2a0ae",
            )?,
        })
    }
}
