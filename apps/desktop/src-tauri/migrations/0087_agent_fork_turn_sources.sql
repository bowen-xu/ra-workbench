-- A provenance index, never a copied submission or an execution admission.
create table agent_fork_turn_sources (
    conversation_id text not null references agent_conversations(id) on delete cascade,
    turn_id text not null,
    source_conversation_id text not null references agent_conversations(id) on delete restrict,
    context_snapshot_id text not null,
    primary key (conversation_id, turn_id),
    check (conversation_id != source_conversation_id)
);
create index agent_fork_turn_sources_origin on agent_fork_turn_sources(source_conversation_id);

-- Existing branches inherit original UUIDs. Find the physical ancestor owning
-- each frozen input, bounded by lineage; history was materialized at creation.
with recursive ancestors(child_id, ancestor_id) as (
    select child_id, parent_id from agent_conversation_forks
    union
    select a.child_id, f.parent_id from ancestors a
    join agent_conversation_forks f on f.child_id = a.ancestor_id
)
insert into agent_fork_turn_sources(conversation_id,turn_id,source_conversation_id,context_snapshot_id)
select distinct e.conversation_id, json_extract(e.payload_json,'$.payload.turnId'),
    s.conversation_id, s.context_snapshot_id
from agent_items e
join ancestors a on a.child_id=e.conversation_id
join agent_submissions s on s.conversation_id=a.ancestor_id
    and s.turn_id=json_extract(e.payload_json,'$.payload.turnId')
    and s.context_snapshot_id=json_extract(e.payload_json,'$.payload.contextSnapshotId')
where json_extract(e.payload_json,'$.payload.type')='turnStarted';
