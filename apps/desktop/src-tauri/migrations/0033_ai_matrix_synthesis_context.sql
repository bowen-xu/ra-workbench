create table ai_matrix_synthesis_run_contexts (
    run_id text primary key references ai_runs(id) on delete restrict,
    project_id text not null references research_projects(id) on delete restrict,
    matrix_id text not null,
    matrix_revision integer not null check(matrix_revision >= 1),
    view_kind text not null check(view_kind in (
        'taxonomy', 'historical_context', 'gap_hypothesis', 'innovation_hypothesis'
    )),
    title text not null check(length(trim(title)) between 1 and 240),
    scope_note text not null check(length(trim(scope_note)) between 1 and 4000),
    created_at text not null,
    foreign key(matrix_id, matrix_revision)
        references evidence_matrix_revisions(matrix_id, revision) on delete restrict
);

create table ai_matrix_synthesis_input_cells (
    run_id text not null references ai_matrix_synthesis_run_contexts(run_id) on delete restrict,
    ordinal integer not null check(ordinal between 0 and 19),
    cell_id text not null,
    cell_revision integer not null check(cell_revision >= 1),
    selection_reason text not null check(length(selection_reason) <= 500),
    created_at text not null,
    primary key(run_id, ordinal),
    unique(run_id, cell_id),
    foreign key(cell_id, cell_revision)
        references evidence_matrix_cell_revisions(cell_id, revision) on delete restrict
);

create trigger ai_matrix_synthesis_context_no_update
before update on ai_matrix_synthesis_run_contexts begin
    select raise(abort, 'AI matrix synthesis contexts are immutable');
end;
create trigger ai_matrix_synthesis_context_no_delete
before delete on ai_matrix_synthesis_run_contexts begin
    select raise(abort, 'AI matrix synthesis contexts are immutable');
end;
create trigger ai_matrix_synthesis_inputs_no_update
before update on ai_matrix_synthesis_input_cells begin
    select raise(abort, 'AI matrix synthesis inputs are immutable');
end;
create trigger ai_matrix_synthesis_inputs_no_delete
before delete on ai_matrix_synthesis_input_cells begin
    select raise(abort, 'AI matrix synthesis inputs are immutable');
end;

create index ai_matrix_synthesis_inputs_cell_idx
    on ai_matrix_synthesis_input_cells(cell_id, cell_revision);
