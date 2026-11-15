//! Fixed original SQL for the seven NAL mutation variants.
use crate::runtime_plugins::{
    RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data::AcceptedDataResource,
    literature_nal_annotation::AnnotationResources, literature_nal_bootstrap::BootstrapResources,
};
use research_workbench_native_sdk::data::{DataAccess, DataField, DataResourceGrant, DataType};

const TEXT: DataField = DataField {
    data_type: DataType::Text,
    nullable: false,
};
const INTEGER: DataField = DataField {
    data_type: DataType::Integer,
    nullable: false,
};

#[derive(Clone)]
pub(crate) struct MutationResources {
    pub(super) annotation: AnnotationResources,
    pub(super) term_concept: AcceptedDataResource,
    pub(super) term_paper: AcceptedDataResource,
    pub(super) concept_used: AcceptedDataResource,
    pub(super) relation_kind: AcceptedDataResource,
    pub(super) kind_upsert: AcceptedDataResource,
    pub(super) rule_insert: AcceptedDataResource,
    pub(super) rule_delete: AcceptedDataResource,
    pub(super) statement_delete: AcceptedDataResource,
    pub(super) annotation_delete: AcceptedDataResource,
}

impl MutationResources {
    pub(crate) fn bootstrap(&self) -> &BootstrapResources {
        self.annotation.bootstrap()
    }

    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        let annotation = AnnotationResources::load(catalog, package)?;
        let load = |name: &str, hash: &str, access, parameters, columns| {
            AcceptedDataResource::load(
                catalog,
                package,
                "literature_library",
                &format!("data/nal-mutation-{name}.v1.sql"),
                DataResourceGrant {
                    id: format!("literature.nal-mutation.{name}"),
                    version: 1,
                    sha256: hash.into(),
                    access,
                    parameters,
                    columns,
                },
            )
        };
        Ok(Self {
            annotation,
            term_concept: load(
                "term-active-concept",
                "dc006d1a22f0d72ad9767ae89373219e3d32720598011ccece98b55fa885fa47",
                DataAccess::Read,
                vec![TEXT],
                vec![INTEGER],
            )?,
            term_paper: load(
                "term-active-paper",
                "cd09b2e60ec058f6032aa4277b8d9455684c991c8e9e1cc4d6d4bb3afc9b00f2",
                DataAccess::Read,
                vec![TEXT],
                vec![INTEGER],
            )?,
            concept_used: load(
                "concept-kind-used",
                "6acdb57d4570c64fd09676da17c00f79ac5544f08134a958701a5484107c9d20",
                DataAccess::Read,
                vec![TEXT],
                vec![INTEGER],
            )?,
            relation_kind: load(
                "relation-kind",
                "360f103d141698aebfe299b9e195e2e64cafa73eb5334233d1993fb8274edcaf",
                DataAccess::Read,
                vec![TEXT],
                vec![INTEGER],
            )?,
            kind_upsert: load(
                "concept-kind-upsert",
                "bbab2f5eb80b3dc3f96b1d2e3c8c95bd25868f457a96c016ec997f4c61513b67",
                DataAccess::Write,
                vec![TEXT, TEXT],
                vec![],
            )?,
            rule_insert: load(
                "rule-insert",
                "2e86830e0c4aed3b126cb71d97fa4bbe88ed8b4a1226414a7e7c9fd709da755b",
                DataAccess::Write,
                vec![TEXT],
                vec![],
            )?,
            rule_delete: load(
                "rule-delete",
                "59aee20070163eeb2c2cc9f008ae0ada115ef1c1a90efab1063d24baba99343d",
                DataAccess::Write,
                vec![TEXT],
                vec![],
            )?,
            statement_delete: load(
                "statement-delete",
                "8c62ec48c09da1eb02005e27c816821d6b77f607f3d874f8ffa2f419450f5a36",
                DataAccess::Write,
                vec![TEXT],
                vec![],
            )?,
            annotation_delete: load(
                "annotation-delete",
                "ada63050f48efccb75315ad9fcf41c659e2fff9b4df12153cf1c797521ff26bf",
                DataAccess::Write,
                vec![TEXT],
                vec![],
            )?,
        })
    }
}
