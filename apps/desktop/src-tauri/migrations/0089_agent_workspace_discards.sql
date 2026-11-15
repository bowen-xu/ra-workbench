-- Freeze each proposal's baseline before execution bindings can be released.
-- Missing legacy metadata stays explicitly unreadable; never fabricate a baseline.
alter table agent_change_sets add column baseline_json text not null default 'null'
    check(json_valid(baseline_json));
update agent_change_sets set baseline_json = coalesce((
    select w.manifest_json from agent_execution_workspaces w
    where w.conversation_id = agent_change_sets.conversation_id
      and w.request_id = agent_change_sets.request_id
      and w.lease_id = agent_change_sets.lease_id
      and w.manifest_hash = agent_change_sets.manifest_hash
), 'null');
create trigger agent_change_set_baseline_immutable
before update of baseline_json on agent_change_sets
begin
    select raise(abort, 'immutable proposal baseline');
end;

-- Independent cleanup journal survives conversation deletion. No source bytes.
create table agent_workspace_discards (
    operation_id text primary key,
    conversation_id text not null,
    request_id text not null,
    turn_id text not null,
    context_snapshot_id text not null,
    manuscript_id text not null,
    lease_id text not null unique,
    environment_id text not null,
    manifest_hash text not null check(length(manifest_hash) = 64),
    inventory_hash text not null check(length(inventory_hash) = 64),
    state text not null default 'pending' check(state in ('pending','complete')),
    created_at text not null default (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    completed_at text,
    unique(conversation_id, request_id),
    check((state = 'pending' and completed_at is null) or
          (state = 'complete' and completed_at is not null))
);
create trigger agent_workspace_discard_identity_immutable
before update of operation_id, conversation_id, request_id, turn_id, context_snapshot_id,
    manuscript_id, lease_id, environment_id, manifest_hash, inventory_hash, created_at
on agent_workspace_discards
begin
    select raise(abort, 'immutable workspace discard');
end;
create trigger agent_workspace_discard_terminal
before update of state, completed_at on agent_workspace_discards
when old.state = 'complete' or new.state != 'complete'
begin
    select raise(abort, 'terminal workspace discard');
end;
create trigger agent_workspace_discard_retained
before delete on agent_workspace_discards
begin
    select raise(abort, 'retained workspace discard');
end;
create trigger agent_workspace_discard_fences_preparation
before insert on agent_execution_workspaces
when exists(select 1 from agent_workspace_discards d
    where (d.conversation_id = new.conversation_id and d.request_id = new.request_id)
       or d.lease_id = new.lease_id)
begin
    select raise(abort, 'discarded workspace');
end;
create trigger agent_workspace_discard_fences_authorization
before update of state on agent_execution_workspaces
when new.state != 'revoked' and exists(select 1 from agent_workspace_discards d
    where d.conversation_id = old.conversation_id and d.request_id = old.request_id)
begin
    select raise(abort, 'discarded workspace authority');
end;
create trigger agent_workspace_discard_fences_identity
before update of conversation_id, request_id, turn_id, context_snapshot_id, manuscript_id,
    lease_id, manifest_hash, manifest_json on agent_execution_workspaces
when exists(select 1 from agent_workspace_discards d
    where d.conversation_id = old.conversation_id and d.request_id = old.request_id)
begin
    select raise(abort, 'discarding workspace identity');
end;
create trigger agent_workspace_discard_preserves_pending_binding
before delete on agent_execution_workspaces
when exists(select 1 from agent_workspace_discards d
    where d.conversation_id = old.conversation_id and d.request_id = old.request_id
      and d.state = 'pending')
begin
    select raise(abort, 'pending workspace discard');
end;
