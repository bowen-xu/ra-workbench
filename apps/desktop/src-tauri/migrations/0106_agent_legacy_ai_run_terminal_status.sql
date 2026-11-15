alter table agent_legacy_ai_run_bindings add column terminal_status text;

create index agent_legacy_ai_run_bindings_terminal_idx
    on agent_legacy_ai_run_bindings(terminal_status, created_at desc);
