-- Reconciled Undo may renew only Pending owners after whole-plan checkpoint
-- preflight. Confirmed receipts, frozen sources and restoration intents stay sealed.
drop trigger agent_proposal_application_files_target_rebind;
create trigger agent_proposal_application_files_target_rebind
before update of target_kind, expected_editor_session, expected_editor_revision
    on agent_proposal_application_files
when (new.target_kind is not old.target_kind or
    new.expected_editor_session is not old.expected_editor_session or
    new.expected_editor_revision is not old.expected_editor_revision) and (
    new.target_kind!='editor' or old.phase!='pending' or new.phase!='pending' or
    old.result_editor_revision is not null or new.result_editor_revision is not null or
    not exists(select 1 from agent_proposal_applications a
        where a.id=old.application_id and (
            (a.direction='apply' and a.status='recovery_required'
                and not exists(select 1 from agent_proposal_application_files f
                    where f.application_id=a.id and f.phase!='pending')) or
            (a.direction='undo' and a.status in ('prepared','recovery_required')
                and old.target_kind='editor'
                and new.expected_editor_session is not old.expected_editor_session
                and not exists(select 1 from agent_proposal_application_files f
                    where f.application_id=a.id and f.phase not in ('pending','acknowledged'))
                and exists(select 1 from agent_proposal_undos u
                    join agent_proposal_applications original on original.id=u.original_operation_id
                    where u.operation_id=a.id and original.direction='apply'
                        and original.status='applied' and original.change_set_id=a.change_set_id)))) or
    not exists(select 1 from agent_change_set_files c
        where c.change_set_id=old.change_set_id and c.relative_path=old.relative_path
        and c.before_sha256 is not null and c.after_sha256 is not null
        and c.before_executable=c.after_executable) or
    exists(select 1 from agent_editor_restorations r where r.application_id=old.application_id))
begin select raise(abort,'application target cannot be rebound'); end;
