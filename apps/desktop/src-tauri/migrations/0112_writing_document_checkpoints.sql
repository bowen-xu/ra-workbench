-- D/B and the file write intent are committed together. No time/count pruning:
-- a pending review or unsaved file must not expire like the legacy D-only draft.
create table writing_document_checkpoints (
    manuscript_id text not null references manuscripts(id) on delete cascade,
    relative_path text not null,
    revision integer not null check(revision between 1 and 9007199254740991),
    checkpoint_json text not null check(json_valid(checkpoint_json)),
    updated_at text not null,
    primary key (manuscript_id, relative_path)
);
