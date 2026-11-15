-- File-operation provenance belongs to the manuscript after admission. Chat
-- inputs may be deleted without cascading frozen objects or application journals.
-- Run with FK enforcement temporarily disabled by the migration runner, which
-- checks all foreign keys before commit and restores enforcement afterwards.
create temporary table agent_change_sets_0119 (
    id text primary key, conversation_id text not null, request_id text not null,
    turn_id text not null, context_snapshot_id text not null, manuscript_id text not null,
    lease_id text not null, manifest_hash text not null, inventory_hash text not null,
    schema_version integer not null, status text not null, created_at text not null,
    baseline_json text not null
);
insert into agent_change_sets_0119(id, conversation_id, request_id, turn_id, context_snapshot_id,
    manuscript_id, lease_id, manifest_hash, inventory_hash, schema_version, status, created_at, baseline_json)
select id, conversation_id, request_id, turn_id, context_snapshot_id, manuscript_id,
    lease_id, manifest_hash, inventory_hash, schema_version, status, created_at, baseline_json
from agent_change_sets;
drop table agent_change_sets;
create table agent_change_sets (
    id text primary key,
    conversation_id text not null,
    request_id text not null,
    turn_id text not null,
    context_snapshot_id text not null,
    manuscript_id text not null references manuscripts(id) on delete restrict,
    lease_id text not null,
    manifest_hash text not null check(length(manifest_hash) = 64),
    inventory_hash text not null check(length(inventory_hash) = 64),
    schema_version integer not null check(schema_version = 1),
    status text not null default 'pending' check(status in
        ('pending','rejected','applying','applied','conflict','recovery_required','undone')),
    created_at text not null default (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    baseline_json text not null default 'null' check(json_valid(baseline_json)),
    unique(conversation_id, request_id, inventory_hash)
);
insert into agent_change_sets(id, conversation_id, request_id, turn_id, context_snapshot_id,
    manuscript_id, lease_id, manifest_hash, inventory_hash, schema_version, status, created_at, baseline_json)
select id, conversation_id, request_id, turn_id, context_snapshot_id, manuscript_id,
    lease_id, manifest_hash, inventory_hash, schema_version, status, created_at, baseline_json
from agent_change_sets_0119;
drop table agent_change_sets_0119;
create index agent_change_sets_history_idx
    on agent_change_sets(conversation_id, manuscript_id, created_at desc, id desc);
create trigger agent_change_sets_immutable
before update of id, conversation_id, request_id, turn_id, context_snapshot_id,
    manuscript_id, lease_id, manifest_hash, inventory_hash, schema_version, created_at
on agent_change_sets
begin select raise(abort, 'immutable change set'); end;
create trigger agent_change_set_baseline_immutable
before update of baseline_json on agent_change_sets
begin select raise(abort, 'immutable proposal baseline'); end;

-- Replace the former submission FK for new publication. Retained provenance
-- never grants authority to publish or start a new apply after chat deletion.
create trigger agent_change_sets_submission_insert
before insert on agent_change_sets
when not exists(select 1 from agent_submissions s
    where s.conversation_id=new.conversation_id and s.request_id=new.request_id
      and s.turn_id=new.turn_id and s.context_snapshot_id=new.context_snapshot_id)
begin select raise(abort, 'missing proposal submission'); end;
create trigger agent_change_sets_submission_delete
before delete on agent_submissions
begin
    delete from agent_change_sets
    where conversation_id=old.conversation_id and request_id=old.request_id
      and not exists(select 1 from agent_proposal_applications a
          where a.change_set_id=agent_change_sets.id);
end;
create trigger agent_proposal_applications_deleted_origin
before insert on agent_proposal_applications
when new.direction='apply' and not exists(select 1 from agent_change_sets c
    join agent_submissions s on s.conversation_id=c.conversation_id and s.request_id=c.request_id
    where c.id=new.change_set_id and s.turn_id=c.turn_id and s.context_snapshot_id=c.context_snapshot_id)
begin select raise(abort, 'deleted application origin'); end;

-- A force-chat-delete marker must no longer bypass file recovery protections.
-- Fence the entire old purge path, including terminal journals and restorations.
create trigger agent_file_operations_preserved_on_chat_delete
before delete on agent_proposal_applications
when exists(select 1 from agent_change_sets c
    join agent_force_delete_markers m on m.conversation_id=c.conversation_id
    where c.id=old.change_set_id)
begin select raise(abort, 'file operation belongs to manuscript'); end;
create trigger agent_file_targets_preserved_on_chat_delete
before delete on agent_proposal_application_files
when exists(select 1 from agent_change_sets c
    join agent_force_delete_markers m on m.conversation_id=c.conversation_id
    where c.id=old.change_set_id)
begin select raise(abort, 'file target belongs to manuscript'); end;
create trigger agent_file_restorations_preserved_on_chat_delete
before delete on agent_editor_restorations
when exists(select 1 from agent_proposal_applications a
    join agent_change_sets c on c.id=a.change_set_id
    join agent_force_delete_markers m on m.conversation_id=c.conversation_id
    where a.id=old.application_id)
begin select raise(abort, 'file restoration belongs to manuscript'); end;
create trigger agent_file_undos_preserved_on_chat_delete
before delete on agent_proposal_undos
when exists(select 1 from agent_proposal_applications a
    join agent_change_sets c on c.id=a.change_set_id
    join agent_force_delete_markers m on m.conversation_id=c.conversation_id
    where a.id=old.operation_id)
begin select raise(abort, 'file undo belongs to manuscript'); end;
create trigger agent_file_undo_sources_preserved_on_chat_delete
before delete on agent_proposal_undo_sources
when exists(select 1 from agent_proposal_applications a
    join agent_change_sets c on c.id=a.change_set_id
    join agent_force_delete_markers m on m.conversation_id=c.conversation_id
    where a.id=old.operation_id)
begin select raise(abort, 'file undo source belongs to manuscript'); end;
