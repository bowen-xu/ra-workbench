-- Freeze the MCP server allowlist with each child task.  Existing tasks were
-- created with no child MCP permission and remain intentionally restricted.
alter table agent_subtasks add column approved_servers_json text not null default '[]';
