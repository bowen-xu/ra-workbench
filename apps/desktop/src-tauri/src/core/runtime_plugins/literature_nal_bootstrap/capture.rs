use super::{
    error::{CaptureError, Result},
    readset::ReadSet,
    resources::BootstrapResources,
    rows::Row,
};
use crate::{
    data_interface::plugin_resources::{ColumnCodec as C, RowPolicy as P},
    runtime_plugins::data::AcceptedDataResource,
};
use research_workbench_native_sdk::{
    data::{DataRows, DataValue},
    nal_bootstrap::*,
};
use rusqlite::Connection;

mod state;

pub(crate) struct PreparedBootstrapSource {
    pub(crate) source: BootstrapSourceV1,
    pub(super) reads: ReadSet,
}

impl PreparedBootstrapSource {
    pub(super) fn same_capture(&self, other: &Self) -> Result<()> {
        self.reads.same_capture(&other.reads)?;
        if self.source != other.source {
            return Err(CaptureError::changed(None));
        }
        Ok(())
    }
}

pub(super) struct Context<'a> {
    pub(super) db: &'a Connection,
    pub(super) reads: &'a mut ReadSet,
}

impl Context<'_> {
    pub(super) fn read<T>(
        &mut self,
        phase: BootstrapPhaseV1,
        resource: &AcceptedDataResource,
        arguments: Vec<DataValue>,
        policy: P,
        codecs: &[C],
        decode: impl FnOnce(DataRows) -> Result<T>,
    ) -> Result<T> {
        let result = self
            .reads
            .observe(self.db, phase, resource, arguments, policy, codecs)?;
        decode(result).inspect_err(|error| self.reads.fail_last(error.clone()))
    }
    fn probe<T>(
        &mut self,
        phase: BootstrapPhaseV1,
        resource: &AcceptedDataResource,
        arguments: Vec<DataValue>,
        policy: P,
        codecs: &[C],
        decode: impl FnOnce(DataRows) -> Result<T>,
    ) -> SourceProbe<T> {
        probe(self.read(phase, resource, arguments, policy, codecs, decode))
    }
}

pub(super) fn probe<T>(value: Result<T>) -> SourceProbe<T> {
    match value {
        Ok(value) => SourceProbe::Available { value },
        Err(_) => SourceProbe::Deferred {},
    }
}
pub(super) fn skipped<T>() -> SourceProbe<T> {
    SourceProbe::Skipped {}
}
pub(super) fn first(rows: DataRows) -> Result<Row> {
    rows.rows
        .into_iter()
        .next()
        .map(Row::from)
        .ok_or(CaptureError::Shape("required-row"))
}
pub(super) fn texts(rows: DataRows) -> Result<Vec<String>> {
    rows.rows
        .into_iter()
        .map(|row| Row::from(row).text())
        .collect()
}

