alter table agent_legacy_ai_run_bindings add column candidate_id text references ai_candidates(id) on delete set null;

create index agent_legacy_ai_run_bindings_candidate_idx
    on agent_legacy_ai_run_bindings(candidate_id);
