create table paper_ai_reading_turns_next (
    id text primary key,
    session_id text not null references paper_ai_reading_sessions(id) on delete restrict,
    ordinal integer not null check(ordinal >= 0),
    user_message text not null check(length(trim(user_message)) > 0),
    assistant_message text not null default '',
    provider_id text not null,
    model_id text not null,
    status text not null check(status in ('running', 'succeeded', 'failed', 'canceled')),
    input_hash text check(input_hash is null or length(input_hash) = 64),
    output_hash text check(output_hash is null or length(output_hash) = 64),
    input_tokens integer check(input_tokens is null or input_tokens >= 0),
    output_tokens integer check(output_tokens is null or output_tokens >= 0),
    error_code text,
    error_message text,
    created_at text not null,
    finished_at text,
    raw_request_json text check(raw_request_json is null or json_valid(raw_request_json)),
    raw_response_json text check(raw_response_json is null or json_valid(raw_response_json)),
    reasoning_effort text,
    context_window_tokens integer
        check(context_window_tokens is null or context_window_tokens > 0),
    context_window_source text
        check(context_window_source is null or context_window_source in (
            'provider_runtime', 'provider_catalog', 'models_dev'
        )),
    unique(session_id, ordinal)
);

insert into paper_ai_reading_turns_next (
    id, session_id, ordinal, user_message, assistant_message, provider_id,
    model_id, status, input_hash, output_hash, input_tokens, output_tokens,
    error_code, error_message, created_at, finished_at, raw_request_json,
    raw_response_json, reasoning_effort, context_window_tokens,
    context_window_source
)
select
    id, session_id, ordinal, user_message, assistant_message, provider_id,
    model_id, status, input_hash, output_hash, input_tokens, output_tokens,
    error_code, error_message, created_at, finished_at, raw_request_json,
    raw_response_json, reasoning_effort, context_window_tokens,
    context_window_source
from paper_ai_reading_turns;

drop table paper_ai_reading_turns;
alter table paper_ai_reading_turns_next rename to paper_ai_reading_turns;

create index paper_ai_reading_turns_session_idx
    on paper_ai_reading_turns(session_id, ordinal);
