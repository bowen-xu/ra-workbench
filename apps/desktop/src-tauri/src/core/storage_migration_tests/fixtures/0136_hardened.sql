-- Trusted developer fragments are distinct from user/assistant chat events.
-- Journal evidence is append-only; rollback filtering belongs to its projection.
create table agent_context_fragments (
    conversation_id text not null references agent_conversations(id) on delete cascade,
    window_id text not null,
    boundary_seq integer not null check (boundary_seq > 0),
    boundary_ordinal integer generated always as (boundary_seq - 1) stored,
    turn_id text not null,
    anchor_item_id text not null,
    fragments_json text not null check (json_valid(fragments_json) and json_type(fragments_json) = 'array'),
    primary key (conversation_id, window_id, boundary_seq),
    foreign key (conversation_id, boundary_ordinal) references agent_items(conversation_id, ordinal) on delete cascade
);
create trigger agent_context_fragments_immutable
before update on agent_context_fragments
begin
    select raise(abort, 'context fragments are immutable');
end;
