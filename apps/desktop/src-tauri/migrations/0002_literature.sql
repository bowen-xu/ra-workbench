create table bibliographic_items (
    id text primary key,
    item_type text not null check(item_type in ('journalArticle', 'conferencePaper', 'preprint', 'report', 'document')),
    title text not null default '',
    abstract_text text not null default '',
    issued_year integer,
    container_title text not null default '',
    volume text not null default '',
    issue text not null default '',
    pages text not null default '',
    url text not null default '',
    language text not null default '',
    bibtex_key text not null default '',
    raw_bibtex text,
    reading_status text not null default 'unread'
        check(reading_status in ('unread', 'reading', 'read')),
    archived_at text,
    created_at text not null,
    updated_at text not null
);

create table creators (
    id text primary key,
    family_name text not null default '',
    given_name text not null default '',
    literal_name text not null default ''
);

create table item_creators (
    item_id text not null references bibliographic_items(id) on delete cascade,
    creator_id text not null references creators(id) on delete cascade,
    role text not null,
    position integer not null check(position >= 0),
    primary key (item_id, role, position)
);

create table identifiers (
    id text primary key,
    item_id text not null references bibliographic_items(id) on delete cascade,
    kind text not null,
    normalized_value text not null,
    display_value text not null
);

create table metadata_provenance (
    item_id text not null references bibliographic_items(id) on delete cascade,
    field_name text not null,
    source_kind text not null,
    source_reference text,
    confirmed_by_user integer not null default 0 check(confirmed_by_user in (0, 1)),
    updated_at text not null,
    primary key (item_id, field_name)
);

create table project_items (
    project_id text not null references research_projects(id) on delete cascade,
    item_id text not null references bibliographic_items(id) on delete cascade,
    added_at text not null,
    primary key (project_id, item_id)
);

create table content_objects (
    sha256 text primary key check(length(sha256) = 64),
    relative_path text not null unique,
    mime_type text not null,
    byte_size integer not null check(byte_size >= 0),
    created_at text not null
);

create table attachments (
    id text primary key,
    item_id text references bibliographic_items(id) on delete cascade,
    content_sha256 text not null references content_objects(sha256) on delete restrict,
    display_name text not null,
    attachment_kind text not null default 'pdf',
    source_path text,
    created_at text not null
);

create table import_jobs (
    id text primary key,
    source_kind text not null,
    state text not null check(state in ('queued', 'running', 'completed', 'completed_with_errors', 'failed', 'cancelled', 'interrupted')),
    total_count integer not null default 0,
    completed_count integer not null default 0,
    created_at text not null,
    updated_at text not null
);

create table import_results (
    id text primary key,
    job_id text not null references import_jobs(id) on delete cascade,
    input_label text not null,
    state text not null check(state in ('pending', 'imported', 'duplicate', 'failed', 'cancelled')),
    item_id text references bibliographic_items(id) on delete set null,
    error_code text,
    error_message text,
    raw_input text,
    created_at text not null
);

create table duplicate_candidates (
    id text primary key,
    left_item_id text not null references bibliographic_items(id) on delete cascade,
    right_item_id text not null references bibliographic_items(id) on delete cascade,
    reason text not null,
    confidence real not null check(confidence between 0 and 1),
    decision text check(decision in ('keep_both', 'merge', 'dismiss')),
    created_at text not null,
    decided_at text,
    check(left_item_id <> right_item_id)
);

create table tags (
    id text primary key,
    name text not null collate nocase unique
);

create table item_tags (
    item_id text not null references bibliographic_items(id) on delete cascade,
    tag_id text not null references tags(id) on delete cascade,
    primary key (item_id, tag_id)
);

create table reading_states (
    item_id text primary key references bibliographic_items(id) on delete cascade,
    attachment_id text references attachments(id) on delete set null,
    page_index integer not null default 0 check(page_index >= 0),
    zoom real not null default 1 check(zoom > 0),
    updated_at text not null
);

create table source_anchors (
    id text primary key,
    item_id text not null references bibliographic_items(id) on delete cascade,
    attachment_id text not null references attachments(id) on delete cascade,
    content_sha256 text not null references content_objects(sha256) on delete restrict,
    selector_version integer not null default 1,
    page_index integer not null check(page_index >= 0),
    page_label text,
    rects_json text not null default '[]' check(json_valid(rects_json)),
    quote_exact text not null default '',
    quote_prefix text not null default '',
    quote_suffix text not null default '',
    created_at text not null
);

create table annotations (
    id text primary key,
    anchor_id text not null references source_anchors(id) on delete cascade,
    annotation_type text not null check(annotation_type in ('highlight', 'note')),
    color text not null default '#f4cf65',
    body text not null default '',
    created_at text not null,
    updated_at text not null
);

create virtual table literature_search using fts5(
    item_id unindexed,
    title,
    creators,
    identifiers,
    tags,
    full_text,
    tokenize = 'unicode61 remove_diacritics 2'
);

create index bibliographic_items_created_at_idx on bibliographic_items(created_at desc);
create index item_creators_item_idx on item_creators(item_id, position);
create index identifiers_item_idx on identifiers(item_id);
create index identifiers_normalized_idx on identifiers(kind, normalized_value);
create index attachments_item_idx on attachments(item_id);
create index import_results_job_idx on import_results(job_id);
create index source_anchors_attachment_page_idx on source_anchors(attachment_id, page_index);
create index annotations_anchor_idx on annotations(anchor_id);
