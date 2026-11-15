create table ai_provider_settings (
    provider_id text primary key check(provider_id in ('fixture-generation', 'deepseek', 'qwen-embedding')),
    provider_kind text not null check(provider_kind in ('generation', 'embedding')),
    enabled integer not null default 0 check(enabled in (0, 1)),
    base_url text not null default '',
    region text not null default '',
    model_id text not null default '',
    thinking_mode text not null default 'disabled'
        check(thinking_mode in ('disabled', 'enabled')),
    timeout_ms integer not null default 60000 check(timeout_ms between 1000 and 300000),
    updated_at text not null
);

insert into ai_provider_settings
    (provider_id, provider_kind, enabled, base_url, region, model_id, thinking_mode,
     timeout_ms, updated_at)
values
    ('fixture-generation', 'generation', 1, '', 'local', 'fixture-v1', 'disabled',
     5000, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('deepseek', 'generation', 0, 'https://api.deepseek.com', '', '', 'disabled',
     60000, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('qwen-embedding', 'embedding', 0, '', '', '', 'disabled',
     60000, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));

create table ai_runs (
    id text primary key,
    task_kind text not null check(task_kind in (
        'provider_diagnostic', 'selection_explanation', 'paper_overview'
    )),
    status text not null check(status in (
        'draft', 'queued', 'resolving_scope', 'retrieving', 'extracting_evidence',
        'generating', 'validating', 'needs_review', 'partial', 'failed', 'canceled'
    )),
    provider_id text not null references ai_provider_settings(provider_id) on delete restrict,
    model_id text not null,
    pipeline_version text not null,
    prompt_version text not null,
    output_schema_version integer not null check(output_schema_version >= 1),
    user_goal text not null default '' check(length(user_goal) <= 2000),
    scope_hash text not null check(length(scope_hash) = 64),
    input_item_count integer not null default 0 check(input_item_count >= 0),
    max_input_tokens integer not null check(max_input_tokens between 1 and 1000000),
    max_output_tokens integer not null check(max_output_tokens between 1 and 100000),
    timeout_ms integer not null check(timeout_ms between 1000 and 300000),
    cloud_consent_version text,
    cloud_consented_at text,
    cancel_requested integer not null default 0 check(cancel_requested in (0, 1)),
    input_tokens integer check(input_tokens is null or input_tokens >= 0),
    output_tokens integer check(output_tokens is null or output_tokens >= 0),
    cache_hit_tokens integer check(cache_hit_tokens is null or cache_hit_tokens >= 0),
    cache_miss_tokens integer check(cache_miss_tokens is null or cache_miss_tokens >= 0),
    error_code text,
    error_message text,
    created_at text not null,
    started_at text,
    finished_at text,
    updated_at text not null,
    check(
        (cloud_consent_version is null and cloud_consented_at is null)
        or (cloud_consent_version is not null and cloud_consented_at is not null)
    )
);

create table ai_scope_items (
    id text primary key,
    run_id text not null references ai_runs(id) on delete cascade,
    ordinal integer not null check(ordinal >= 0),
    item_kind text not null check(item_kind in ('paper', 'attachment', 'note', 'anchor')),
    object_id text not null,
    parent_object_id text,
    content_sha256 text,
    object_revision integer,
    snapshot_hash text not null check(length(snapshot_hash) = 64),
    created_at text not null,
    unique(run_id, ordinal),
    unique(run_id, item_kind, object_id),
    check(content_sha256 is null or length(content_sha256) = 64),
    check(object_revision is null or object_revision >= 0)
);

create table ai_run_stages (
    id text primary key,
    run_id text not null references ai_runs(id) on delete cascade,
    stage_kind text not null check(stage_kind in (
        'resolving_scope', 'retrieving', 'extracting_evidence', 'generating', 'validating'
    )),
    attempt integer not null check(attempt >= 1),
    status text not null check(status in (
        'queued', 'running', 'succeeded', 'failed', 'canceled', 'discarded'
    )),
    input_hash text check(input_hash is null or length(input_hash) = 64),
    output_hash text check(output_hash is null or length(output_hash) = 64),
    input_tokens integer check(input_tokens is null or input_tokens >= 0),
    output_tokens integer check(output_tokens is null or output_tokens >= 0),
    duration_ms integer check(duration_ms is null or duration_ms >= 0),
    error_code text,
    error_message text,
    started_at text,
    finished_at text,
    created_at text not null,
    unique(run_id, stage_kind, attempt)
);

create index ai_runs_status_created_idx on ai_runs(status, created_at desc);
create index ai_runs_provider_created_idx on ai_runs(provider_id, created_at desc);
create index ai_scope_items_run_idx on ai_scope_items(run_id, ordinal);
create index ai_run_stages_run_idx on ai_run_stages(run_id, created_at, attempt);
