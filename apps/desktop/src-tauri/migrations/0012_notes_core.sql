create table note_pages (
    id text primary key,
    title text not null check(length(title) between 1 and 240),
    parent_page_id text references note_pages(id) on delete set null,
    position integer not null default 0 check(position >= 0),
    status text not null default 'active' check(status in ('active', 'archived', 'trashed')),
    is_favorite integer not null default 0 check(is_favorite in (0, 1)),
    revision integer not null default 0 check(revision >= 0),
    schema_version integer not null default 1 check(schema_version >= 1),
    created_at text not null,
    updated_at text not null,
    trashed_at text
);

create table note_blocks (
    id text primary key,
    scope_kind text not null check(scope_kind in ('page', 'reading')),
    page_id text references note_pages(id) on delete cascade,
    parent_block_id text references note_blocks(id) on delete cascade,
    position integer not null default 0 check(position >= 0),
    block_type text not null,
    props_json text not null default '{}' check(json_valid(props_json)),
    content_json text not null default '{"kind":"none"}' check(json_valid(content_json)),
    plain_text text not null default '',
    schema_version integer not null default 1 check(schema_version >= 1),
    revision integer not null default 0 check(revision >= 0),
    status text not null default 'active' check(status in ('active', 'archived', 'trashed')),
    is_favorite integer not null default 0 check(is_favorite in (0, 1)),
    created_at text not null,
    updated_at text not null,
    trashed_at text,
    check((scope_kind = 'page' and page_id is not null) or (scope_kind = 'reading' and page_id is null))
);

create table reading_note_sources (
    block_id text primary key references note_blocks(id) on delete cascade,
    current_anchor_id text unique references source_anchors(id) on delete set null,
    paper_id_snapshot text not null,
    paper_title_snapshot text not null default '',
    attachment_id_snapshot text not null,
    attachment_name_snapshot text not null default '',
    content_sha256 text not null,
    page_index integer not null check(page_index >= 0),
    page_label text,
    rects_json text not null default '[]' check(json_valid(rects_json)),
    quote_exact text not null default '',
    quote_prefix text not null default '',
    quote_suffix text not null default '',
    locator_version integer not null default 1 check(locator_version >= 1),
    created_at text not null,
    updated_at text not null
);

create table note_links (
    id text primary key,
    source_page_id text not null references note_pages(id) on delete cascade,
    source_block_id text references note_blocks(id) on delete set null,
    target_kind text not null check(target_kind in ('note_page', 'reading_note', 'paper', 'claim', 'manuscript')),
    target_id text not null,
    relation_type text not null check(relation_type in ('reference', 'mention', 'derived_from')),
    target_snapshot_json text not null default '{}' check(json_valid(target_snapshot_json)),
    created_at text not null,
    unique(source_page_id, source_block_id, target_kind, target_id, relation_type)
);

create table note_page_tags (
    page_id text not null references note_pages(id) on delete cascade,
    tag_id text not null references tags(id) on delete cascade,
    primary key (page_id, tag_id)
);

create table reading_note_tags (
    block_id text not null references note_blocks(id) on delete cascade,
    tag_id text not null references tags(id) on delete cascade,
    primary key (block_id, tag_id)
);

create table project_note_pages (
    project_id text not null references research_projects(id) on delete cascade,
    page_id text not null references note_pages(id) on delete cascade,
    added_at text not null,
    primary key (project_id, page_id)
);

create table project_reading_notes (
    project_id text not null references research_projects(id) on delete cascade,
    block_id text not null references note_blocks(id) on delete cascade,
    added_at text not null,
    primary key (project_id, block_id)
);

create table note_revisions (
    id text primary key,
    page_id text not null references note_pages(id) on delete cascade,
    page_revision integer not null check(page_revision >= 0),
    schema_version integer not null check(schema_version >= 1),
    document_json text not null check(json_valid(document_json)),
    reason text not null check(reason in ('autosave', 'manual', 'import', 'restore', 'structure')),
    created_at text not null,
    unique(page_id, page_revision)
);

create table note_block_assets (
    block_id text not null references note_blocks(id) on delete cascade,
    content_sha256 text not null references content_objects(sha256) on delete restrict,
    display_name text not null,
    mime_type text not null,
    created_at text not null,
    primary key (block_id, content_sha256)
);

create index note_pages_parent_position_idx on note_pages(parent_page_id, position);
create index note_pages_status_updated_idx on note_pages(status, updated_at desc);
create index note_blocks_page_position_idx on note_blocks(page_id, parent_block_id, position);
create index note_blocks_reading_status_idx on note_blocks(scope_kind, status, updated_at desc);
create index reading_note_sources_paper_page_idx on reading_note_sources(paper_id_snapshot, page_index);
create index note_links_target_idx on note_links(target_kind, target_id, created_at);
create unique index note_links_identity_idx on note_links(
    source_page_id,
    coalesce(source_block_id, ''),
    target_kind,
    target_id,
    relation_type
);
create index note_revisions_page_idx on note_revisions(page_id, page_revision desc);
