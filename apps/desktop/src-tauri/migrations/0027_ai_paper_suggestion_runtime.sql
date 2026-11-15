create table ai_runs_next (
    id text primary key,
    task_kind text not null check(task_kind in (
        'provider_diagnostic', 'selection_explanation', 'paper_overview', 'paper_suggestions'
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

create table ai_scope_items_next (
    id text primary key,
    run_id text not null references ai_runs_next(id) on delete cascade,
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

create table ai_run_stages_next (
    id text primary key,
    run_id text not null references ai_runs_next(id) on delete cascade,
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

create table ai_evidence_bundles_next (
    id text primary key,
    run_id text not null unique references ai_runs_next(id) on delete restrict,
    scope_hash text not null check(length(scope_hash) = 64),
    retrieval_config_version text not null,
    validation_status text not null check(validation_status in ('valid', 'partial', 'invalid')),
    created_at text not null
);

create table ai_evidence_items_next (
    id text primary key,
    bundle_id text not null references ai_evidence_bundles_next(id) on delete restrict,
    citation_key text not null check(length(citation_key) between 1 and 80),
    ordinal integer not null check(ordinal >= 0),
    anchor_id text not null references source_anchors(id) on delete restrict,
    paper_id text not null,
    paper_title text not null,
    attachment_id text not null,
    content_sha256 text not null check(length(content_sha256) = 64),
    page_index integer not null check(page_index >= 0),
    page_label text,
    quote_exact text not null check(length(trim(quote_exact)) > 0),
    quote_prefix text not null default '',
    quote_suffix text not null default '',
    context_text text not null,
    relation_kind text not null check(relation_kind in ('direct_quote', 'context')),
    validation_status text not null check(validation_status in ('valid', 'invalid')),
    created_at text not null,
    unique(bundle_id, citation_key),
    unique(bundle_id, ordinal)
);

create table ai_candidates_next (
    id text primary key,
    run_id text not null unique references ai_runs_next(id) on delete restrict,
    evidence_bundle_id text not null references ai_evidence_bundles_next(id) on delete restrict,
    candidate_kind text not null check(candidate_kind in (
        'selection_explanation_v1', 'paper_overview_unit_v1', 'concept_suggestion_v1',
        'definition_suggestion_v1', 'relation_suggestion_v1', 'attention_suggestion_v1'
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

create table ai_candidate_revisions_next (
    id text primary key,
    candidate_id text not null references ai_candidates_next(id) on delete restrict,
    revision integer not null check(revision >= 1),
    payload_json text not null check(json_valid(payload_json)),
    payload_hash text not null check(length(payload_hash) = 64),
    editor_kind text not null check(editor_kind in ('ai', 'human', 'ai_retry')),
    source_run_id text unique references ai_runs_next(id) on delete restrict,
    parent_revision integer check(parent_revision is null or parent_revision >= 1),
    change_reason text not null default '',
    created_at text not null,
    unique(candidate_id, revision),
    check(
        (revision = 1 and editor_kind = 'ai' and source_run_id is not null
         and parent_revision is null)
        or (revision > 1 and editor_kind = 'human' and source_run_id is null
            and parent_revision = revision - 1)
        or (revision > 1 and editor_kind = 'ai_retry' and source_run_id is not null
            and parent_revision = revision - 1)
    )
);

create table ai_review_decisions_next (
    id text primary key,
    candidate_id text not null references ai_candidates_next(id) on delete restrict,
    candidate_revision integer not null check(candidate_revision >= 1),
    action text not null check(action in (
        'accept_unchanged', 'accept_after_edit', 'reject', 'defer', 'request_more_evidence'
    )),
    rationale text not null default '',
    idempotency_key text not null unique check(length(idempotency_key) between 1 and 200),
    target_object_kind text check(target_object_kind is null or target_object_kind = 'reading_unit'),
    target_object_id text references structured_reading_units(id) on delete restrict,
    target_object_revision integer check(target_object_revision is null or target_object_revision >= 1),
    created_at text not null,
    check(
        (action in ('accept_unchanged', 'accept_after_edit')
         and target_object_kind = 'reading_unit'
         and target_object_id is not null and target_object_revision is not null)
        or
        (action not in ('accept_unchanged', 'accept_after_edit')
         and target_object_kind is null
         and target_object_id is null and target_object_revision is null)
    ),
    foreign key(candidate_id, candidate_revision)
        references ai_candidate_revisions_next(candidate_id, revision) on delete restrict
);

create table ai_evidence_inspections_next (
    id text primary key,
    candidate_id text not null references ai_candidates_next(id) on delete restrict,
    candidate_revision integer not null check(candidate_revision >= 1),
    evidence_item_id text not null references ai_evidence_items_next(id) on delete restrict,
    viewed_at text not null,
    unique(candidate_id, candidate_revision, evidence_item_id),
    foreign key(candidate_id, candidate_revision)
        references ai_candidate_revisions_next(candidate_id, revision) on delete restrict
);

create table ai_overview_retry_runs_next (
    run_id text primary key references ai_runs_next(id) on delete restrict,
    candidate_id text not null references ai_candidates_next(id) on delete restrict,
    base_revision integer not null check(base_revision >= 1),
    target_fields_json text not null check(
        json_valid(target_fields_json) and json_type(target_fields_json) = 'array'
        and json_array_length(target_fields_json) between 1 and 9
    ),
    created_at text not null,
    foreign key(candidate_id, base_revision)
        references ai_candidate_revisions_next(candidate_id, revision) on delete restrict
);

create table ai_overview_profile_runs_next (
    run_id text primary key references ai_runs_next(id) on delete restrict,
    paper_id text not null,
    selection_revision integer not null check(selection_revision >= 1),
    profile_refs_json text not null check(
        json_valid(profile_refs_json) and json_type(profile_refs_json) = 'array'
        and json_array_length(profile_refs_json) between 1 and 3
    ),
    field_kinds_json text not null check(
        json_valid(field_kinds_json) and json_type(field_kinds_json) = 'array'
        and json_array_length(field_kinds_json) between 1 and 9
    ),
    created_at text not null
);

create table ai_attention_feedback_next (
    id text primary key,
    candidate_id text not null references ai_candidates_next(id) on delete restrict,
    candidate_revision integer not null check(candidate_revision >= 1),
    suggestion_id text not null check(length(suggestion_id) between 1 and 80),
    feedback_kind text not null check(feedback_kind in ('useful', 'not_useful', 'dismissed')),
    note text not null default '' check(length(note) <= 1000),
    idempotency_key text not null unique check(length(idempotency_key) between 1 and 200),
    created_at text not null,
    foreign key(candidate_id, candidate_revision)
        references ai_candidate_revisions_next(candidate_id, revision) on delete restrict
);

insert into ai_runs_next
(id, task_kind, status, provider_id, model_id, pipeline_version, prompt_version,
 output_schema_version, user_goal, scope_hash, input_item_count, max_input_tokens,
 max_output_tokens, timeout_ms, cloud_consent_version, cloud_consented_at,
 cancel_requested, input_tokens, output_tokens, cache_hit_tokens, cache_miss_tokens,
 error_code, error_message, created_at, started_at, finished_at, updated_at)
select id, task_kind, status, provider_id, model_id, pipeline_version, prompt_version,
       output_schema_version, user_goal, scope_hash, input_item_count, max_input_tokens,
       max_output_tokens, timeout_ms, cloud_consent_version, cloud_consented_at,
       cancel_requested, input_tokens, output_tokens, cache_hit_tokens, cache_miss_tokens,
       error_code, error_message, created_at, started_at, finished_at, updated_at
from ai_runs;

insert into ai_scope_items_next
(id, run_id, ordinal, item_kind, object_id, parent_object_id, content_sha256,
 object_revision, snapshot_hash, created_at)
select id, run_id, ordinal, item_kind, object_id, parent_object_id, content_sha256,
       object_revision, snapshot_hash, created_at
from ai_scope_items;

insert into ai_run_stages_next
(id, run_id, stage_kind, attempt, status, input_hash, output_hash, input_tokens,
 output_tokens, duration_ms, error_code, error_message, started_at, finished_at, created_at)
select id, run_id, stage_kind, attempt, status, input_hash, output_hash, input_tokens,
       output_tokens, duration_ms, error_code, error_message, started_at, finished_at, created_at
from ai_run_stages;

insert into ai_evidence_bundles_next
(id, run_id, scope_hash, retrieval_config_version, validation_status, created_at)
select id, run_id, scope_hash, retrieval_config_version, validation_status, created_at
from ai_evidence_bundles;

insert into ai_evidence_items_next
(id, bundle_id, citation_key, ordinal, anchor_id, paper_id, paper_title, attachment_id,
 content_sha256, page_index, page_label, quote_exact, quote_prefix, quote_suffix,
 context_text, relation_kind, validation_status, created_at)
select id, bundle_id, citation_key, ordinal, anchor_id, paper_id, paper_title, attachment_id,
       content_sha256, page_index, page_label, quote_exact, quote_prefix, quote_suffix,
       context_text, relation_kind, validation_status, created_at
from ai_evidence_items;

insert into ai_candidates_next
(id, run_id, evidence_bundle_id, candidate_kind, schema_version, current_revision,
 review_state, validation_status, original_payload_hash, created_at, updated_at)
select id, run_id, evidence_bundle_id, candidate_kind, schema_version, current_revision,
       review_state, validation_status, original_payload_hash, created_at, updated_at
from ai_candidates;

insert into ai_candidate_revisions_next
(id, candidate_id, revision, payload_json, payload_hash, editor_kind, source_run_id,
 parent_revision, change_reason, created_at)
select id, candidate_id, revision, payload_json, payload_hash, editor_kind, source_run_id,
       parent_revision, change_reason, created_at
from ai_candidate_revisions;

insert into ai_review_decisions_next
(id, candidate_id, candidate_revision, action, rationale, idempotency_key,
 target_object_kind, target_object_id, target_object_revision, created_at)
select id, candidate_id, candidate_revision, action, rationale, idempotency_key,
       target_object_kind, target_object_id, target_object_revision, created_at
from ai_review_decisions;

insert into ai_evidence_inspections_next
(id, candidate_id, candidate_revision, evidence_item_id, viewed_at)
select id, candidate_id, candidate_revision, evidence_item_id, viewed_at
from ai_evidence_inspections;

insert into ai_overview_retry_runs_next
(run_id, candidate_id, base_revision, target_fields_json, created_at)
select run_id, candidate_id, base_revision, target_fields_json, created_at
from ai_overview_retry_runs;

insert into ai_overview_profile_runs_next
(run_id, paper_id, selection_revision, profile_refs_json, field_kinds_json, created_at)
select run_id, paper_id, selection_revision, profile_refs_json, field_kinds_json, created_at
from ai_overview_profile_runs;

insert into ai_attention_feedback_next
(id, candidate_id, candidate_revision, suggestion_id, feedback_kind, note,
 idempotency_key, created_at)
select id, candidate_id, candidate_revision, suggestion_id, feedback_kind, note,
       idempotency_key, created_at
from ai_attention_feedback;

drop table ai_attention_feedback;
drop table ai_overview_profile_runs;
drop table ai_overview_retry_runs;
drop table ai_evidence_inspections;
drop table ai_review_decisions;
drop table ai_candidate_revisions;
drop table ai_candidates;
drop table ai_evidence_items;
drop table ai_evidence_bundles;
drop table ai_run_stages;
drop table ai_scope_items;
drop table ai_runs;

alter table ai_runs_next rename to ai_runs;
alter table ai_scope_items_next rename to ai_scope_items;
alter table ai_run_stages_next rename to ai_run_stages;
alter table ai_evidence_bundles_next rename to ai_evidence_bundles;
alter table ai_evidence_items_next rename to ai_evidence_items;
alter table ai_candidates_next rename to ai_candidates;
alter table ai_candidate_revisions_next rename to ai_candidate_revisions;
alter table ai_review_decisions_next rename to ai_review_decisions;
alter table ai_evidence_inspections_next rename to ai_evidence_inspections;
alter table ai_overview_retry_runs_next rename to ai_overview_retry_runs;
alter table ai_overview_profile_runs_next rename to ai_overview_profile_runs;
alter table ai_attention_feedback_next rename to ai_attention_feedback;

create trigger ai_candidate_revisions_no_update before update on ai_candidate_revisions begin
    select raise(abort, 'ai candidate revisions are immutable');
end;
create trigger ai_candidate_revisions_no_delete before delete on ai_candidate_revisions begin
    select raise(abort, 'ai candidate revisions are immutable');
end;
create trigger ai_review_decisions_no_update before update on ai_review_decisions begin
    select raise(abort, 'ai review decisions are immutable');
end;
create trigger ai_review_decisions_no_delete before delete on ai_review_decisions begin
    select raise(abort, 'ai review decisions are immutable');
end;
create trigger ai_evidence_inspections_no_update before update on ai_evidence_inspections begin
    select raise(abort, 'ai evidence inspections are immutable');
end;
create trigger ai_evidence_inspections_no_delete before delete on ai_evidence_inspections begin
    select raise(abort, 'ai evidence inspections are immutable');
end;
create trigger ai_overview_retry_runs_no_update before update on ai_overview_retry_runs begin
    select raise(abort, 'ai overview retry contexts are immutable');
end;
create trigger ai_overview_retry_runs_no_delete before delete on ai_overview_retry_runs begin
    select raise(abort, 'ai overview retry contexts are immutable');
end;
create trigger ai_overview_profile_runs_no_update before update on ai_overview_profile_runs begin
    select raise(abort, 'AI overview profile contexts are immutable');
end;
create trigger ai_overview_profile_runs_no_delete before delete on ai_overview_profile_runs begin
    select raise(abort, 'AI overview profile contexts are immutable');
end;
create trigger ai_attention_feedback_no_update before update on ai_attention_feedback begin
    select raise(abort, 'AI attention feedback is immutable');
end;
create trigger ai_attention_feedback_no_delete before delete on ai_attention_feedback begin
    select raise(abort, 'AI attention feedback is immutable');
end;

create index ai_runs_status_created_idx on ai_runs(status, created_at desc);
create index ai_runs_provider_created_idx on ai_runs(provider_id, created_at desc);
create index ai_scope_items_run_idx on ai_scope_items(run_id, ordinal);
create index ai_run_stages_run_idx on ai_run_stages(run_id, created_at, attempt);
create index ai_evidence_items_anchor_idx on ai_evidence_items(anchor_id);
create index ai_candidates_review_idx on ai_candidates(review_state, updated_at desc);
create index ai_review_decisions_candidate_idx
    on ai_review_decisions(candidate_id, created_at desc);
create index ai_evidence_inspections_candidate_idx
    on ai_evidence_inspections(candidate_id, candidate_revision, viewed_at desc);
create index ai_overview_retry_candidate_idx
    on ai_overview_retry_runs(candidate_id, base_revision);
create index ai_attention_feedback_candidate_idx
    on ai_attention_feedback(candidate_id, candidate_revision, suggestion_id, created_at);

create table ai_suggestion_run_contexts (
    run_id text primary key references ai_runs(id) on delete restrict,
    candidate_kind text not null check(candidate_kind in (
        'concept_suggestion_v1', 'definition_suggestion_v1',
        'relation_suggestion_v1', 'attention_suggestion_v1'
    )),
    created_at text not null
);

create trigger ai_suggestion_run_contexts_no_update
before update on ai_suggestion_run_contexts begin
    select raise(abort, 'AI suggestion run contexts are immutable');
end;
create trigger ai_suggestion_run_contexts_no_delete
before delete on ai_suggestion_run_contexts begin
    select raise(abort, 'AI suggestion run contexts are immutable');
end;
