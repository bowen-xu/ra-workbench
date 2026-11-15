-- Inherited history retains its original source identities. Retain the parent
-- and its provenance until branch-aware deletion/GC explicitly releases them.
create table agent_conversation_forks (
    child_id text primary key not null references agent_conversations(id) on delete cascade,
    parent_id text not null references agent_conversations(id) on delete restrict,
    through_turn_id text not null,
    parent_end_seq integer not null check (parent_end_seq > 0),
    resource_kind text not null check (resource_kind in ('manuscript', 'paper', 'note')),
    resource_id text not null,
    created_at text not null default (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    check (child_id != parent_id)
);
create index agent_conversation_forks_parent on agent_conversation_forks(parent_id);
