create virtual table note_search using fts5(
    object_id unindexed,
    object_kind unindexed,
    title,
    body,
    citation_text,
    paper_title,
    creators,
    tags,
    project_titles,
    source_state unindexed,
    updated_at unindexed,
    tokenize = 'unicode61 remove_diacritics 2'
);

create table note_index_state (
    id integer primary key check(id = 1),
    schema_version integer not null check(schema_version >= 1),
    state text not null check(state in ('ready', 'dirty', 'rebuilding', 'failed')),
    last_rebuilt_at text,
    last_error_code text
);

insert into note_index_state(id, schema_version, state)
values (1, 1, 'dirty');
