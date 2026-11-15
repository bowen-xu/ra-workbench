create table agent_input_queue_previews (
    input_id text primary key references agent_input_queue_identities(input_id) on delete cascade,
    conversation_id text not null,
    request_id text not null,
    expected_revision integer not null check (expected_revision >= 0),
    result_json text not null check (json_valid(result_json)),
    unique (conversation_id, request_id),
    foreign key (conversation_id, request_id)
        references agent_submissions(conversation_id, request_id) on delete cascade
);
