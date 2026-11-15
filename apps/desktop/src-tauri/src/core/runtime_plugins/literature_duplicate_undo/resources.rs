//! Fixed undo resources; domain SQL remains in the accepted literature package.
use crate::runtime_plugins::{
    RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data::AcceptedDataResource,
};
use research_workbench_native_sdk::data::{DataAccess, DataField, DataResourceGrant, DataType};

const T: DataField = DataField {
    data_type: DataType::Text,
    nullable: false,
};
const N: DataField = DataField {
    data_type: DataType::Text,
    nullable: true,
};
const I: DataField = DataField {
    data_type: DataType::Integer,
    nullable: false,
};

#[derive(Clone)]
pub(crate) struct UndoResources {
    pub(super) header: AcceptedDataResource,
    pub(super) collection_revision: AcceptedDataResource,
    pub(super) collection_collections: AcceptedDataResource,
    pub(super) collection_types: AcceptedDataResource,
    pub(super) collection_relations: AcceptedDataResource,
    pub(super) collection_memberships: AcceptedDataResource,
    pub(super) collection_assignments: AcceptedDataResource,
    pub(super) nal_revision: AcceptedDataResource,
    pub(super) nal_concepts: AcceptedDataResource,
    pub(super) nal_statements: AcceptedDataResource,
    pub(super) nal_annotations: AcceptedDataResource,
    pub(super) nal_rules: AcceptedDataResource,
    pub(super) delete_statement: AcceptedDataResource,
    pub(super) delete_annotation: AcceptedDataResource,
    pub(super) increment_nal_revision: AcceptedDataResource,
    pub(super) delete_assignment: AcceptedDataResource,
    pub(super) delete_membership: AcceptedDataResource,
    pub(super) increment_collection_revision: AcceptedDataResource,
    pub(super) restore_anchor: AcceptedDataResource,
    pub(super) restore_reading_sources: AcceptedDataResource,
    pub(super) restore_attachment: AcceptedDataResource,
    pub(super) restore_reading_state: AcceptedDataResource,
    pub(super) restore_identifier: AcceptedDataResource,
    pub(super) delete_project: AcceptedDataResource,
    pub(super) delete_tag: AcceptedDataResource,
    pub(super) delete_provenance: AcceptedDataResource,
    pub(super) restore_archive: AcceptedDataResource,
    pub(super) reset_candidate: AcceptedDataResource,
    pub(super) mark_undone: AcceptedDataResource,
    pub(super) search_delete: AcceptedDataResource,
    pub(super) search_insert: AcceptedDataResource,
    pub(super) collection_scope: AcceptedDataResource,
    pub(super) statement_scope: AcceptedDataResource,
    pub(super) annotation_scope: AcceptedDataResource,
    pub(super) vector_equal: AcceptedDataResource,
}

