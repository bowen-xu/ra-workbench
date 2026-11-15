-- Keep frozen proposal and application identities while allowing identical
-- inventories from distinct execution leases. Foreign keys are disabled only
-- for the rebuild and checked by the migration runner before commit.
create temporary table agent_change_sets_0229 as
select id, conversation_id, request_id, turn_id, context_snapshot_id,
    manuscript_id, lease_id, manifest_hash, inventory_hash, schema_version,
    status, created_at, baseline_json from agent_change_sets;
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
    unique(conversation_id, request_id, lease_id, inventory_hash)
);
insert into agent_change_sets(id, conversation_id, request_id, turn_id, context_snapshot_id,
    manuscript_id, lease_id, manifest_hash, inventory_hash, schema_version, status, created_at, baseline_json)
select id, conversation_id, request_id, turn_id, context_snapshot_id, manuscript_id,
    lease_id, manifest_hash, inventory_hash, schema_version, status, created_at, baseline_json
from agent_change_sets_0229;
drop table agent_change_sets_0229;
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
create trigger agent_change_sets_submission_insert
before insert on agent_change_sets
when not exists(select 1 from agent_submissions s
    where s.conversation_id=new.conversation_id and s.request_id=new.request_id
      and s.turn_id=new.turn_id and s.context_snapshot_id=new.context_snapshot_id)
begin select raise(abort, 'missing proposal submission'); end;
