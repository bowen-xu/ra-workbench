//! Exact original confirmation-unit statements.
use crate::runtime_plugins::literature_reading_units::accepted::ReadingUnitAcceptanceSql;
use crate::runtime_plugins::{
    RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data::load_text,
};
pub(crate) struct ConfirmationUnitSql {
    pub(super) template_lookup: String,
    pub(super) ordinary_lookup: String,
    pub(super) template_link: String,
    pub(super) update: String,
    pub(super) current_revision: String,
    pub(super) revert: String,
    pub(super) restore: String,
    pub(super) snapshot: String,
    pub(super) snapshot_evidence: String,
    pub(super) evidence_delete: String,
    pub(super) evidence_insert: String,
    pub(super) evidence_restore: String,
    pub(super) revision_insert: String,
    pub(super) acceptance: ReadingUnitAcceptanceSql,
}
impl ConfirmationUnitSql {
    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        Ok(Self {
            template_lookup: load_text(
                catalog,
                package,
                "literature_library",
                "data/paper-confirmation-unit-template-lookup.v1.sql",
                "30192695659e764682036defa03dd493910238659d576616505978c5d2df82b1",
            )?,
            ordinary_lookup: load_text(
                catalog,
                package,
                "literature_library",
                "data/paper-confirmation-unit-ordinary-lookup.v1.sql",
                "44ce8f0762608c35220e8a910e48ba6836aa5bffeaec919c0455d47965dff3ad",
            )?,
            template_link: load_text(
                catalog,
                package,
                "literature_library",
                "data/paper-confirmation-unit-template-link.v1.sql",
                "f87b929815183f1e9915fac73b6778a9018b64ff4d797d8ff235f4785a355393",
            )?,
            update: load_text(
                catalog,
                package,
                "literature_library",
                "data/paper-confirmation-unit-update.v1.sql",
                "a580c82192a056e61364d849307bc7b962070b480d455d4ea36666cf7c8d844e",
            )?,
            current_revision: load_text(
                catalog,
                package,
                "literature_library",
                "data/paper-confirmation-unit-current-revision.v1.sql",
                "85216f0701b9af452d433caf13f5f7d9862d97d3aa929be4dfb699c9fe53e974",
            )?,
            revert: load_text(
                catalog,
                package,
                "literature_library",
                "data/paper-confirmation-unit-revert.v1.sql",
                "8d66a0e17a6be29af0ed949bcb015d035be763efefde1aee3f3ae7ab0721fb4f",
            )?,
            restore: load_text(
                catalog,
                package,
                "literature_library",
                "data/paper-confirmation-unit-restore.v1.sql",
                "110930d51f66ee96e042f54dd36ee7e7f139814993eae179b35423c048bb6316",
            )?,
            snapshot: load_text(
                catalog,
                package,
                "literature_library",
                "data/paper-confirmation-unit-snapshot.v1.sql",
                "32b5d55c7d8af4ce1f9b16c7ae626f176b75c85f781ac9d4ddf088fe567059e9",
            )?,
            snapshot_evidence: load_text(
                catalog,
                package,
                "literature_library",
                "data/paper-confirmation-unit-snapshot-evidence.v1.sql",
                "7a87903fe3d7859aa6ec78610ee78bff43cbe8bf78e07b377bfd1ae6020fbb92",
            )?,
            evidence_delete: load_text(
                catalog,
                package,
                "literature_library",
                "data/paper-confirmation-unit-evidence-delete.v1.sql",
                "282e6d85f6100e7de8f4eb076f93d9cfdb42eff42c3edd417438621fbfc48923",
            )?,
            evidence_insert: load_text(
                catalog,
                package,
                "literature_library",
                "data/paper-confirmation-unit-evidence-insert.v1.sql",
                "0532afda44e67f0217cefcbf8f52ab9b5d3bf2e8743f45dad2a95c47826f4e7e",
            )?,
            evidence_restore: load_text(
                catalog,
                package,
                "literature_library",
                "data/paper-confirmation-unit-evidence-restore.v1.sql",
                "76efd4c19ff9906bb1c68de136db5d43155dae31ea279e2d39712a264e27f284",
            )?,
            revision_insert: load_text(
                catalog,
                package,
                "literature_library",
                "data/nal-bootstrap-research-revision-insert.v1.sql",
                "1a31986a2e74182d3f5a70491fec1feda7cde5cd3cefd8d4ffb5723608393e9d",
            )?,
            acceptance: ReadingUnitAcceptanceSql::load(catalog, package)?,
        })
    }
}
