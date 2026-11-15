//! Frozen inverse sources are provenance; undispatched targets own live CAS.
use super::*;
use applications::ApplicationPhase;
use research_agent_core::workspace::EditorVersion;
use std::collections::BTreeMap;

/// Caller has preflighted every source, byte version and live target in this tx.
pub(crate) fn rebind_pending_editor_targets_in(
    tx: &mut Transaction<'_>,
    original: ApplicationScope,
    operation: Uuid,
    targets: &BTreeMap<String, Option<EditorVersion>>,
) -> Result<(), Error> {
    let saved = read_in(tx, original, operation)?.ok_or(Error::Conflict)?;
    if !matches!(
        saved.application.status,
        ApplicationStatus::Prepared | ApplicationStatus::RecoveryRequired
    ) || saved.application.files.len() != targets.len()
        || saved.application.files.iter().any(|file| {
            !matches!(
                file.phase,
                ApplicationPhase::Pending | ApplicationPhase::Acknowledged
            )
        })
    {
        return Err(Error::Conflict);
    }
    let sp = tx.savepoint()?;
    for file in saved.application.files {
        if file.phase == ApplicationPhase::Acknowledged {
            continue;
        }
        let current = targets.get(&file.path).ok_or(Error::Conflict)?;
        match (&file.target, current) {
            (ApplicationTarget::Disk, None) => {}
            (
                ApplicationTarget::Editor {
                    expected,
                    result_revision: None,
                },
                Some(current),
            ) => {
                if expected == current {
                    continue;
                }
                if expected.session_id == current.session_id
                    || current.session_id.is_nil()
                    || current.revision.is_nil()
                {
                    return Err(Error::Conflict);
                }
                if sp.execute(
                    "update agent_proposal_application_files
                     set expected_editor_session=?3,expected_editor_revision=?4,
                         updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')
                     where application_id=?1 and relative_path=?2 and phase='pending'",
                    params![
                        operation.to_string(),
                        file.path,
                        current.session_id.to_string(),
                        current.revision.to_string()
                    ],
                )? != 1
                {
                    return Err(Error::Conflict);
                }
            }
            _ => return Err(Error::Conflict),
        }
    }
    sp.commit()?;
    Ok(())
}
