alter table metadata_candidates add column import_result_id text
    references import_results(id) on delete set null;

create index metadata_candidates_import_result_idx
    on metadata_candidates(import_result_id);
