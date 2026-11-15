create table evidence_matrices (
    id text primary key,
    project_id text not null references research_projects(id) on delete cascade,
    title text not null check(length(trim(title)) between 1 and 240),
    paper_ids_json text not null check(json_valid(paper_ids_json)),
    dimensions_json text not null check(json_valid(dimensions_json)),
    current_revision integer not null check(current_revision >= 1),
    created_at text not null,
    updated_at text not null
);

create table evidence_matrix_revisions (
    id text primary key,
    matrix_id text not null references evidence_matrices(id) on delete restrict,
    revision integer not null check(revision >= 1),
    snapshot_json text not null check(json_valid(snapshot_json)),
    editor_kind text not null check(editor_kind in ('human', 'ai_review')),
    change_reason text not null check(length(change_reason) between 1 and 120),
    created_at text not null,
    unique(matrix_id, revision)
);

create table evidence_matrix_cells (
    id text primary key,
    matrix_id text not null references evidence_matrices(id) on delete cascade,
    paper_id text not null references bibliographic_items(id) on delete restrict,
    dimension_id text not null check(length(dimension_id) between 1 and 80),
    coverage_state text not null check(coverage_state in (
        'unreviewed', 'not_found', 'populated'
    )),
    coverage_note text not null default '' check(length(coverage_note) <= 2000),
    statements_json text not null default '[]' check(json_valid(statements_json)),
    provenance_kind text not null check(provenance_kind in (
        'human_original', 'ai_accepted_unchanged', 'ai_accepted_after_edit'
    )),
    current_revision integer not null check(current_revision >= 1),
    created_at text not null,
    updated_at text not null,
    unique(matrix_id, paper_id, dimension_id)
);

create table evidence_matrix_cell_revisions (
    id text primary key,
    cell_id text not null references evidence_matrix_cells(id) on delete restrict,
    revision integer not null check(revision >= 1),
    snapshot_json text not null check(json_valid(snapshot_json)),
    editor_kind text not null check(editor_kind in ('human', 'ai_review')),
    change_reason text not null check(length(change_reason) between 1 and 120),
    created_at text not null,
    unique(cell_id, revision)
);

create index evidence_matrices_project_idx
    on evidence_matrices(project_id, updated_at desc);
create index evidence_matrix_cells_matrix_idx
    on evidence_matrix_cells(matrix_id, paper_id, dimension_id);

create trigger evidence_matrix_revisions_no_update
before update on evidence_matrix_revisions begin
    select raise(abort, 'evidence matrix revisions are immutable');
end;
create trigger evidence_matrix_revisions_no_delete
before delete on evidence_matrix_revisions begin
    select raise(abort, 'evidence matrix revisions are immutable');
end;
create trigger evidence_matrix_cell_revisions_no_update
before update on evidence_matrix_cell_revisions begin
    select raise(abort, 'evidence matrix cell revisions are immutable');
end;
create trigger evidence_matrix_cell_revisions_no_delete
before delete on evidence_matrix_cell_revisions begin
    select raise(abort, 'evidence matrix cell revisions are immutable');
end;
