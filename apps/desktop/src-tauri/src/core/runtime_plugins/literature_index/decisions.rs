//! Re-enter the original SQL prefix after a lock-free finite Native wait.
use super::{
    failure::Error,
    resources::{IndexResources, IndexSql},
};
use crate::ai::structured_resource_validation::DecisionOrigin;
use research_workbench_native_sdk::semantic_index::rules::{IndexDecisionV1, IndexInputV1};
use rusqlite::{Row, types::ValueRef};
use std::cell::{Cell, RefCell};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Scalar {
    Null,
    Integer(i64),
    Real(u64),
    Text(Vec<u8>),
    Blob(Vec<u8>),
}
#[derive(Clone, Debug, PartialEq, Eq)]
enum Observation {
    Statement(IndexSql),
    Row(Vec<Scalar>),
}
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Need {
    pub input: IndexInputV1,
    observations: Vec<Observation>,
}
pub(super) struct Decided {
    pub need: Need,
    pub decision: IndexDecisionV1,
    pub origin: DecisionOrigin,
}
#[derive(Default)]
pub(super) struct Cache(pub Vec<Decided>);
pub(super) struct IndexRun<'a> {
    pub resources: &'a IndexResources,
    cache: &'a Cache,
    slot: Cell<usize>,
    observations: RefCell<Vec<Observation>>,
}
impl<'a> IndexRun<'a> {
    pub fn new(resources: &'a IndexResources, cache: &'a Cache) -> Self {
        Self {
            resources,
            cache,
            slot: Cell::new(0),
            observations: RefCell::new(Vec::new()),
        }
    }
    pub fn sql(&self, key: IndexSql) -> &str {
        self.observations
            .borrow_mut()
            .push(Observation::Statement(key));
        self.resources.sql(key)
    }
    /// Decode first, preserving the original field and failure order. Capturing
    /// raw successful cells adds no new conversion before the original decoder.
    pub fn row<T>(
        &self,
        row: &Row<'_>,
        decode: impl FnOnce(&Row<'_>) -> rusqlite::Result<T>,
    ) -> rusqlite::Result<T> {
        let value = decode(row)?;
        let cells = (0..row.as_ref().column_count())
            .map(|index| {
                // Each index is bounded by this exact statement's column count.
                match row.get_ref_unwrap(index) {
                    ValueRef::Null => Scalar::Null,
                    ValueRef::Integer(value) => Scalar::Integer(value),
                    ValueRef::Real(value) => Scalar::Real(value.to_bits()),
                    ValueRef::Text(value) => Scalar::Text(value.to_vec()),
                    ValueRef::Blob(value) => Scalar::Blob(value.to_vec()),
                }
            })
            .collect();
        self.observations.borrow_mut().push(Observation::Row(cells));
        Ok(value)
    }
    pub fn decide(&self, input: IndexInputV1) -> Result<IndexDecisionV1, Error> {
        let need = Need {
            input,
            observations: self.observations.borrow().clone(),
        };
        let slot = self.slot.get();
        let Some(decided) = self.cache.0.get(slot) else {
            return Err(Error::Need(Box::new(need)));
        };
        if decided.need != need {
            return Err(Error::Conflict(decided.origin.clone()));
        }
        self.slot.set(slot + 1);
        match &decided.decision {
            IndexDecisionV1::Rejected { message } => Err(Error::Rejected {
                message: message.clone(),
                origin: decided.origin.clone(),
            }),
            value => Ok(value.clone()),
        }
    }
    pub fn validate(&self, input: IndexInputV1) -> Result<(), Error> {
        match self.decide(input)? {
            IndexDecisionV1::Validated => Ok(()),
            _ => Err(Error::mismatch()),
        }
    }
    pub fn flag(&self, input: IndexInputV1) -> Result<bool, Error> {
        match self.decide(input)? {
            IndexDecisionV1::Flag { value } => Ok(value),
            _ => Err(Error::mismatch()),
        }
    }
}
