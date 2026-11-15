//! Fixed original paper-conversation reads and pointer write on the caller transaction.
use super::{
    creation_policy::{CreationNeed, CreationPolicy},
    error::AnalysisError,
};
use crate::ai::{
    paper_generation_admission::PaperConversationAdmission, structured_resource_validation::Flow,
};
use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

pub(crate) fn existing_generation_conversation(
    db: &Connection,
    sql: ConversationStatements<'_>,
    policy: &CreationPolicy,
    paper: &str,
) -> Result<Option<Uuid>, Flow<AnalysisError, CreationNeed>> {
    let core = PaperConversationAdmission::new(db);
    let repository = GenerationConversationRepository::new(db, sql);
    let portrait_active = core
        .active_run("paper_profile_classification", paper)
        .map_err(sql_error)?;
    let analysis_active = repository.active_analysis(paper).map_err(sql_error)?;
    policy.availability(portrait_active, analysis_active)?;
    let pointer = repository.pointer(paper).map_err(sql_error)?;
    let existing = core
        .unarchived_conversation(pointer.as_deref())
        .map_err(sql_error)?;
    policy.conversation(existing)
}

pub(crate) fn resolve_generation_conversation(
    db: &Connection,
    sql: ConversationStatements<'_>,
    policy: &CreationPolicy,
    paper: &str,
    candidate: Uuid,
) -> Result<Uuid, Flow<AnalysisError, CreationNeed>> {
    if let Some(existing) = existing_generation_conversation(db, sql, policy, paper)? {
        return Ok(existing);
    }
    let repository = GenerationConversationRepository::new(db, sql);
    let title = repository.title(paper).map_err(sql_error)?;
    PaperConversationAdmission::new(db)
        .insert_conversation_and_binding(candidate, paper, title.as_deref())
        .map_err(sql_error)?;
    repository
        .set_pointer(paper, candidate)
        .map_err(sql_error)?;
    Ok(candidate)
}

fn sql_error(error: rusqlite::Error) -> Flow<AnalysisError, CreationNeed> {
    Flow::original(error.into())
}

pub(crate) struct GenerationConversationRepository<'tx> {
    transaction: &'tx Connection,
    sql: ConversationStatements<'tx>,
}

impl<'tx> GenerationConversationRepository<'tx> {
    pub(crate) fn new(transaction: &'tx Connection, sql: ConversationStatements<'tx>) -> Self {
        Self { transaction, sql }
    }

    pub(crate) fn active_analysis(&self, paper: &str) -> rusqlite::Result<bool> {
        self.transaction
            .query_row(self.sql.active, [paper], |row| row.get(0))
    }

    pub(crate) fn pointer(&self, paper: &str) -> rusqlite::Result<Option<String>> {
        self.transaction
            .query_row(self.sql.pointer, [paper], |row| {
                row.get::<_, Option<String>>(0)
            })
            .optional()
            .map(Option::flatten)
    }

    pub(crate) fn title(&self, paper: &str) -> rusqlite::Result<Option<String>> {
        self.transaction
            .query_row(self.sql.title, [paper], |row| row.get(0))
            .optional()
    }

    pub(crate) fn set_pointer(&self, paper: &str, conversation: Uuid) -> rusqlite::Result<()> {
        self.transaction.execute(
            self.sql.set_pointer,
            params![paper, conversation.to_string()],
        )?;
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ConversationStatements<'a> {
    pub active: &'a str,
    pub pointer: &'a str,
    pub title: &'a str,
    pub set_pointer: &'a str,
}