impl UndoResources {
    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        let loader = Loader { catalog, package };
        Ok(Self {
            header: loader.undo(
                "header",
                "c3f06f758ae8f61008cf87967b18a874c6c7ef8f3996d21613733901921257ee",
                DataAccess::Read,
                &[T],
                &[T, T, T],
            )?,
            collection_revision: loader.undo(
                "collection-revision",
                "0638f464e3ed8b6b01a469fe41e980eb8b1c6e5445b6b3c2becc9d15649cc3d0",
                DataAccess::Read,
                &[],
                &[I],
            )?,
            collection_collections: loader.undo(
                "collection-collections",
                "c31f3be5c7b06c8211f360cd46a5ea303ae9971f0291f36007a7fe9612aed787",
                DataAccess::Read,
                &[],
                &[T, T, T, N],
            )?,
            collection_types: loader.undo(
                "collection-types",
                "ebe6c7c433c7ae75de58853efada58d13eb1e775c3624627f3ac4d8a63856f17",
                DataAccess::Read,
                &[],
                &[T, T, T, I, I],
            )?,
            collection_relations: loader.undo(
                "collection-relations",
                "de7d852f700d9ddb6775fe34f0e2dea6ec11f29b517fb0e580529f2b6ba6c2ce",
                DataAccess::Read,
                &[],
                &[T, T, T, T],
            )?,
            collection_memberships: loader.undo(
                "collection-memberships",
                "5476cf7f3a248d6045825e6c81274f684649a5aacc9d2bc01b875dd8d2743fb4",
                DataAccess::Read,
                &[],
                &[T, T],
            )?,
            collection_assignments: loader.undo(
                "collection-assignments",
                "d4863f31b212bb8fd51f0bbf33ad9d624fcbac6a9cacc01a624ad25aa5b77f2e",
                DataAccess::Read,
                &[],
                &[T, T],
            )?,
            nal_revision: loader.nal_state(
                "state-revision",
                "2ba220db09049103ad7cda0f18f097787b8d61a327c144eaa1eb3ec924a9d811",
                &[I],
            )?,
            nal_concepts: loader.nal_state(
                "state-concepts",
                "2d9070c314584c91d56271db2c5fa7c429abb8a4a6543fbb959ea180bb583d68",
                &[T, T, T, I, T],
            )?,
            nal_statements: loader.nal_state(
                "state-statements",
                "0aa93e785efe07d4e3ee5a9edda8b88d0e38157f3302dad280eeaac3445c0599",
                &[T, T, T, T],
            )?,
            nal_annotations: loader.nal_state(
                "state-annotations",
                "c7f5efbad3a3cd467900dcd292fd374c50944ddaccd121b47ac31654032b07df",
                &[T, T, T, T],
            )?,
            nal_rules: loader.nal_state(
                "state-rules",
                "26139b905d3d915f6a3d605108a36bf1169f2e5ed07846a29892ff4a2e2ba923",
                &[T],
            )?,
            delete_statement: loader.undo(
                "delete-statement",
                "34e95978a9692d960886e928deb7d7335719232edfa04e3a923eda9c7a30ecea",
                DataAccess::Write,
                &[T],
                &[],
            )?,
            delete_annotation: loader.undo(
                "delete-annotation",
                "458fa0f3d96cbc162ed581c0412bd55b60cfb400c173c48ae778e74bdf345e00",
                DataAccess::Write,
                &[T],
                &[],
            )?,
            increment_nal_revision: loader.undo(
                "increment-nal-revision",
                "5a3bbfaec5a693ea4f8161f938ef9e06062ed8e76783e3ca5f536283ba731100",
                DataAccess::Write,
                &[],
                &[],
            )?,
            delete_assignment: loader.undo(
                "delete-assignment",
                "d62b1ec37578c65b9273d56404c0a959df39d171fe1f9d6af894385610f67272",
                DataAccess::Write,
                &[T, T],
                &[],
            )?,
            delete_membership: loader.undo(
                "delete-membership",
                "ddcb8a64296591ef12452860ff0002f9d5aee1c1f47d5081e4f473b2a87b9e08",
                DataAccess::Write,
                &[T, T],
                &[],
            )?,
            increment_collection_revision: loader.undo(
                "increment-collection-revision",
                "12ea366dcd0e087573739546fb896d833fbe8c049d7f79f6f3af26c9c989157e",
                DataAccess::Write,
                &[],
                &[],
            )?,
            restore_anchor: loader.undo(
                "restore-anchor",
                "ff611eade5bf79ec8c5a7464007ca0dabfd7f57bec6a3584b0447ae9acc66d4c",
                DataAccess::Write,
                &[T, T, T],
                &[],
            )?,
            restore_reading_sources: loader.undo(
                "restore-reading-sources",
                "28d6958dea86d0c85ab9d4b52b8826d6eede2c63d6824723eb67e1b57c6ea726",
                DataAccess::Write,
                &[T, T],
                &[],
            )?,
            restore_attachment: loader.undo(
                "restore-attachment",
                "8ca873ca09c82083acb73181bfc6b29b4b63bd42ea90a89b57cffcb835a953a2",
                DataAccess::Write,
                &[T, T, T],
                &[],
            )?,
            restore_reading_state: loader.undo(
                "restore-reading-state",
                "48aaa5ef79e16a0a963b6f92cc6da3ce8f234d7ec218430d2e1824b104c6ea3e",
                DataAccess::Write,
                &[T, T, T],
                &[],
            )?,
            restore_identifier: loader.undo(
                "restore-identifier",
                "96e996ccbe7c268a94cfe9a64cca0038ab171b245cc96e1d361066185211e17b",
                DataAccess::Write,
                &[T, T, T],
                &[],
            )?,
            delete_project: loader.undo(
                "delete-project",
                "5698f752fd66e1db7b5d12d17807ca8645a2ff20c613242a1cdd7b25fe45c0d5",
                DataAccess::Write,
                &[T, T],
                &[],
            )?,
            delete_tag: loader.undo(
                "delete-tag",
                "f89ddfaf93275c12cde8b9b1d8980b5ddcf450654b88e5488a15a69de7e48519",
                DataAccess::Write,
                &[T, T],
                &[],
            )?,
            delete_provenance: loader.undo(
                "delete-provenance",
                "b45ffd69d4c89a05bc337b90006b36730b61eac3c502ee46cad0446f756daea8",
                DataAccess::Write,
                &[T, T],
                &[],
            )?,
            restore_archive: loader.undo(
                "restore-archive",
                "01946f81a9fb45ee7af5c051b02c57f177552c23d88cc8f3f6e9e94f4a07c458",
                DataAccess::Write,
                &[T, N],
                &[],
            )?,
            reset_candidate: loader.undo(
                "reset-candidate",
                "5de78aac45bf719d96191acab0f6ce2441769b19d79eba9ad3898354acffe3b9",
                DataAccess::Write,
                &[T],
                &[],
            )?,
            mark_undone: loader.undo(
                "mark-undone",
                "91c5f8a2f9da1bc90300d1b13b4008d57376daf1bac183ab50233b097fd751fe",
                DataAccess::Write,
                &[T],
                &[],
            )?,
            search_delete: loader.undo(
                "search-delete",
                "715c7a5add6de4717ac9e65b14e27cfabe82ede482fda066a46ae900837ea9fe",
                DataAccess::Write,
                &[T],
                &[],
            )?,
            search_insert: loader.undo(
                "search-insert",
                "2b090db8c7abc5eaa72bfafdaf1ca5320a2347d6fc3adb1ed77e2b81824cc85f",
                DataAccess::Write,
                &[T],
                &[],
            )?,
            collection_scope: loader.undo(
                "collection-scope",
                "55ab9580b79f8c70b8ed40c1a4717541ec530eb5013b1bd49dd65b709173798a",
                DataAccess::Read,
                &[T, T],
                &[T],
            )?,
            statement_scope: loader.undo(
                "statement-scope",
                "2ae437cc4a76b2f907a97092c0f5bc11343f63a30781d57d91a9c83cbb53489d",
                DataAccess::Read,
                &[T, T],
                &[T],
            )?,
            annotation_scope: loader.undo(
                "annotation-scope",
                "f3d82c7a2a5dd143c2d6b0f1ec08e87fa93ed3fc22617220aae9aa87c8a01908",
                DataAccess::Read,
                &[T, T],
                &[T],
            )?,
            vector_equal: loader.undo(
                "vector-equal",
                "7967cf96f86a67f06db2ab624e06d2d3152822cbce7a620221a5245366aa431a",
                DataAccess::Read,
                &[T, T],
                &[I],
            )?,
        })
    }

    pub(crate) fn package_key(&self) -> &str {
        self.header.package_key()
    }
}

