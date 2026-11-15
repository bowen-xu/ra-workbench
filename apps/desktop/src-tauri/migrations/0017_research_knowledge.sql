alter table claims add column revision integer not null default 1 check(revision >= 1);
alter table claims add column provenance_kind text not null default 'human_original'
    check(provenance_kind in ('human_original', 'ai_accepted_unchanged', 'ai_accepted_after_edit', 'legacy_ai'));
alter table claims add column epistemic_role text not null default 'assertion'
    check(epistemic_role in ('assertion', 'source_paraphrase', 'synthesis', 'inference', 'hypothesis', 'question', 'working_note'));

update claims set provenance_kind = case origin
    when 'ai' then 'legacy_ai'
    else 'human_original'
end;

create table research_object_revisions (
    id text primary key,
    object_kind text not null check(object_kind in (
        'claim', 'reading_unit', 'term', 'concept', 'term_usage', 'definition_claim'
    )),
    object_id text not null,
    revision integer not null check(revision >= 1),
    snapshot_json text not null check(json_valid(snapshot_json)),
    editor_kind text not null check(editor_kind in ('human', 'migration', 'ai_review', 'system')),
    change_reason text not null default '',
    created_at text not null,
    unique(object_kind, object_id, revision)
);

insert into research_object_revisions
    (id, object_kind, object_id, revision, snapshot_json, editor_kind, change_reason, created_at)
select lower(hex(randomblob(16))), 'claim', id, 1,
       json_object(
           'id', id,
           'projectId', project_id,
           'statement', statement,
           'provenanceKind', provenance_kind,
           'epistemicRole', epistemic_role,
           'revision', 1
       ),
       'migration', 'migration_0017', updated_at
from claims;

create table structured_reading_units (
    id text primary key,
    project_id text references research_projects(id) on delete set null,
    paper_id text references bibliographic_items(id) on delete set null,
    unit_kind text not null check(unit_kind in (
        'research_question', 'approach', 'key_result', 'conclusion', 'limitation',
        'open_question', 'argument', 'objection', 'response', 'other'
    )),
    title text not null default '' check(length(title) <= 240),
    body text not null check(length(trim(body)) > 0),
    provenance_kind text not null default 'human_original'
        check(provenance_kind in ('human_original', 'ai_accepted_unchanged', 'ai_accepted_after_edit')),
    epistemic_role text not null check(epistemic_role in (
        'source_paraphrase', 'synthesis', 'inference', 'hypothesis', 'question', 'working_note'
    )),
    revision integer not null default 1 check(revision >= 1),
    created_at text not null,
    updated_at text not null
);

create table reading_unit_evidence_links (
    id text primary key,
    reading_unit_id text not null references structured_reading_units(id) on delete cascade,
    anchor_id text not null references source_anchors(id) on delete restrict,
    stance text not null check(stance in ('supports', 'opposes', 'context')),
    rationale text not null default '',
    created_at text not null,
    unique(reading_unit_id, anchor_id, stance)
);

create table terms (
    id text primary key,
    text text not null check(length(trim(text)) > 0 and length(text) <= 500),
    normalized_text text not null check(length(normalized_text) > 0),
    language text not null default '' check(length(language) <= 32),
    provenance_kind text not null default 'human_original'
        check(provenance_kind in ('human_original', 'ai_accepted_unchanged', 'ai_accepted_after_edit')),
    revision integer not null default 1 check(revision >= 1),
    created_at text not null,
    updated_at text not null,
    unique(normalized_text, language)
);

create table concepts (
    id text primary key,
    label text not null default '' check(length(label) <= 500),
    explanation text not null check(length(trim(explanation)) > 0),
    disambiguation text not null default '' check(length(disambiguation) <= 1000),
    domain text not null default '' check(length(domain) <= 500),
    tradition text not null default '' check(length(tradition) <= 500),
    period text not null default '' check(length(period) <= 200),
    provenance_kind text not null default 'human_original'
        check(provenance_kind in ('human_original', 'ai_accepted_unchanged', 'ai_accepted_after_edit')),
    revision integer not null default 1 check(revision >= 1),
    created_at text not null,
    updated_at text not null
);

create table concept_terms (
    concept_id text not null references concepts(id) on delete cascade,
    term_id text not null references terms(id) on delete restrict,
    relation_kind text not null check(relation_kind in ('preferred', 'alias', 'translation', 'abbreviation')),
    created_at text not null,
    primary key(concept_id, term_id)
);

create table term_usages (
    id text primary key,
    term_id text not null references terms(id) on delete restrict,
    concept_id text references concepts(id) on delete restrict,
    anchor_id text not null references source_anchors(id) on delete restrict,
    usage_text text not null default '',
    provenance_kind text not null default 'human_original'
        check(provenance_kind in ('human_original', 'ai_accepted_unchanged', 'ai_accepted_after_edit')),
    revision integer not null default 1 check(revision >= 1),
    created_at text not null,
    updated_at text not null,
    unique(term_id, anchor_id, concept_id)
);

create table definition_claims (
    id text primary key,
    concept_id text not null references concepts(id) on delete cascade,
    statement text not null check(length(trim(statement)) > 0),
    scope_note text not null default '',
    provenance_kind text not null default 'human_original'
        check(provenance_kind in ('human_original', 'ai_accepted_unchanged', 'ai_accepted_after_edit')),
    epistemic_role text not null default 'definition' check(epistemic_role = 'definition'),
    revision integer not null default 1 check(revision >= 1),
    created_at text not null,
    updated_at text not null
);

create table definition_claim_evidence_links (
    id text primary key,
    definition_claim_id text not null references definition_claims(id) on delete cascade,
    anchor_id text not null references source_anchors(id) on delete restrict,
    stance text not null check(stance in ('supports', 'opposes', 'context')),
    rationale text not null default '',
    created_at text not null,
    unique(definition_claim_id, anchor_id, stance)
);

create index research_object_revisions_object_idx
    on research_object_revisions(object_kind, object_id, revision desc);
create index structured_reading_units_scope_idx
    on structured_reading_units(project_id, paper_id, updated_at desc);
create index reading_unit_evidence_anchor_idx
    on reading_unit_evidence_links(anchor_id);
create index terms_normalized_idx on terms(normalized_text, language);
create index concepts_updated_idx on concepts(updated_at desc);
create index concept_terms_term_idx on concept_terms(term_id, concept_id);
create unique index concept_terms_one_preferred_idx
    on concept_terms(concept_id) where relation_kind = 'preferred';
create index term_usages_concept_idx on term_usages(concept_id, created_at desc);
create index term_usages_anchor_idx on term_usages(anchor_id);
create index definition_claims_concept_idx on definition_claims(concept_id, updated_at desc);
create index definition_claim_evidence_anchor_idx
    on definition_claim_evidence_links(anchor_id);
