-- Version 3 carries an explicit host authority boundary before wire role mapping.
create table agent_responses_checkpoints_v3 (
    turn_id text primary key not null references agent_submissions(turn_id) on delete cascade,
    schema_version integer not null check (schema_version in (1, 2, 3)),
    checkpoint_json text not null check (json_valid(checkpoint_json))
        check (length(cast(checkpoint_json as blob)) <= 33554432),
    updated_at text not null default (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
insert into agent_responses_checkpoints_v3
    (turn_id, schema_version, checkpoint_json, updated_at)
select turn_id, schema_version, checkpoint_json, updated_at from agent_responses_checkpoints;
drop table agent_responses_checkpoints;
alter table agent_responses_checkpoints_v3 rename to agent_responses_checkpoints;
