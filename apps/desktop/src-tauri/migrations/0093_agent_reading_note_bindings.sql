-- Reading Note IDs belong to note_blocks, independently of Note Page IDs.
-- Rebuild only association tables; retain every history row and lineage field.
create table agent_conversation_bindings_next (
    conversation_id text not null references agent_conversations(id) on delete cascade,
    resource_kind text not null check (resource_kind in ('manuscript', 'paper', 'note', 'readingNote')),
    resource_id text not null check (length(resource_id) between 1 and 256),
    created_at text not null default (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    primary key (conversation_id, resource_kind, resource_id)
);
insert into agent_conversation_bindings_next select * from agent_conversation_bindings;
drop table agent_conversation_bindings;
alter table agent_conversation_bindings_next rename to agent_conversation_bindings;
create index agent_conversation_bindings_resource_idx
    on agent_conversation_bindings(resource_kind, resource_id, conversation_id);

create table agent_conversation_forks_next (
    child_id text primary key not null references agent_conversations(id) on delete cascade,
    parent_id text not null references agent_conversations(id) on delete restrict,
    through_turn_id text not null,
    parent_end_seq integer not null check (parent_end_seq > 0),
    resource_kind text not null check (resource_kind in ('manuscript', 'paper', 'note', 'readingNote')),
    resource_id text not null,
    created_at text not null default (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    check (child_id != parent_id)
);
insert into agent_conversation_forks_next select * from agent_conversation_forks;
drop table agent_conversation_forks;
alter table agent_conversation_forks_next rename to agent_conversation_forks;
create index agent_conversation_forks_parent on agent_conversation_forks(parent_id);
