-- Frozen proposal provenance and plan membership never change. Only an
-- undispatched apply may renew live editor CAS identities during recovery;
-- native admission must first validate the whole plan's bytes and disk hashes.
drop trigger agent_proposal_application_files_identity_immutable;
create trigger agent_proposal_application_files_identity_immutable
before update of application_id, change_set_id, relative_path
    on agent_proposal_application_files
begin select raise(abort,'immutable application target identity'); end;

create trigger agent_proposal_application_files_target_rebind
before update of target_kind, expected_editor_session, expected_editor_revision
    on agent_proposal_application_files
when (new.target_kind is not old.target_kind or
    new.expected_editor_session is not old.expected_editor_session or
    new.expected_editor_revision is not old.expected_editor_revision) and (
    new.target_kind!='editor' or old.phase!='pending' or new.phase!='pending' or
    old.result_editor_revision is not null or new.result_editor_revision is not null or
    not exists(select 1 from agent_proposal_applications a
        where a.id=old.application_id and a.direction='apply' and a.status='recovery_required') or
    exists(select 1 from agent_proposal_application_files f
        where f.application_id=old.application_id and f.phase!='pending') or
    exists(select 1 from agent_editor_restorations r where r.application_id=old.application_id))
begin select raise(abort,'application target cannot be rebound'); end;
