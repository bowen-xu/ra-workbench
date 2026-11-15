//! Original outer merge SQL; callers retain selection, scheduling and settlement.
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

#[derive(Clone)]
pub(crate) struct OuterMergeResources {
    pub(crate) header: AcceptedDataResource,
    pub(crate) archived_at: AcceptedDataResource,
    pub(crate) attachments: AcceptedDataResource,
    pub(crate) identifiers: AcceptedDataResource,
    pub(crate) anchors: AcceptedDataResource,
    pub(crate) added_projects: AcceptedDataResource,
    pub(crate) added_tags: AcceptedDataResource,
    pub(crate) added_provenance: AcceptedDataResource,
    pub(crate) insert_project: AcceptedDataResource,
    pub(crate) insert_tag: AcceptedDataResource,
    pub(crate) insert_provenance: AcceptedDataResource,
    pub(crate) move_reading_states: AcceptedDataResource,
    pub(crate) move_attachments: AcceptedDataResource,
    pub(crate) move_identifiers: AcceptedDataResource,
    pub(crate) move_anchors: AcceptedDataResource,
    pub(crate) move_reading_sources: AcceptedDataResource,
    pub(crate) archive_source: AcceptedDataResource,
    pub(crate) mark_candidate: AcceptedDataResource,
    pub(crate) search_delete: AcceptedDataResource,
    pub(crate) search_insert: AcceptedDataResource,
    pub(crate) titles: AcceptedDataResource,
    pub(crate) timestamp: AcceptedDataResource,
    pub(crate) history_insert: AcceptedDataResource,
}