impl BootstrapResources {
    /// R0 is planning input, never a revision token. All failures stay private
    /// until their original reachable named phase and all returned data is owned.
    pub(crate) fn capture_source(&self, db: &Connection) -> PreparedBootstrapSource {
        let mut reads = ReadSet::default();
        let mut c = Context {
            db,
            reads: &mut reads,
        };
        let legacy_imported = c.probe(
            BootstrapPhaseV1::LegacyHeader,
            &self.legacy_header,
            vec![],
            P::FirstRequired,
            &[C::Bool],
            |rows| first(rows)?.boolean(),
        );
        let collections: SourceProbe<Vec<LegacyCollectionV1>> =
            if matches!(legacy_imported, SourceProbe::Available { value: false }) {
                c.probe(
                    BootstrapPhaseV1::Collections,
                    &self.legacy_collections,
                    vec![],
                    P::All,
                    &[C::String, C::String, C::String, C::OptionalString],
                    |rows| {
                        rows.rows
                            .into_iter()
                            .map(|r| {
                                let mut r = Row::from(r);
                                Ok(LegacyCollectionV1 {
                                    id: r.text()?,
                                    name: r.text()?,
                                    description: r.text()?,
                                    concept_id: r.optional_text()?,
                                })
                            })
                            .collect()
                    },
                )
            } else {
                skipped()
            };
        let children = matches!(&collections, SourceProbe::Available {value} if !value.is_empty());
        let relation_types: SourceProbe<Vec<LegacyRelationTypeV1>> = if children {
            c.probe(
                BootstrapPhaseV1::RelationTypes,
                &self.legacy_types,
                vec![],
                P::All,
                &[C::String, C::String, C::String, C::Bool],
                |rows| {
                    rows.rows
                        .into_iter()
                        .map(|r| {
                            let mut r = Row::from(r);
                            Ok(LegacyRelationTypeV1 {
                                id: r.text()?,
                                name: r.text()?,
                                inverse_name: r.text()?,
                                directed: r.boolean()?,
                            })
                        })
                        .collect()
                },
            )
        } else {
            skipped()
        };
        let related_field_used = if matches!(&relation_types, SourceProbe::Available {value} if value.iter().any(|r|r.id=="related-field"))
        {
            c.probe(
                BootstrapPhaseV1::RelatedFieldExistence,
                &self.legacy_related_used,
                vec![DataValue::Text("related-field".into())],
                P::FirstRequired,
                &[C::Bool],
                |rows| first(rows)?.boolean(),
            )
        } else {
            skipped()
        };
        let relations = if children {
            c.probe(
                BootstrapPhaseV1::Relations,
                &self.legacy_relations,
                vec![],
                P::All,
                &[C::String, C::String, C::String, C::String],
                |rows| {
                    rows.rows
                        .into_iter()
                        .map(|r| {
                            let mut r = Row::from(r);
                            Ok(LegacyRelationV1 {
                                id: r.text()?,
                                source_id: r.text()?,
                                target_id: r.text()?,
                                type_id: r.text()?,
                            })
                        })
                        .collect()
                },
            )
        } else {
            skipped()
        };
        let memberships = if children {
            c.probe(
                BootstrapPhaseV1::Memberships,
                &self.legacy_memberships,
                vec![],
                P::All,
                &[C::String, C::String],
                |rows| {
                    rows.rows
                        .into_iter()
                        .map(|r| {
                            let mut r = Row::from(r);
                            Ok(LegacyMembershipV1 {
                                item_id: r.text()?,
                                collection_id: r.text()?,
                            })
                        })
                        .collect()
                },
            )
        } else {
            skipped()
        };
        let assignments = if children {
            c.probe(
                BootstrapPhaseV1::Assignments,
                &self.legacy_assignments,
                vec![],
                P::All,
                &[C::String, C::String],
                |rows| {
                    rows.rows
                        .into_iter()
                        .map(|r| {
                            let mut r = Row::from(r);
                            Ok(LegacyAssignmentV1 {
                                item_id: r.text()?,
                                relation_id: r.text()?,
                            })
                        })
                        .collect()
                },
            )
        } else {
            skipped()
        };
        let repair_version = c.probe(
            BootstrapPhaseV1::RepairVersion,
            &self.repair_version,
            vec![],
            P::FirstRequired,
            &[C::I64],
            |rows| first(rows)?.integer(),
        );
        let repair_records: SourceProbe<Vec<RepairRecordV1>> = if matches!(repair_version,SourceProbe::Available{value} if value!=1)
        {
            c.probe(
                BootstrapPhaseV1::RepairRecords,
                &self.repair_records,
                vec![],
                P::All,
                &[C::String, C::String, C::String],
                |rows| {
                    rows.rows
                        .into_iter()
                        .map(|r| {
                            let mut r = Row::from(r);
                            Ok(RepairRecordV1 {
                                id: r.text()?,
                                subject_json: r.text()?,
                                predicate_json: r.text()?,
                            })
                        })
                        .collect()
                },
            )
        } else {
            skipped()
        };
        let cleanup_exists = c.probe(
            BootstrapPhaseV1::CleanupExistence,
            &self.cleanup_exists,
            vec![],
            P::FirstRequired,
            &[C::Bool],
            |rows| first(rows)?.boolean(),
        );
        let raw_state = probe(state::capture(self, &mut c));
        let needs_concepts = children
            || matches!(&repair_records,SourceProbe::Available{value} if !value.is_empty());
        let occupied_name_keys = if needs_concepts {
            c.probe(
                BootstrapPhaseV1::ConceptNameCollision,
                &self.active_name_keys,
                vec![],
                P::All,
                &[C::String],
                texts,
            )
        } else {
            skipped()
        };
        let configured_embedding = if needs_concepts {
            probe(c.reads.configured(c.db))
        } else {
            skipped()
        };
        PreparedBootstrapSource {
            source: BootstrapSourceV1 {
                legacy_imported,
                collections,
                relation_types,
                related_field_used,
                relations,
                memberships,
                assignments,
                repair_version,
                repair_records,
                cleanup_exists,
                raw_state,
                occupied_name_keys,
                configured_embedding,
            },
            reads,
        }
    }
}
