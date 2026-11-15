-- Durable application intent. Frozen proposal files retain the actual before/after
-- objects; restrict deletion while a journal refers to them, including after ACK.
create table agent_proposal_applications (
    id text primary key,
    change_set_id text not null references agent_change_sets(id) on delete restrict,
    direction text not null check(direction in ('apply','undo')),
    status text not null default 'prepared' check(status in
        ('prepared','applying','applied','recovery_required','rolled_back')),
    created_at text not null default (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at text not null default (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    unique(id, change_set_id)
);
create unique index agent_proposal_applications_active_idx
    on agent_proposal_applications(change_set_id)
    where status in ('prepared','applying','recovery_required');
create index agent_proposal_applications_recovery_idx
    on agent_proposal_applications(status, updated_at, id);

create table agent_proposal_application_files (
    application_id text not null,
    change_set_id text not null,
    relative_path text not null,
    target_kind text not null check(target_kind in ('disk','editor')),
    expected_editor_session text,
    expected_editor_revision text,
    phase text not null default 'pending' check(phase in
        ('pending','dispatched','acknowledged','restored','conflict')),
    result_editor_revision text,
    updated_at text not null default (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    primary key(application_id, relative_path),
    foreign key(application_id, change_set_id)
        references agent_proposal_applications(id, change_set_id) on delete cascade,
    foreign key(change_set_id, relative_path)
        references agent_change_set_files(change_set_id, relative_path) on delete restrict,
    check((target_kind='disk' and expected_editor_session is null and
        expected_editor_revision is null and result_editor_revision is null) or
        (target_kind='editor' and length(expected_editor_session)=36 and
        length(expected_editor_revision)=36 and expected_editor_session is not null and
        expected_editor_revision is not null)),
    check(result_editor_revision is null or length(result_editor_revision)=36),
    check(target_kind!='editor' or phase!='acknowledged' or result_editor_revision is not null)
);

create trigger agent_proposal_applications_identity_immutable
before update of id, change_set_id, direction, created_at on agent_proposal_applications
begin select raise(abort,'immutable proposal application'); end;
create trigger agent_proposal_application_files_identity_immutable
before update of application_id, change_set_id, relative_path, target_kind,
    expected_editor_session, expected_editor_revision on agent_proposal_application_files
begin select raise(abort,'immutable application target'); end;

-- A plan is complete before dispatch starts. Never silently append a new target
-- after an earlier file may already have been written.
create trigger agent_proposal_application_files_prepared_insert
before insert on agent_proposal_application_files
when not exists(select 1 from agent_proposal_applications
    where id=new.application_id and status='prepared')
begin select raise(abort,'application plan is sealed'); end;

-- Active/uncertain intent must be reconciled rather than removed by cleanup.
create trigger agent_proposal_applications_active_delete
before delete on agent_proposal_applications
when old.status in ('prepared','applying','recovery_required')
begin select raise(abort,'unresolved proposal application'); end;
create trigger agent_proposal_application_files_active_delete
before delete on agent_proposal_application_files
when exists(select 1 from agent_proposal_applications where id=old.application_id
    and status in ('prepared','applying','recovery_required'))
begin select raise(abort,'unresolved application target'); end;

create trigger agent_proposal_applications_transition
before update of status on agent_proposal_applications
when new.status != old.status and not (
    (old.status='prepared' and new.status in ('applying','recovery_required','rolled_back')) or
    (old.status='applying' and new.status in ('applied','recovery_required','rolled_back')) or
    (old.status='recovery_required' and new.status in ('applying','applied','rolled_back')))
begin select raise(abort,'invalid application transition'); end;
create trigger agent_proposal_applications_complete_plan
before update of status on agent_proposal_applications
when new.status in ('applying','applied') and (
    (select count(*) from agent_proposal_application_files where application_id=new.id)=0 or
    (select count(*) from agent_proposal_application_files where application_id=new.id) !=
    (select count(*) from agent_change_set_files where change_set_id=new.change_set_id))
begin select raise(abort,'incomplete application plan'); end;
create trigger agent_proposal_applications_terminal_files
before update of status on agent_proposal_applications
when (new.status='applied' and exists(select 1 from agent_proposal_application_files
    where application_id=new.id and phase!='acknowledged')) or
    (new.status='rolled_back' and exists(select 1 from agent_proposal_application_files
    where application_id=new.id and phase not in ('pending','restored')))
begin select raise(abort,'unresolved application files'); end;
create trigger agent_proposal_application_files_transition
before update of phase on agent_proposal_application_files
when new.phase != old.phase and not (
    (old.phase='pending' and new.phase in ('dispatched','conflict','restored')) or
    (old.phase='dispatched' and new.phase in ('acknowledged','conflict','restored')) or
    (old.phase='acknowledged' and new.phase in ('conflict','restored')) or
    (old.phase='conflict' and new.phase in ('dispatched','acknowledged','restored')))
begin select raise(abort,'invalid application file transition'); end;
create trigger agent_proposal_applications_initial_state
before insert on agent_proposal_applications when new.status!='prepared'
begin select raise(abort,'application must start prepared'); end;
create trigger agent_proposal_application_files_initial_state
before insert on agent_proposal_application_files
when new.phase!='pending' or new.result_editor_revision is not null
begin select raise(abort,'application target must start pending'); end;
create trigger agent_proposal_application_files_execution_owner
before update of phase, result_editor_revision on agent_proposal_application_files
when not exists(select 1 from agent_proposal_applications where id=old.application_id
    and status in ('applying','recovery_required'))
begin select raise(abort,'application is not executing or reconciling'); end;
