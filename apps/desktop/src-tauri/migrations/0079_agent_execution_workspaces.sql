create table agent_execution_workspaces (
    conversation_id text not null,
    request_id text not null,
    turn_id text not null unique,
    context_snapshot_id text not null,
    manuscript_id text not null,
    lease_id text not null unique,
    manifest_hash text not null check (length(manifest_hash) = 64),
    manifest_json text not null check (json_valid(manifest_json)),
    policy text not null default 'offline_workspace_v1' check (policy = 'offline_workspace_v1'),
    state text not null default 'prepared' check (state in ('prepared', 'approved', 'revoked')),
    primary key (conversation_id, request_id),
    foreign key (conversation_id, request_id) references agent_submissions(conversation_id, request_id) on delete cascade
);

create trigger agent_execution_workspace_terminal
after insert on agent_items
when json_extract(new.payload_json, '$.payload.type') = 'turnFinished'
begin
    update agent_execution_workspaces set state = 'revoked'
    where conversation_id = new.conversation_id
      and turn_id = json_extract(new.payload_json, '$.payload.turnId');
end;
