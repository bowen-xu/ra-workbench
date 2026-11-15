-- Preserve editable title text independently of its normalized navigation label.
-- NULL is the backwards-readable projection of the existing title column.
-- No document/history/baseline is rewritten by this migration.
alter table note_pages add column document_title text
    check (document_title is null or (
        length(document_title) <= 240
        and instr(document_title, char(10)) = 0
        and instr(document_title, char(13)) = 0
    ));
