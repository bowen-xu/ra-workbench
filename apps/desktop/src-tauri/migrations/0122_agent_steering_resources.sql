-- Structured Steer metadata survives alongside the immutable UserMessage item.
-- Old text-only rows continue to mean no resources.
create table agent_steering_resources (
    input_id text primary key references agent_steering_inputs(input_id) on delete cascade,
    input_json text not null,
    skill_instructions_json text not null
);
create table agent_turn_resource_grants (
    turn_id text primary key references agent_submissions(turn_id) on delete cascade,
    grants_json text not null
);
