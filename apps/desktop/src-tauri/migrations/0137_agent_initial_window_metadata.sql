-- Full-context metadata is a trusted snapshot, separate from chat output.
-- Append rather than overwrite so rollback/fork can select a surviving source.
create table agent_initial_window_metadata (
    conversation_id text not null references agent_conversations(id) on delete cascade,
    window_id text not null references agent_initial_context_windows(window_id) on delete cascade,
    boundary_seq integer not null check (boundary_seq > 0),
    boundary_ordinal integer generated always as (boundary_seq - 1) stored,
    turn_id text not null,
    body text not null,
    primary key (conversation_id, window_id, boundary_seq),
    foreign key (conversation_id, boundary_ordinal) references agent_items(conversation_id, ordinal) on delete cascade
);
create trigger agent_initial_window_metadata_immutable
before update on agent_initial_window_metadata
begin
    select raise(abort, 'initial window metadata is immutable');
end;
