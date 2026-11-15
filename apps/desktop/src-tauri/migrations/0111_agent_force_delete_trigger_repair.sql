-- Rebuild force-delete triggers for databases that applied an earlier 0110
-- definition. The marker table is intentionally preserved.
create table if not exists agent_force_delete_markers (
    conversation_id text primary key,
    request_id text not null unique
);

drop trigger agent_workspace_discard_retained;
create trigger agent_workspace_discard_retained
before delete on agent_workspace_discards
when not exists(select 1 from agent_force_delete_markers m
    where m.conversation_id=old.conversation_id)
begin
    select raise(abort, 'retained workspace discard');
end;

drop trigger agent_workspace_discard_preserves_pending_binding;
create trigger agent_workspace_discard_preserves_pending_binding
before delete on agent_execution_workspaces
when exists(select 1 from agent_workspace_discards d
    where d.conversation_id = old.conversation_id and d.request_id = old.request_id
      and d.state = 'pending')
  and not exists(select 1 from agent_force_delete_markers m
    where m.conversation_id=old.conversation_id)
begin
    select raise(abort, 'pending workspace discard');
end;

drop trigger agent_proposal_applications_active_delete;
create trigger agent_proposal_applications_active_delete
before delete on agent_proposal_applications
when old.status in ('prepared','applying','recovery_required')
  and not exists(select 1 from agent_force_delete_markers m
    join agent_change_sets c on c.conversation_id=m.conversation_id
    where c.id=old.change_set_id)
begin
    select raise(abort,'unresolved proposal application');
end;

drop trigger agent_proposal_application_files_active_delete;
create trigger agent_proposal_application_files_active_delete
before delete on agent_proposal_application_files
when exists(select 1 from agent_proposal_applications where id=old.application_id
    and status in ('prepared','applying','recovery_required'))
  and not exists(select 1 from agent_force_delete_markers m
    join agent_change_sets c on c.conversation_id=m.conversation_id
    join agent_proposal_applications a on a.change_set_id=c.id
    where a.id=old.application_id)
begin
    select raise(abort,'unresolved application target');
end;

drop trigger agent_proposal_undos_active_delete;
create trigger agent_proposal_undos_active_delete
before delete on agent_proposal_undos
when exists(select 1 from agent_proposal_applications where id=old.operation_id
    and status in ('prepared','applying','recovery_required'))
  and not exists(select 1 from agent_force_delete_markers m
    join agent_change_sets c on c.conversation_id=m.conversation_id
    join agent_proposal_applications a on a.change_set_id=c.id
    where a.id=old.operation_id)
begin
    select raise(abort,'unresolved undo origin');
end;

drop trigger agent_proposal_undo_sources_active_delete;
create trigger agent_proposal_undo_sources_active_delete
before delete on agent_proposal_undo_sources
when exists(select 1 from agent_proposal_applications where id=old.operation_id
    and status in ('prepared','applying','recovery_required'))
  and not exists(select 1 from agent_force_delete_markers m
    join agent_change_sets c on c.conversation_id=m.conversation_id
    join agent_proposal_applications a on a.change_set_id=c.id
    where a.id=old.operation_id)
begin
    select raise(abort,'unresolved undo source');
end;
