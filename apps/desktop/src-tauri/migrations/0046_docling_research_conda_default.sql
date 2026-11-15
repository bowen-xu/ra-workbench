alter table docling_settings
add column conda_environment text not null default 'research'
check(length(conda_environment) <= 200);

update docling_settings
set conda_environment = 'research',
    executable = 'docling-serve',
    updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
where id = 1
  and executable in ('docling-serve', 'conda run -n research docling-serve');
