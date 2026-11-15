-- Exact request/receipt replay prevents duplicate copies after an IPC failure.
create table note_conflict_resolutions (
 request_id text primary key,
 request_json text not null check(json_valid(request_json)),
 result_json text not null check(json_valid(result_json))
);
