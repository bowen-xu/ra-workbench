create table citation_bindings (
    manuscript_id text not null references manuscripts(id) on delete cascade,
    literature_item_id text references bibliographic_items(id) on delete set null,
    source_item_id text not null,
    citation_key text not null check(length(citation_key) between 1 and 128),
    bibtex_entry text not null,
    state text not null default 'active' check(state in ('active', 'missing_source')),
    pinned_at text not null,
    updated_at text not null,
    primary key (manuscript_id, source_item_id),
    unique (manuscript_id, citation_key)
);

create table managed_bibliographies (
    manuscript_id text primary key references manuscripts(id) on delete cascade,
    relative_path text not null default 'references.bib',
    content_hash text,
    status text not null default 'ready'
        check(status in ('ready', 'conflict', 'write_error')),
    generated_at text,
    projection_version integer not null default 1
);

create index citation_bindings_key_idx
    on citation_bindings(manuscript_id, citation_key);

create trigger citation_binding_source_removed
after update of literature_item_id on citation_bindings
when new.literature_item_id is null
begin
    update citation_bindings set state = 'missing_source',
        updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
    where manuscript_id = new.manuscript_id and source_item_id = new.source_item_id;
end;
