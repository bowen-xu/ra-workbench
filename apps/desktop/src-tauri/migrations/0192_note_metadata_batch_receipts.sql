-- Persist only operation identities and metadata receipts; no document bodies.
create table note_metadata_batch_receipts (
    operation_id text primary key,
    request_json text not null,
    receipt_json text not null,
    created_at text not null
);
