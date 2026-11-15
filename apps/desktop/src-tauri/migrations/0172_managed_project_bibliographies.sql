-- Project-only bibliography entries are independent of the global library.
-- Preserve original import bytes; managed files are deterministic projections.
create table manuscript_bibliography_files (
    manuscript_id text not null references manuscripts(id) on delete cascade,
    relative_path text not null,
    original_content text not null,
    support_content text not null,
    content_hash text not null,
    primary key (manuscript_id, relative_path)
);
create table manuscript_bibliography_entries (
    id text primary key,
    manuscript_id text not null references manuscripts(id) on delete cascade,
    citation_key text not null,
    original_key text not null,
    original_bibtex text not null,
    imported_draft_json text not null,
    bibtex_entry text not null,
    literature_item_id text references bibliographic_items(id) on delete set null,
    library_snapshot_json text,
    unique (manuscript_id, citation_key)
);
create table manuscript_bibliography_members (
    manuscript_id text not null,
    relative_path text not null,
    entry_id text not null references manuscript_bibliography_entries(id) on delete cascade,
    primary key (manuscript_id, relative_path, entry_id),
    foreign key (manuscript_id, relative_path)
        references manuscript_bibliography_files(manuscript_id, relative_path) on delete cascade
);
