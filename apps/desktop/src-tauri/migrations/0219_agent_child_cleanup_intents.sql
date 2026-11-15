-- Keep the original cleanup identity even if the archive transaction becomes
-- uncertain and the parent turn cannot persist its failure event.
create table agent_child_cleanup_intents (
    child_id text primary key not null,
    request_id text not null,
    prepare_operation_id text not null unique,
    archive_operation_id text not null unique,
    expected_watermark integer not null check(expected_watermark >= 0 and expected_watermark <= 9007199254740991),
    epoch text not null,
    version integer not null check(version = 1)
);
