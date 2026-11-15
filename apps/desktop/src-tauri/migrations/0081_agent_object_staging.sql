-- Survives conversation deletion: every attempted new object remains discoverable
-- until its bytes are verified and registered for normal reference/GC handling.
create table agent_object_staging (
    sha256 text primary key check(length(sha256) = 64 and sha256 not glob '*[^0-9a-f]*'),
    byte_size integer not null check(byte_size between 0 and 268435456),
    last_attempt_at text,
    created_at text not null default (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
