-- Execution consent is scoped to the bound chat host, independent of manuscripts.
create table agent_host_shell_grants (
 conversation_id text primary key references agent_conversations(id) on delete cascade,
 host_kind text not null,
 resource_id text not null,
 revision text not null
);
create table agent_turn_shell_grants (
 turn_id text primary key references agent_submissions(turn_id) on delete cascade,
 grant_revision text not null
);
create table agent_host_execution_workspaces (
 turn_id text primary key references agent_submissions(turn_id) on delete cascade,
 manifest_json text not null,
 grant_revision text not null,
 created_at text not null default (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
