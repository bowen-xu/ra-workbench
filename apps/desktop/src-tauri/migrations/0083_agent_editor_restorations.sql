-- A restore transaction has its own identity, distinct from the original edit.
create table agent_editor_restorations (
    application_id text not null,
    relative_path text not null,
    restore_id text not null unique check(length(restore_id)=36),
    editor_session text not null check(length(editor_session)=36),
    expected_revision text not null check(length(expected_revision)=36),
    result_revision text check(result_revision is null or
        (length(result_revision)=36 and result_revision!=expected_revision)),
    created_at text not null default (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    primary key(application_id, relative_path),
    foreign key(application_id, relative_path)
        references agent_proposal_application_files(application_id, relative_path) on delete restrict
);
create trigger agent_editor_restorations_admission
before insert on agent_editor_restorations
when new.result_revision is not null or not exists(
    select 1 from agent_proposal_application_files f
    join agent_proposal_applications a on a.id=f.application_id
    where f.application_id=new.application_id and f.relative_path=new.relative_path
      and a.status='recovery_required' and f.target_kind='editor' and f.phase='acknowledged'
      and f.expected_editor_session=new.editor_session and f.result_editor_revision=new.expected_revision
)
begin select raise(abort,'editor restoration requires confirmed edit and recovery owner'); end;
create trigger agent_editor_restorations_identity
before update of application_id, relative_path, restore_id, editor_session, expected_revision, created_at
on agent_editor_restorations
begin select raise(abort,'immutable editor restoration'); end;
create trigger agent_editor_restorations_receipt
before update of result_revision on agent_editor_restorations
when old.result_revision is not null or new.result_revision is null or not exists(
    select 1 from agent_proposal_applications where id=old.application_id and status='recovery_required'
)
begin select raise(abort,'invalid editor restoration receipt'); end;
