-- Transaction-local row scope for permanent literature cleanup. The owner
-- populates exact rows, deletes children first, then removes this scope before
-- commit. No document, concept, task or shared-file ownership is inferred here.
create table literature_cleanup_rows (
    item_id text not null,
    table_name text not null,
    row_key integer not null,
    primary key(item_id, table_name, row_key)
) without rowid;
create index literature_cleanup_rows_guard on literature_cleanup_rows(table_name,row_key);

drop trigger paper_reading_profile_selection_revisions_no_delete;
create trigger paper_reading_profile_selection_revisions_no_delete
before delete on paper_reading_profile_selection_revisions
when not exists(select 1 from literature_cleanup_rows
    where table_name='paper_reading_profile_selection_revisions' and row_key=old.rowid)
begin
    select raise(abort, 'reading profile selection revisions are immutable');
end;

drop trigger reference_record_revisions_no_delete;
create trigger reference_record_revisions_no_delete
before delete on reference_record_revisions
when not exists(select 1 from literature_cleanup_rows
    where table_name='reference_record_revisions' and row_key=old.rowid)
begin
    select raise(abort, 'reference record revisions are immutable');
end;

drop trigger evidence_matrix_cell_revisions_no_delete;
create trigger evidence_matrix_cell_revisions_no_delete
before delete on evidence_matrix_cell_revisions
when not exists(select 1 from literature_cleanup_rows
    where table_name='evidence_matrix_cell_revisions' and row_key=old.rowid)
begin
    select raise(abort, 'evidence matrix cell revisions are immutable');
end;

drop trigger ai_evidence_inspections_no_delete;
create trigger ai_evidence_inspections_no_delete
before delete on ai_evidence_inspections
when not exists(select 1 from literature_cleanup_rows
    where table_name='ai_evidence_inspections' and row_key=old.rowid)
begin
    select raise(abort, 'ai evidence inspections are immutable');
end;

drop trigger ai_overview_profile_runs_no_delete;
create trigger ai_overview_profile_runs_no_delete
before delete on ai_overview_profile_runs
when not exists(select 1 from literature_cleanup_rows
    where table_name='ai_overview_profile_runs' and row_key=old.rowid)
begin
    select raise(abort, 'AI overview profile contexts are immutable');
end;

drop trigger ai_matrix_synthesis_inputs_no_delete;
create trigger ai_matrix_synthesis_inputs_no_delete
before delete on ai_matrix_synthesis_input_cells
when not exists(select 1 from literature_cleanup_rows
    where table_name='ai_matrix_synthesis_input_cells' and row_key=old.rowid)
begin
    select raise(abort, 'AI matrix synthesis inputs are immutable');
end;

drop trigger ai_citation_assessments_no_delete;
create trigger ai_citation_assessments_no_delete
before delete on ai_citation_assessments
when not exists(select 1 from literature_cleanup_rows
    where table_name='ai_citation_assessments' and row_key=old.rowid)
begin
    select raise(abort, 'ai citation assessments are immutable');
end;

drop trigger ai_overview_confirmation_batches_no_delete;
create trigger ai_overview_confirmation_batches_no_delete
before delete on ai_overview_confirmation_batches
when not exists(select 1 from literature_cleanup_rows
    where table_name='ai_overview_confirmation_batches' and row_key=old.rowid)
begin
    select raise(abort, 'AI overview confirmation batches are immutable');
end;

drop trigger ai_overview_confirmation_undos_no_delete;
create trigger ai_overview_confirmation_undos_no_delete
before delete on ai_overview_confirmation_undos
when not exists(select 1 from literature_cleanup_rows
    where table_name='ai_overview_confirmation_undos' and row_key=old.rowid)
begin
    select raise(abort, 'AI overview confirmation undos are immutable');
end;

drop trigger ai_paper_review_confirmation_batches_no_delete;
create trigger ai_paper_review_confirmation_batches_no_delete
before delete on ai_paper_review_confirmation_batches
when not exists(select 1 from literature_cleanup_rows
    where table_name='ai_paper_review_confirmation_batches' and row_key=old.rowid)
begin
    select raise(abort, 'AI paper review confirmation batches are immutable');
end;

drop trigger ai_paper_review_metadata_confirmations_no_delete;
create trigger ai_paper_review_metadata_confirmations_no_delete
before delete on ai_paper_review_metadata_confirmations
when not exists(select 1 from literature_cleanup_rows
    where table_name='ai_paper_review_metadata_confirmations' and row_key=old.rowid)
begin
    select raise(abort, 'AI paper review metadata confirmations are immutable');
end;

drop trigger ai_paper_review_overview_batch_links_no_delete;
create trigger ai_paper_review_overview_batch_links_no_delete
before delete on ai_paper_review_overview_batch_links
when not exists(select 1 from literature_cleanup_rows
    where table_name='ai_paper_review_overview_batch_links' and row_key=old.rowid)
begin
    select raise(abort, 'AI paper review overview links are immutable');
end;

drop trigger ai_paper_review_metadata_confirmation_undos_no_delete;
create trigger ai_paper_review_metadata_confirmation_undos_no_delete
before delete on ai_paper_review_metadata_confirmation_undos
when not exists(select 1 from literature_cleanup_rows
    where table_name='ai_paper_review_metadata_confirmation_undos' and row_key=old.rowid)
begin
    select raise(abort, 'AI paper review metadata undos are immutable');
end;

drop trigger ai_paper_review_confirmation_undos_no_delete;
create trigger ai_paper_review_confirmation_undos_no_delete
before delete on ai_paper_review_confirmation_undos
when not exists(select 1 from literature_cleanup_rows
    where table_name='ai_paper_review_confirmation_undos' and row_key=old.rowid)
begin
    select raise(abort, 'AI paper review undos are immutable');
end;

drop trigger paper_concept_assertion_revisions_no_delete;
create trigger paper_concept_assertion_revisions_no_delete
before delete on paper_concept_assertion_revisions
when not exists(select 1 from literature_cleanup_rows
    where table_name='paper_concept_assertion_revisions' and row_key=old.rowid)
begin
    select raise(abort, 'paper concept assertion revisions are immutable');
end;

drop trigger ai_overview_template_runs_no_delete;
create trigger ai_overview_template_runs_no_delete
before delete on ai_overview_template_runs
when not exists(select 1 from literature_cleanup_rows
    where table_name='ai_overview_template_runs' and row_key=old.rowid)
begin
    select raise(abort, 'overview template snapshots are immutable');
end;

drop trigger ai_overview_field_confirmations_no_delete;
create trigger ai_overview_field_confirmations_no_delete
before delete on ai_overview_field_confirmations
when not exists(select 1 from literature_cleanup_rows
    where table_name='ai_overview_field_confirmations' and row_key=old.rowid)
begin
    select raise(abort, 'AI overview field confirmations are immutable');
end;

drop trigger ai_overview_field_review_resets_no_delete;
create trigger ai_overview_field_review_resets_no_delete
before delete on ai_overview_field_review_resets
when not exists(select 1 from literature_cleanup_rows
    where table_name='ai_overview_field_review_resets' and row_key=old.rowid)
begin
    select raise(abort, 'reading field review resets are immutable');
end;

