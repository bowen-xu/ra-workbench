//! The original duplicate undo, using package rules and Core-owned transactions.
pub(crate) mod commands;
mod execute;
mod load;
mod outcome;
mod resources;

pub(crate) use execute::apply;
pub(crate) use load::{CapturedUndo, capture};
pub(crate) use outcome::{UndoCause, UndoFailure, UndoOutcome, UndoPhase, UndoProblem};
pub(crate) use resources::UndoResources;

#[cfg(test)]
pub(crate) mod tests;
