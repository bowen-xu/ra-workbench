-- Exact user approvals for host-owned MCP server revisions.
-- A revision change requires a new approval; this table never stores secrets.
create table agent_mcp_approvals (
    server_id text primary key,
    environment_id text not null,
    revision text not null,
    approved_at text not null default (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
