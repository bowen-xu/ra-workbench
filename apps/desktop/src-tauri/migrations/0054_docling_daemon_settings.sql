alter table docling_settings rename to docling_settings_legacy;

create table docling_settings (
    id integer primary key check(id = 1),
    enabled integer not null default 1 check(enabled in (0, 1)),
    conda_environment text not null default 'research'
        check(length(trim(conda_environment)) between 1 and 200),
    default_port integer not null default 5001 check(default_port between 1 and 65535),
    fallback_start_port integer not null default 5010 check(fallback_start_port between 1 and 65535),
    fallback_end_port integer not null default 5099 check(fallback_end_port between 1 and 65535),
    idle_shutdown_ms integer not null default 1200000 check(idle_shutdown_ms between 60000 and 86400000),
    request_timeout_ms integer not null default 300000 check(request_timeout_ms between 10000 and 3600000),
    updated_at text not null
);

insert into docling_settings (
    id, enabled, conda_environment, default_port, fallback_start_port,
    fallback_end_port, idle_shutdown_ms, request_timeout_ms, updated_at
)
select
    id,
    enabled,
    case when length(trim(conda_environment)) = 0 then 'research' else trim(conda_environment) end,
    default_port,
    fallback_start_port,
    fallback_end_port,
    idle_shutdown_ms,
    request_timeout_ms,
    updated_at
from docling_settings_legacy;

drop table docling_settings_legacy;
