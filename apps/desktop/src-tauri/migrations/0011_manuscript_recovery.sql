create table manuscript_recovery_drafts (
    manuscript_id text not null references manuscripts(id) on delete cascade,
    relative_path text not null,
    base_sha256 text not null check(length(base_sha256) = 64),
    content text not null,
    updated_at text not null,
    primary key (manuscript_id, relative_path)
);

create index manuscript_recovery_updated_idx
    on manuscript_recovery_drafts(updated_at);
