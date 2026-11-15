-- A committed raw draft precedes parsing and publication, with resumable request identity.
create table structured_resource_write_intents (
    request_id text primary key,
    resource_id text not null references structured_resources(id) on delete cascade,
    request_hash text not null,
    draft_id text not null,
    draft_revision integer not null check(draft_revision>=1),
    creator_scope text not null,
    created_at text not null default(strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
