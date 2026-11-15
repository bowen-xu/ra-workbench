-- Provider replay state is valid only for the installed model history window.
-- Keep earlier state rows for history; a replacement must not reuse their prefix.
create table agent_responses_window_bindings (
    turn_id text primary key references agent_submissions(turn_id) on delete cascade,
    window_id text not null references agent_context_checkpoints(id) on delete cascade
);
create index agent_responses_window_bindings_window_idx
    on agent_responses_window_bindings(window_id);
