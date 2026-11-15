create table paper_ai_reading_sessions (
    id text primary key,
    paper_id text not null references bibliographic_items(id) on delete restrict,
    attachment_ids_json text not null check(json_valid(attachment_ids_json)),
    parse_run_ids_json text not null check(json_valid(parse_run_ids_json)),
    provider_id text not null references llm_provider_configs(provider_id) on delete restrict,
    model_id text not null check(length(trim(model_id)) between 1 and 240),
    title text not null default '',
    status text not null check(status in ('active', 'failed', 'archived')),
    created_at text not null,
    updated_at text not null
);

create table paper_ai_reading_turns (
    id text primary key,
    session_id text not null references paper_ai_reading_sessions(id) on delete restrict,
    ordinal integer not null check(ordinal >= 0),
    user_message text not null check(length(trim(user_message)) > 0),
    assistant_message text not null default '',
    provider_id text not null,
    model_id text not null,
    status text not null check(status in ('running', 'succeeded', 'failed')),
    input_hash text check(input_hash is null or length(input_hash) = 64),
    output_hash text check(output_hash is null or length(output_hash) = 64),
    input_tokens integer check(input_tokens is null or input_tokens >= 0),
    output_tokens integer check(output_tokens is null or output_tokens >= 0),
    error_code text,
    error_message text,
    created_at text not null,
    finished_at text,
    unique(session_id, ordinal)
);

create table paper_ai_reading_turn_context_items (
    id text primary key,
    turn_id text not null references paper_ai_reading_turns(id) on delete restrict,
    context_item_id text not null,
    paper_id text not null,
    attachment_id text not null,
    content_sha256 text not null check(length(content_sha256) = 64),
    parse_run_id text not null references docling_parse_runs(id) on delete restrict,
    raw_ref text not null,
    item_kind text not null,
    page_index integer check(page_index is null or page_index >= 0),
    regions_json text not null check(json_valid(regions_json)),
    charspan_json text check(charspan_json is null or json_valid(charspan_json)),
    source_text_hash text check(source_text_hash is null or length(source_text_hash) = 64),
    image_content_sha256 text references content_objects(sha256) on delete restrict,
    created_at text not null,
    unique(turn_id, context_item_id)
);

create index paper_ai_reading_sessions_paper_idx
    on paper_ai_reading_sessions(paper_id, updated_at desc);

create index paper_ai_reading_turns_session_idx
    on paper_ai_reading_turns(session_id, ordinal);
