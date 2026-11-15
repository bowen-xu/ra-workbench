create table manuscripts (
    id text primary key,
    project_id text not null references research_projects(id) on delete cascade,
    title text not null check(length(title) between 1 and 180),
    root_path text not null default 'main.tex',
    status text not null default 'active' check(status in ('active', 'archived')),
    created_at text not null,
    updated_at text not null
);

create table manuscript_files (
    manuscript_id text not null references manuscripts(id) on delete cascade,
    relative_path text not null,
    file_kind text not null check(file_kind in ('tex', 'bib', 'image', 'style', 'other')),
    byte_size integer not null check(byte_size >= 0),
    sha256 text not null check(length(sha256) = 64),
    modified_at text not null,
    primary key (manuscript_id, relative_path)
);

create table writing_sessions (
    manuscript_id text primary key references manuscripts(id) on delete cascade,
    open_paths_json text not null default '[]' check(json_valid(open_paths_json)),
    active_path text,
    cursor_state_json text not null default '{}' check(json_valid(cursor_state_json)),
    layout_state_json text not null default '{}' check(json_valid(layout_state_json)),
    updated_at text not null
);

create index manuscripts_project_updated_idx
    on manuscripts(project_id, updated_at desc);
create index manuscript_files_kind_idx
    on manuscript_files(manuscript_id, file_kind, relative_path);
