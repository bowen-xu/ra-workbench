-- Preserve legacy paper-reading metadata beside the normalized shared journal.
-- The shared event stream remains the history projection; these tables retain
-- fields that have no lossless event equivalent (bootstrap/debug/attachment
-- provenance) until the old UI and exports no longer need them.
create table agent_paper_reading_legacy_sessions (
    conversation_id text primary key references agent_conversations(id) on delete cascade,
    paper_id text not null references bibliographic_items(id) on delete cascade,
    legacy_session_id text not null,
    attachment_ids_json text not null check(json_valid(attachment_ids_json) and json_type(attachment_ids_json) = 'array'),
    parse_run_ids_json text not null check(json_valid(parse_run_ids_json) and json_type(parse_run_ids_json) = 'array'),
    bootstrap_kind text not null check(bootstrap_kind in ('fresh_paper', 'structured_history')),
    source_workflow_id text,
    source_workflow_revision integer check(source_workflow_revision is null or source_workflow_revision >= 1),
    bootstrap_messages_json text not null check(json_valid(bootstrap_messages_json) and json_type(bootstrap_messages_json) = 'array'),
    bootstrap_hash text check(bootstrap_hash is null or length(bootstrap_hash) = 64),
    status text not null,
    title text not null,
    created_at text not null,
    updated_at text not null,
    unique(paper_id, legacy_session_id)
);

create index agent_paper_reading_legacy_sessions_paper_idx
    on agent_paper_reading_legacy_sessions(paper_id, updated_at desc);

create table agent_paper_reading_legacy_turns (
    conversation_id text not null references agent_conversations(id) on delete cascade,
    legacy_turn_id text not null,
    request_id text not null,
    turn_id text not null,
    ordinal integer not null check(ordinal >= 0),
    provider_id text not null,
    model_id text not null,
    reasoning_effort text,
    status text not null,
    input_hash text check(input_hash is null or length(input_hash) = 64),
    output_hash text check(output_hash is null or length(output_hash) = 64),
    input_tokens integer check(input_tokens is null or input_tokens >= 0),
    output_tokens integer check(output_tokens is null or output_tokens >= 0),
    context_window_tokens integer check(context_window_tokens is null or context_window_tokens > 0),
    context_window_source text,
    error_code text,
    error_message text,
    messages_json text not null check(json_valid(messages_json) and json_type(messages_json) = 'array'),
    raw_request_json text check(raw_request_json is null or json_valid(raw_request_json)),
    raw_response_json text check(raw_response_json is null or json_valid(raw_response_json)),
    created_at text not null,
    finished_at text,
    primary key(conversation_id, legacy_turn_id),
    unique(conversation_id, request_id),
    unique(conversation_id, turn_id)
);

create index agent_paper_reading_legacy_turns_conversation_idx
    on agent_paper_reading_legacy_turns(conversation_id, ordinal);
