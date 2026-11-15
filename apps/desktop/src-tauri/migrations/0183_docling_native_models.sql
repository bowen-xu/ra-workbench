-- Preserve historical Conda settings and immutable parse runs; the native runtime ignores Conda.
alter table docling_settings add column model_manifest_url text not null
    default 'http://127.0.0.1:4174/models/docling/latest.json';