impl OuterMergeResources {
    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        let loader = Loader { catalog, package };
        Ok(Self {
            header: loader.outer(
                "header",
                "c1d3c77f8ba26da7ccb49aa491f9e07bf39613a4a5abd11e511ddb20465678c2",
                DataAccess::Read,
                &[T],
                &[T, T],
            )?,
            archived_at: loader.outer(
                "archived-at",
                "384cac10da083a7be760e01945138b382bd06190bb22a80334a88d32b6da679d",
                DataAccess::Read,
                &[T],
                &[N],
            )?,
            attachments: loader.outer(
                "attachments",
                "9dc7c56b91b43bce7e9ad728210b8909bc61246fbec996f7de4d3a2dfb40fed8",
                DataAccess::Read,
                &[T],
                &[T],
            )?,
            identifiers: loader.outer(
                "identifiers",
                "c4178203cbbf67d65063994f05b8c822c9ef4f2b734ed57d64ef31ac7c9de71d",
                DataAccess::Read,
                &[T],
                &[T],
            )?,
            anchors: loader.outer(
                "anchors",
                "99390ecae8320067f3897ba30680864d151fb1e2567f12a48a62a0163edd075b",
                DataAccess::Read,
                &[T],
                &[T],
            )?,
            added_projects: loader.outer(
                "added-projects",
                "c6ad55903b384e754dca065fd6d6f5113f625fb355f918c10e94926f54745d2e",
                DataAccess::Read,
                &[T, T],
                &[T],
            )?,
            added_tags: loader.outer(
                "added-tags",
                "5e8b9f30211b00befe70cbeed47f436795740ba4942118147c09b611d5af037a",
                DataAccess::Read,
                &[T, T],
                &[T],
            )?,
            added_provenance: loader.outer(
                "added-provenance",
                "944d0d4799a93a9fe292fdf42dc5056b36a30291ba3423dd6d6c0a984570fbed",
                DataAccess::Read,
                &[T, T],
                &[T],
            )?,
            insert_project: loader.outer(
                "insert-project",
                "c46b25392d430a2a8fc1e6fbdc98cbf85d552d2e46174c2ca59390fcee3b5d95",
                DataAccess::Write,
                &[T, T, T],
                &[],
            )?,
            insert_tag: loader.outer(
                "insert-tag",
                "4c1a66671d9febd848e1b216c66d4323c6b69a66a357ec2062b46c90dd6df761",
                DataAccess::Write,
                &[T, T],
                &[],
            )?,
            insert_provenance: loader.outer(
                "insert-provenance",
                "758405c52973b9819f00268f7fa4cc2e94153aee08172c822dfb1c1f6d97e802",
                DataAccess::Write,
                &[T, T, T],
                &[],
            )?,
            move_reading_states: loader.outer(
                "move-reading-states",
                "f2b8fb247fbc265bb1c7139b9dd22049c200537b9b86a85a6783e70fdb948ed6",
                DataAccess::Write,
                &[T, T],
                &[],
            )?,
            move_attachments: loader.outer(
                "move-attachments",
                "64aa3a92cc14970e6a1e15933491e41ada702aa73e537177253dfb294e7802c1",
                DataAccess::Write,
                &[T, T],
                &[],
            )?,
            move_identifiers: loader.outer(
                "move-identifiers",
                "e86fcdf80a4869422e5afe766cac2af66d5b4f0aaabb4986a213fa3eedf31b77",
                DataAccess::Write,
                &[T, T],
                &[],
            )?,
            move_anchors: loader.outer(
                "move-anchors",
                "587d223aeb242cd56124b12c9daabf213964d973af5f1dc485b1fd88be54ba3a",
                DataAccess::Write,
                &[T, T],
                &[],
            )?,
            move_reading_sources: loader.outer(
                "move-reading-sources",
                "28d6958dea86d0c85ab9d4b52b8826d6eede2c63d6824723eb67e1b57c6ea726",
                DataAccess::Write,
                &[T, T],
                &[],
            )?,
            archive_source: loader.outer(
                "archive-source",
                "0f84025406abbf79a590f10e197800ffe7de2e918e65d9eec6f8c9e4359d7eb5",
                DataAccess::Write,
                &[T],
                &[],
            )?,
            mark_candidate: loader.outer(
                "mark-candidate",
                "10e06ff343cf6d941041a1dc532bccf3caf124a3c20bb986a0953c1c4b693941",
                DataAccess::Write,
                &[T],
                &[],
            )?,
            search_delete: loader.search(
                "search-delete",
                "715c7a5add6de4717ac9e65b14e27cfabe82ede482fda066a46ae900837ea9fe",
            )?,
            search_insert: loader.search(
                "search-insert",
                "2b090db8c7abc5eaa72bfafdaf1ca5320a2347d6fc3adb1ed77e2b81824cc85f",
            )?,
            titles: loader.outer(
                "titles",
                "11302a18f3bef22364dc2a182cad24c7aedf01032f210ca683a452d4acac4332",
                DataAccess::Read,
                &[T, T],
                &[T, T],
            )?,
            timestamp: loader.outer(
                "timestamp",
                "d3dc28e1b112e68a935a0b4dfaae226af8e5dc70634c4b4255d72d22ac4a7338",
                DataAccess::Read,
                &[],
                &[T],
            )?,
            history_insert: loader.outer(
                "history-insert",
                "69883e209e0eae9576bcfed1271fab835dc98c449fd89aec07f77117726d221f",
                DataAccess::Write,
                &[T, T, T, T, T],
                &[],
            )?,
        })
    }
}

struct Loader<'a> {
    catalog: &'a RuntimePluginCatalog,
    package: &'a RuntimePluginPackage,
}

impl Loader<'_> {
    fn outer(
        &self,
        name: &str,
        sha256: &str,
        access: DataAccess,
        parameters: &[DataField],
        columns: &[DataField],
    ) -> Result<AcceptedDataResource, RuntimePluginError> {
        self.load(
            &format!("data/duplicate-merge-outer-{name}.v1.sql"),
            &format!("literature.duplicate-merge.outer.{name}"),
            sha256,
            access,
            parameters,
            columns,
        )
    }

    fn search(&self, name: &str, sha256: &str) -> Result<AcceptedDataResource, RuntimePluginError> {
        self.load(
            &format!("data/duplicate-undo-{name}.v1.sql"),
            &format!("literature.duplicate-undo.{name}"),
            sha256,
            DataAccess::Write,
            &[T],
            &[],
        )
    }

    fn load(
        &self,
        path: &str,
        id: &str,
        sha256: &str,
        access: DataAccess,
        parameters: &[DataField],
        columns: &[DataField],
    ) -> Result<AcceptedDataResource, RuntimePluginError> {
        AcceptedDataResource::load(
            self.catalog,
            self.package,
            "literature_library",
            path,
            DataResourceGrant {
                id: id.into(),
                version: 1,
                sha256: sha256.into(),
                access,
                parameters: parameters.to_vec(),
                columns: columns.to_vec(),
            },
        )
    }
}
