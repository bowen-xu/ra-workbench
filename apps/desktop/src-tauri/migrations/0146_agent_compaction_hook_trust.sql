-- Hook discovery is not execution permission. Bind consent to an exact source,
-- command handler, host and model; preserve revocation versions for stale CAS.
create table agent_compaction_hook_trust (
    source_root text not null,
    handler_id text not null check(length(handler_id) = 36),
    host text not null check(host in ('writing', 'reading', 'review', 'notes')),
    provider_id text not null check(length(provider_id) between 1 and 256),
    model_id text not null check(length(model_id) between 1 and 256),
    model_revision text not null,
    source_device text not null,
    source_inode text not null,
    configuration_revision text not null
        check(length(configuration_revision) = 64
              and configuration_revision not glob '*[^0-9a-f]*'),
    revision integer not null check(revision between 1 and 9007199254740991),
    granted integer not null check(granted in (0, 1)),
    created_at text not null default (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at text not null default (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    primary key (source_root, handler_id, host, provider_id, model_id, model_revision)
);
