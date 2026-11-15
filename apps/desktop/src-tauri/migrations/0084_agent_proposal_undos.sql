-- Undo owns a fresh baseline and target journal; the original apply stays terminal.
create table agent_proposal_undos (
    operation_id text primary key references agent_proposal_applications(id) on delete restrict,
    original_operation_id text not null references agent_proposal_applications(id) on delete restrict,
    schema_version integer not null default 1 check(schema_version=1),
    check(operation_id!=original_operation_id)
);
create index agent_proposal_undos_original_idx on agent_proposal_undos(original_operation_id,operation_id);
create table agent_proposal_undo_sources (
    operation_id text not null references agent_proposal_undos(operation_id) on delete restrict,
    relative_path text not null,
    source_json text check(source_json is null or
        (length(source_json)<=16384 and json_valid(source_json))),
    primary key(operation_id,relative_path),
    foreign key(operation_id,relative_path)
        references agent_proposal_application_files(application_id,relative_path) on delete restrict
);
create trigger agent_proposal_undos_admission
before insert on agent_proposal_undos when not exists (
    select 1 from agent_proposal_applications u
    join agent_proposal_applications a on a.id=new.original_operation_id
    join agent_change_sets c on c.id=a.change_set_id
    where u.id=new.operation_id and u.direction='undo' and u.status='prepared'
      and a.direction='apply' and a.status='applied' and c.status='applied'
      and u.change_set_id=a.change_set_id
      and not exists(select 1 from agent_proposal_application_files f
          where f.application_id=a.id and f.phase!='acknowledged')
)
begin select raise(abort,'invalid undo origin'); end;
create trigger agent_proposal_undos_immutable before update on agent_proposal_undos
begin select raise(abort,'immutable undo origin'); end;
create trigger agent_proposal_undo_sources_immutable before update on agent_proposal_undo_sources
begin select raise(abort,'immutable undo source'); end;
create trigger agent_proposal_undo_sources_admission
before insert on agent_proposal_undo_sources when not exists (
    select 1 from agent_proposal_applications where id=new.operation_id and status='prepared' and direction='undo'
)
begin select raise(abort,'undo plan is sealed'); end;
create trigger agent_proposal_undo_sources_active_delete
before delete on agent_proposal_undo_sources when exists (
    select 1 from agent_proposal_applications where id=old.operation_id and status in ('prepared','applying','recovery_required')
)
begin select raise(abort,'unresolved undo source'); end;
create trigger agent_proposal_undos_active_delete
before delete on agent_proposal_undos when exists (
    select 1 from agent_proposal_applications where id=old.operation_id and status in ('prepared','applying','recovery_required')
)
begin select raise(abort,'unresolved undo origin'); end;
create trigger agent_proposal_undos_complete_plan
before update of status on agent_proposal_applications
when new.direction='undo' and new.status in ('applying','applied') and (
    not exists(select 1 from agent_proposal_undos where operation_id=new.id) or
    (select count(*) from agent_proposal_undo_sources where operation_id=new.id) !=
    (select count(*) from agent_proposal_application_files where application_id=new.id)
)
begin select raise(abort,'incomplete undo baseline'); end;
