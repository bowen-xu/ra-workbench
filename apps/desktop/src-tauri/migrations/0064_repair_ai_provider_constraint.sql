create table ai_provider_settings_next (
    provider_id text primary key check(provider_id in (
        'fixture-generation', 'deepseek', 'qwen-embedding', 'openai', 'openai_api'
    )),
    provider_kind text not null check(provider_kind in ('generation', 'embedding')),
    enabled integer not null default 0 check(enabled in (0, 1)),
    base_url text not null default '',
    region text not null default '',
    model_id text not null default '',
    thinking_mode text not null default 'disabled'
        check(thinking_mode in ('disabled', 'enabled')),
    timeout_ms integer not null default 60000 check(timeout_ms between 1000 and 300000),
    updated_at text not null
);

insert into ai_provider_settings_next
    (provider_id, provider_kind, enabled, base_url, region, model_id, thinking_mode,
     timeout_ms, updated_at)
select provider_id, provider_kind, enabled, base_url, region, model_id, thinking_mode,
       timeout_ms, updated_at
from ai_provider_settings;

drop table ai_provider_settings;
alter table ai_provider_settings_next rename to ai_provider_settings;
