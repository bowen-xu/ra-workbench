-- Stable bibliography identities own a base label independently of its suffix.
create table manuscript_citation_key_bases (
    manuscript_id text not null references manuscripts(id) on delete cascade,
    reference_id text not null,
    base_key text not null,
    primary key (manuscript_id, reference_id)
);
-- Canonical rows and checkpoints commit with a durable filesystem redo plan.
create table manuscript_citation_key_writes (
    manuscript_id text primary key references manuscripts(id) on delete cascade,
    payload_json text not null
);
