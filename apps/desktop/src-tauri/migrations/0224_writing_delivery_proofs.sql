-- A verified delivery must retain its original evidence after a later editor
-- operation replaces the current checkpoint receipt. This does not grant a
-- new workspace or permission to replay the application.
create table writing_agent_delivery_proofs (
    delivery_id text primary key check(length(delivery_id)=36),
    conversation_id text not null check(length(conversation_id)=36),
    request_id text not null check(length(request_id)=36),
    turn_id text not null check(length(turn_id)=36),
    context_snapshot_id text not null check(length(context_snapshot_id)=36),
    manuscript_id text not null references manuscripts(id) on delete cascade,
    lease_id text not null check(length(lease_id)=36),
    manifest_hash text not null check(length(manifest_hash)=64),
    proof_json text not null check(json_valid(proof_json)),
    unique(conversation_id, request_id, lease_id)
);
create trigger writing_agent_delivery_proofs_immutable
before update on writing_agent_delivery_proofs
begin select raise(abort, 'delivery proof is immutable'); end;
create trigger writing_agent_delivery_proofs_no_delete
before delete on writing_agent_delivery_proofs
when exists(select 1 from manuscripts where id=old.manuscript_id)
begin select raise(abort, 'delivery proof cannot be deleted'); end;
