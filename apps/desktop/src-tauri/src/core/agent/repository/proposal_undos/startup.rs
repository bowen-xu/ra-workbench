//! Retire interrupted undo ownership; prepared baselines have no side effects.
use super::*;

pub fn recover_on_startup(db: &mut Connection) -> Result<usize, Error> {
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let scopes = {
        let mut query = tx.prepare("select u.operation_id,u.original_operation_id,a.change_set_id,c.conversation_id,c.manuscript_id from agent_proposal_undos u join agent_proposal_applications a on a.id=u.operation_id join agent_change_sets c on c.id=a.change_set_id where a.status='applying' order by u.operation_id")?;
        let rows = query
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut scopes = Vec::new();
        for (operation, original, proposal, conversation, manuscript) in rows {
            let parse = |text: &str| {
                Uuid::parse_str(text)
                    .ok()
                    .filter(|id| !id.is_nil())
                    .ok_or(Error::Conflict)
            };
            scopes.push((
                parse(&operation)?,
                ApplicationScope {
                    operation: parse(&original)?,
                    proposal: parse(&proposal)?,
                    conversation: parse(&conversation)?,
                    manuscript: parse(&manuscript)?,
                },
            ));
        }
        scopes
    };
    for (operation, original) in &scopes {
        require_recovery_in(&tx, *original, *operation)?;
    }
    tx.commit()?;
    Ok(scopes.len())
}
