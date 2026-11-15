-- Application-owned Git worktrees are durable inventory, not disposable paths.
-- Recovery marks uncertainty; it never authorizes automatic deletion.
create table agent_git_workspaces (
    id text primary key,
    manuscript_id text not null references manuscripts(id) on delete restrict,
    repository_root text not null,
    worktree_path text not null unique,
    revision text not null,
    state text not null default 'active' check(state in ('active','released','recovery_required')),
    created_at text not null default (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    released_at text
);
create index agent_git_workspaces_manuscript_idx
    on agent_git_workspaces(manuscript_id, created_at desc, id desc);
