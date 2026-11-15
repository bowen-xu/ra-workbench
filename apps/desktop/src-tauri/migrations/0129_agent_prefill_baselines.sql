create table agent_prefill_baselines (
    conversation_id text not null references agent_conversations(id) on delete cascade,
    model_json text not null check (json_valid(model_json) and length(model_json) <= 4096),
    window_key text not null,
    rollback_epoch integer not null check (rollback_epoch >= 0),
    baseline_json text not null check (json_valid(baseline_json) and length(baseline_json) <= 2048),
    primary key (conversation_id, model_json, window_key, rollback_epoch)
);
