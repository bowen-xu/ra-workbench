alter table manuscripts add column last_successful_compile_run_id text;

create table manuscript_compile_runs (
    id text primary key,
    manuscript_id text not null references manuscripts(id) on delete cascade,
    engine text not null check(engine in ('pdflatex', 'xelatex', 'lualatex')),
    root_path text not null,
    status text not null check(status in ('queued', 'running', 'succeeded', 'failed', 'cancelled', 'timed_out', 'interrupted')),
    input_hash text,
    toolchain_json text not null default '{}' check(json_valid(toolchain_json)),
    raw_output text not null default '',
    exit_code integer,
    cancellation_requested integer not null default 0 check(cancellation_requested in (0, 1)),
    created_at text not null,
    started_at text,
    finished_at text
);

create table manuscript_compile_diagnostics (
    id text primary key,
    run_id text not null references manuscript_compile_runs(id) on delete cascade,
    severity text not null check(severity in ('error', 'warning', 'info')),
    message text not null,
    relative_path text,
    line integer check(line is null or line >= 1),
    column_number integer check(column_number is null or column_number >= 1),
    position integer not null check(position >= 0)
);

create table manuscript_compile_artifacts (
    id text primary key,
    run_id text not null references manuscript_compile_runs(id) on delete cascade,
    artifact_kind text not null check(artifact_kind in ('pdf', 'log', 'synctex')),
    content_sha256 text references content_objects(sha256) on delete restrict,
    cache_relative_path text,
    byte_size integer not null check(byte_size >= 0),
    check(content_sha256 is not null or cache_relative_path is not null),
    unique(run_id, artifact_kind)
);

create unique index manuscript_one_active_compile_idx
    on manuscript_compile_runs(manuscript_id)
    where status in ('queued', 'running');
create index manuscript_compile_runs_recent_idx
    on manuscript_compile_runs(manuscript_id, created_at desc);
create index manuscript_compile_diagnostics_run_idx
    on manuscript_compile_diagnostics(run_id, position);
