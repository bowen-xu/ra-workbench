insert into llm_provider_configs
    (provider_id, enabled, base_url, timeout_ms, updated_at)
values
    ('openai', 1, '', 120000, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));

-- ai_runs still references the legacy provider table. Keep a compatibility row
-- until that historical table is removed in a dedicated schema migration; live
-- OpenAI configuration is read exclusively from llm_provider_configs.
pragma ignore_check_constraints = on;
insert into ai_provider_settings
    (provider_id, provider_kind, enabled, base_url, region, model_id, thinking_mode,
     timeout_ms, updated_at)
values
    ('openai', 'generation', 0, '', '', '', 'disabled', 120000,
     strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));
pragma ignore_check_constraints = off;

alter table llm_model_routes add column reasoning_effort text;
alter table ai_runs add column reasoning_effort text;

insert into llm_model_routes
    (route_kind, route_id, model_kind, provider_id, model_id, updated_at)
select 'default', 'text', 'text', 'deepseek', model_id,
       strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
from ai_provider_settings
where provider_id = 'deepseek' and enabled = 1 and length(trim(model_id)) > 0
  and not exists (
      select 1 from llm_model_routes
      where route_kind = 'default' and route_id = 'text'
  );
