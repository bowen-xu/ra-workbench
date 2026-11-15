-- A new compensation owns current CAS; the acknowledged Undo remains immutable.
-- Existing restoration identities never regain dispatch permission.
drop trigger agent_editor_restorations_admission;
create trigger agent_editor_restorations_admission
before insert on agent_editor_restorations
when new.result_revision is not null or not exists(
    select 1 from agent_proposal_application_files f
    join agent_proposal_applications a on a.id=f.application_id
    where f.application_id=new.application_id and f.relative_path=new.relative_path
      and a.status='recovery_required' and f.target_kind='editor' and f.phase='acknowledged'
      and ((f.expected_editor_session=new.editor_session and f.result_editor_revision=new.expected_revision)
        or (a.direction='undo' and f.expected_editor_session!=new.editor_session
          and exists(select 1 from agent_proposal_undos u
            join agent_proposal_applications original on original.id=u.original_operation_id
            join agent_change_sets source on source.id=a.change_set_id
            join writing_document_checkpoints c on c.manuscript_id=source.manuscript_id and c.relative_path=f.relative_path
            where u.operation_id=a.id and original.direction='apply' and original.status='applied'
              and json_extract(c.checkpoint_json,'$.saveIntent') is null
              and json_extract(c.checkpoint_json,'$.editorReceipt.operationId')=a.id
              and json_extract(c.checkpoint_json,'$.editorReceipt.sessionId')=f.expected_editor_session
              and json_extract(c.checkpoint_json,'$.editorReceipt.revision')=f.result_editor_revision
              and json_extract(c.checkpoint_json,'$.editorReceipt.text')=json_extract(c.checkpoint_json,'$.content'))))
)
begin select raise(abort,'editor restoration requires confirmed edit and recovery owner'); end;
