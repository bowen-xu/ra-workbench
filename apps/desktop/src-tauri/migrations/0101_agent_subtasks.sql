-- Child-task lifecycle records. The parent conversation remains authoritative;
-- this table only records bounded scheduling and terminal reconciliation.
create table agent_subtasks (
    child_conversation_id text primary key,
    parent_conversation_id text not null,
    parent_context_snapshot_id text not null,
    child_context_snapshot_id text not null,
    depth integer not null check(depth >= 1 and depth <= 2),
    model_calls integer not null check(model_calls >= 0),
    tool_calls integer not null check(tool_calls >= 0),
    status text not null check(status in ('running','completed','canceled','failed','recovery_required')),
    created_at text not null default (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    finished_at text
);
create index agent_subtasks_parent_idx on agent_subtasks(parent_conversation_id, created_at desc);
