create table llm_provider_configs (
    provider_id text primary key,
    enabled integer not null default 0 check(enabled in (0, 1)),
    base_url text not null default '',
    timeout_ms integer not null default 60000 check(timeout_ms between 1000 and 300000),
    updated_at text not null
);

insert into llm_provider_configs
    (provider_id, enabled, base_url, timeout_ms, updated_at)
select 'deepseek', enabled,
       case when rtrim(base_url, '/') in ('https://api.deepseek.com', 'https://api.deepseek.com/v1')
            then '' else base_url end,
       timeout_ms, updated_at
from ai_provider_settings where provider_id = 'deepseek';

insert into llm_provider_configs
    (provider_id, enabled, base_url, timeout_ms, updated_at)
select 'qwen', enabled, base_url, timeout_ms, updated_at
from ai_provider_settings where provider_id = 'qwen-embedding';

create table llm_model_routes (
    route_kind text not null check(route_kind in ('default', 'use_case')),
    route_id text not null,
    model_kind text not null check(model_kind in ('text', 'embedding', 'vision')),
    provider_id text not null references llm_provider_configs(provider_id) on delete restrict,
    model_id text not null check(length(trim(model_id)) between 1 and 240),
    updated_at text not null,
    primary key(route_kind, route_id)
);

create index llm_model_routes_provider_idx on llm_model_routes(provider_id);
