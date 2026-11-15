-- Native provider continuation; not user-visible history or credentials.
create table agent_responses_checkpoints (
    turn_id text primary key not null references agent_submissions(turn_id) on delete cascade,
    schema_version integer not null check (schema_version = 1),
    checkpoint_json text not null check (json_valid(checkpoint_json))
        check (length(cast(checkpoint_json as blob)) <= 33554432),
    updated_at text not null default (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
