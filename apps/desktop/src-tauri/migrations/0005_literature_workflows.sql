alter table import_jobs add column cancellation_requested integer not null default 0
    check(cancellation_requested in (0, 1));

alter table import_results add column updated_at text;

update import_results set updated_at = created_at where updated_at is null;

create table metadata_candidates (
    id text primary key,
    item_id text references bibliographic_items(id) on delete cascade,
    source_kind text not null,
    source_reference text,
    draft_json text not null check(json_valid(draft_json)),
    raw_payload text,
    state text not null default 'pending'
        check(state in ('pending', 'applied', 'rejected')),
    created_at text not null,
    decided_at text
);

create table merge_operations (
    id text primary key,
    survivor_item_id text not null references bibliographic_items(id) on delete restrict,
    merged_item_id text not null references bibliographic_items(id) on delete restrict,
    snapshot_json text not null check(json_valid(snapshot_json)),
    state text not null default 'applied' check(state in ('applied', 'undone')),
    created_at text not null,
    undone_at text,
    check(survivor_item_id <> merged_item_id)
);

create table claims (
    id text primary key,
    project_id text references research_projects(id) on delete cascade,
    statement text not null check(length(trim(statement)) > 0),
    origin text not null default 'user' check(origin in ('user', 'ai')),
    created_at text not null,
    updated_at text not null
);

create table evidence_links (
    id text primary key,
    claim_id text not null references claims(id) on delete cascade,
    anchor_id text not null references source_anchors(id) on delete restrict,
    stance text not null check(stance in ('supports', 'opposes', 'context')),
    rationale text not null default '',
    created_at text not null,
    unique(claim_id, anchor_id, stance)
);

create index metadata_candidates_item_idx on metadata_candidates(item_id, created_at desc);
create index merge_operations_items_idx on merge_operations(survivor_item_id, merged_item_id);
create index claims_project_idx on claims(project_id, created_at desc);
create index evidence_links_claim_idx on evidence_links(claim_id, created_at);
create index evidence_links_anchor_idx on evidence_links(anchor_id);
