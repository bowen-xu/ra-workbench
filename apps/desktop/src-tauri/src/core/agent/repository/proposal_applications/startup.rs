//! Retire ownership left by a previous process without replaying file effects.
use super::*;

/// Called before the application service admits any work, never on UI remount.
/// Keep every target phase and receipt intact. The entire scan commits together;
/// malformed identities or inconsistent proposal state fail startup recovery.
pub fn recover_on_startup(db: &mut Connection) -> Result<usize, Error> {
    let mut tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let scopes = {
        let mut statement = tx.prepare(
            "select a.id,a.change_set_id,c.conversation_id,c.manuscript_id
             from agent_proposal_applications a join agent_change_sets c on c.id=a.change_set_id
             where a.direction='apply' and a.status='applying' order by a.id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        let mut scopes = Vec::new();
        for row in rows {
            let (operation, proposal, conversation, manuscript) = row?;
            let parse = |value: &str| {
                Uuid::parse_str(value)
                    .ok()
                    .filter(|id| !id.is_nil())
                    .ok_or(Error::Conflict)
            };
            scopes.push(ApplicationScope {
                operation: parse(&operation)?,
                proposal: parse(&proposal)?,
                conversation: parse(&conversation)?,
                manuscript: parse(&manuscript)?,
            });
        }
        scopes
    };
    for scope in &scopes {
        require_recovery_in(&mut tx, *scope)?;
    }
    tx.commit()?;
    Ok(scopes.len())
}
