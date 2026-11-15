-- Freeze bounded user-configured portrait tags and reusable template definitions.
-- Other task kinds retain the deployed 2,000-character constraint.
-- Rebuild with foreign keys disabled; the migration runner checks every FK before commit.
create table ai_runs_catalog_next (
    id text primary key,
    task_kind text not null check(task_kind in (
        'provider_diagnostic', 'selection_explanation', 'paper_overview',
        'paper_profile_classification', 'paper_concept_map', 'paper_suggestions', 'reference_extraction', 'matrix_synthesis'
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
    user_goal text not null default '' check(
        (task_kind = 'paper_profile_classification' and length(cast(user_goal as blob)) <= 64000)
        or (task_kind <> 'paper_profile_classification' and length(user_goal) <= 2000)
    ),
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
    reasoning_effort text,
    context_snapshot_id text references paper_context_snapshots(id) on delete restrict,
    check(
        (cloud_consent_version is null and cloud_consented_at is null)
        or (cloud_consent_version is not null and cloud_consented_at is not null)
    )
);

insert into ai_runs_catalog_next (
    id, task_kind, status, provider_id, model_id, pipeline_version, prompt_version,
    output_schema_version, user_goal, scope_hash, input_item_count, max_input_tokens,
    max_output_tokens, timeout_ms, cloud_consent_version, cloud_consented_at,
    cancel_requested, input_tokens, output_tokens, cache_hit_tokens, cache_miss_tokens,
    error_code, error_message, created_at, started_at, finished_at, updated_at,
    reasoning_effort, context_snapshot_id
)
select
    id, task_kind, status, provider_id, model_id, pipeline_version, prompt_version,
    output_schema_version, user_goal, scope_hash, input_item_count, max_input_tokens,
    max_output_tokens, timeout_ms, cloud_consent_version, cloud_consented_at,
    cancel_requested, input_tokens, output_tokens, cache_hit_tokens, cache_miss_tokens,
    error_code, error_message, created_at, started_at, finished_at, updated_at,
    reasoning_effort, context_snapshot_id
from ai_runs;

drop table ai_runs;
alter table ai_runs_catalog_next rename to ai_runs;

create index ai_runs_status_created_idx on ai_runs(status, created_at desc);
create index ai_runs_provider_created_idx on ai_runs(provider_id, created_at desc);