struct Loader<'a> {
    catalog: &'a RuntimePluginCatalog,
    package: &'a RuntimePluginPackage,
}

impl Loader<'_> {
    fn undo(
        &self,
        name: &str,
        sha256: &str,
        access: DataAccess,
        parameters: &[DataField],
        columns: &[DataField],
    ) -> Result<AcceptedDataResource, RuntimePluginError> {
        AcceptedDataResource::load(
            self.catalog,
            self.package,
            "literature_library",
            &format!("data/duplicate-undo-{name}.v1.sql"),
            DataResourceGrant {
                id: format!("literature.duplicate-undo.{name}"),
                version: 1,
                sha256: sha256.into(),
                access,
                parameters: parameters.to_vec(),
                columns: columns.to_vec(),
            },
        )
    }

    fn nal_state(
        &self,
        name: &str,
        sha256: &str,
        columns: &[DataField],
    ) -> Result<AcceptedDataResource, RuntimePluginError> {
        AcceptedDataResource::load(
            self.catalog,
            self.package,
            "literature_library",
            &format!("data/nal-bootstrap-{name}.v1.sql"),
            DataResourceGrant {
                id: format!("literature.nal-bootstrap.{name}"),
                version: 1,
                sha256: sha256.into(),
                access: DataAccess::Read,
                parameters: vec![],
                columns: columns.to_vec(),
            },
        )
    }
}
