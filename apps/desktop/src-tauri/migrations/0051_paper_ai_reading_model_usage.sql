alter table paper_ai_reading_sessions
    add column reasoning_effort text;

alter table paper_ai_reading_turns
    add column reasoning_effort text;

alter table paper_ai_reading_turns
    add column context_window_tokens integer
        check(context_window_tokens is null or context_window_tokens > 0);

alter table paper_ai_reading_turns
    add column context_window_source text
        check(context_window_source is null or context_window_source in (
            'provider_runtime', 'provider_catalog', 'models_dev'
        ));
