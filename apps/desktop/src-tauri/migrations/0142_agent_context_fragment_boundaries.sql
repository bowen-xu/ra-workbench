-- v136 was strengthened in place during development. Preserve both recorded
-- checksums and install the boundary FK forward without discarding fragments.
create table agent_context_fragments_upgraded (
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
insert into agent_context_fragments_upgraded
    (conversation_id, window_id, boundary_seq, turn_id, anchor_item_id, fragments_json)
select conversation_id, window_id, boundary_seq, turn_id, anchor_item_id, fragments_json
from agent_context_fragments;
drop table agent_context_fragments;
alter table agent_context_fragments_upgraded rename to agent_context_fragments;
create trigger agent_context_fragments_immutable
before update on agent_context_fragments
begin
    select raise(abort, 'context fragments are immutable');
end;
