//! Fixed resources for singular NAL annotations and their original startup chain.
use crate::runtime_plugins::{
    RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data::AcceptedDataResource,
    literature_nal_bootstrap::BootstrapResources,
};
use research_workbench_native_sdk::data::{DataAccess, DataField, DataResourceGrant, DataType};

const TEXT: DataField = DataField {
    data_type: DataType::Text,
    nullable: false,
};
const OPTIONAL_TEXT: DataField = DataField {
    data_type: DataType::Text,
    nullable: true,
};
const INTEGER: DataField = DataField {
    data_type: DataType::Integer,
    nullable: false,
};

#[derive(Clone)]
pub(crate) struct AnnotationResources {
    pub(super) bootstrap: BootstrapResources,
    pub(super) note_active: AcceptedDataResource,
    pub(super) insert: AcceptedDataResource,
    pub(super) revision_increment: AcceptedDataResource,
}

impl AnnotationResources {
    pub(crate) fn bootstrap(&self) -> &BootstrapResources {
        &self.bootstrap
    }

    pub(crate) fn revision_increment(&self) -> &AcceptedDataResource {
        &self.revision_increment
    }

    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        let bootstrap = BootstrapResources::load(catalog, package)?;
        let load = |name: &str, hash: &str, access, parameters, columns| {
            AcceptedDataResource::load(
                catalog,
                package,
                "literature_library",
                &format!("data/nal-annotation-{name}.v1.sql"),
                DataResourceGrant {
                    id: format!("literature.nal-annotation.{name}"),
                    version: 1,
                    sha256: hash.into(),
                    access,
                    parameters,
                    columns,
                },
            )
        };
        Ok(Self {
            bootstrap,
            note_active: load(
                "note-active",
                "828670cdeda761e1e8d9979e1a242bb394f90faab78e0eb735cc0b9e8b1e1d06",
                DataAccess::Read,
                vec![TEXT],
                vec![INTEGER],
            )?,
            insert: load(
                "insert",
                "6f2840c86d1446b93f244201fe2a922c53e2f2921b0f90af231a72b2b7918e78",
                DataAccess::Write,
                vec![TEXT, OPTIONAL_TEXT, OPTIONAL_TEXT, TEXT],
                vec![],
            )?,
            revision_increment: load(
                "revision-increment",
                "9f470b14146c14bfcb22446e585727ecb6deba000d08096df0e12faf938d2f5a",
                DataAccess::Write,
                vec![],
                vec![],
            )?,
        })
    }
}
