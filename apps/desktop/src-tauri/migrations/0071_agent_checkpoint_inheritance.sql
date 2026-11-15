-- Version 2 records whether a checkpoint is inherited rather than this turn's response.
create table agent_responses_checkpoints_v2 (
    turn_id text primary key not null references agent_submissions(turn_id) on delete cascade,
    schema_version integer not null check (schema_version in (1, 2)),
    checkpoint_json text not null check (json_valid(checkpoint_json))
        check (length(cast(checkpoint_json as blob)) <= 33554432),
    updated_at text not null default (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
insert into agent_responses_checkpoints_v2
    (turn_id, schema_version, checkpoint_json, updated_at)
select turn_id, schema_version, checkpoint_json, updated_at from agent_responses_checkpoints;
drop table agent_responses_checkpoints;
alter table agent_responses_checkpoints_v2 rename to agent_responses_checkpoints;
