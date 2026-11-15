create table agent_steering_gates (
    turn_id text primary key references agent_submissions(turn_id) on delete cascade,
    sealed integer not null default 0 check (sealed in (0, 1))
);
create table agent_steering_inputs (
    input_id text primary key,
    turn_id text not null references agent_steering_gates(turn_id) on delete cascade,
    ordinal integer not null check (ordinal >= 0),
    text text not null check (length(cast(text as blob)) between 1 and 65536),
    unique (turn_id, ordinal)
);
