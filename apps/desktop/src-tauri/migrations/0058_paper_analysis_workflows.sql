create table ai_runs_next (
    id text primary key,
    task_kind text not null check(task_kind in (
        'provider_diagnostic', 'selection_explanation', 'paper_overview',
        'paper_concept_map', 'paper_suggestions', 'reference_extraction', 'matrix_synthesis'
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
    reasoning_effort text,
    context_snapshot_id text references paper_context_snapshots(id) on delete restrict,
    check(
        (cloud_consent_version is null and cloud_consented_at is null)
        or (cloud_consent_version is not null and cloud_consented_at is not null)
    )
);

insert into ai_runs_next (
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
alter table ai_runs_next rename to ai_runs;

create index ai_runs_status_created_idx on ai_runs(status, created_at desc);
create index ai_runs_provider_created_idx on ai_runs(provider_id, created_at desc);

create table ai_candidates_next (
    id text primary key,
    run_id text not null unique references ai_runs(id) on delete restrict,
    evidence_bundle_id text not null references ai_evidence_bundles(id) on delete restrict,
    candidate_kind text not null check(candidate_kind in (
        'selection_explanation_v1', 'paper_overview_unit_v1', 'paper_concept_map_v1',
        'concept_suggestion_v1', 'definition_suggestion_v1',
        'relation_suggestion_v1', 'attention_suggestion_v1',
        'reference_record_candidate_v1', 'derived_research_view_v1'
    )),
    schema_version integer not null check(schema_version = 1),
    current_revision integer not null default 1 check(current_revision >= 1),
    review_state text not null default 'unreviewed' check(review_state in (
        'unreviewed', 'edited', 'deferred', 'needs_more_evidence',
        'accepted_unchanged', 'accepted_after_edit', 'rejected'
    )),
    validation_status text not null check(validation_status = 'valid'),
    original_payload_hash text not null check(length(original_payload_hash) = 64),
    created_at text not null,
    updated_at text not null
);

insert into ai_candidates_next (
    id, run_id, evidence_bundle_id, candidate_kind, schema_version, current_revision,
    review_state, validation_status, original_payload_hash, created_at, updated_at
)
select
    id, run_id, evidence_bundle_id, candidate_kind, schema_version, current_revision,
    review_state, validation_status, original_payload_hash, created_at, updated_at
from ai_candidates;

drop table ai_candidates;
alter table ai_candidates_next rename to ai_candidates;

create index ai_candidates_review_idx on ai_candidates(review_state, updated_at desc);

create table paper_analysis_workflows (
    id text primary key,
    paper_id text not null references bibliographic_items(id) on delete restrict,
    context_snapshot_id text not null references paper_context_snapshots(id) on delete restrict,
    status text not null check(status in (
        'preparing', 'overview_running', 'overview_ready', 'concepts_running',
        'needs_review', 'partial', 'failed', 'canceled', 'superseded'
    )),
    provider_id text not null references ai_provider_settings(provider_id) on delete restrict,
    model_id text not null check(length(trim(model_id)) between 1 and 240),
    reasoning_effort text,
    prompt_set_version text not null,
    active_turn_kind text check(active_turn_kind is null or active_turn_kind in (
        'paper_overview', 'paper_concept_map'
    )),
    revision integer not null default 1 check(revision >= 1),
    supersedes_workflow_id text references paper_analysis_workflows(id) on delete restrict,
    created_at text not null,
    updated_at text not null,
    completed_at text
);

create table paper_analysis_turns (
    id text primary key,
    workflow_id text not null references paper_analysis_workflows(id) on delete restrict,
    ordinal integer not null check(ordinal in (0, 1)),
    turn_kind text not null check(turn_kind in ('paper_overview', 'paper_concept_map')),
    status text not null check(status in (
        'queued', 'running', 'succeeded', 'failed', 'canceled', 'interrupted'
    )),
    candidate_id text references ai_candidates(id) on delete restrict,
    created_at text not null,
    finished_at text,
    unique(workflow_id, ordinal),
    unique(workflow_id, turn_kind),
    check(
        (ordinal = 0 and turn_kind = 'paper_overview')
        or (ordinal = 1 and turn_kind = 'paper_concept_map')
    )
);

create table paper_analysis_turn_attempts (
    id text primary key,
    turn_id text not null references paper_analysis_turns(id) on delete restrict,
    attempt integer not null check(attempt >= 1),
    status text not null check(status in (
        'queued', 'running', 'succeeded', 'failed', 'canceled', 'interrupted'
    )),
    normalized_messages_json text not null
        check(json_valid(normalized_messages_json) and json_type(normalized_messages_json) = 'array'),
    provider_id text not null references ai_provider_settings(provider_id) on delete restrict,
    model_id text not null check(length(trim(model_id)) between 1 and 240),
    reasoning_effort text,
    ai_run_id text references ai_runs(id) on delete restrict,
    candidate_id text references ai_candidates(id) on delete restrict,
    input_hash text check(input_hash is null or length(input_hash) = 64),
    output_hash text check(output_hash is null or length(output_hash) = 64),
    input_tokens integer check(input_tokens is null or input_tokens >= 0),
    output_tokens integer check(output_tokens is null or output_tokens >= 0),
    error_code text,
    error_message text,
    created_at text not null,
    started_at text,
    finished_at text,
    unique(turn_id, attempt)
);

create index paper_analysis_workflows_paper_idx
    on paper_analysis_workflows(paper_id, created_at desc);

create index paper_analysis_turns_workflow_idx
    on paper_analysis_turns(workflow_id, ordinal);

create index paper_analysis_attempts_turn_idx
    on paper_analysis_turn_attempts(turn_id, attempt desc);
