-- External lifecycle operations must invalidate classification impact previews too.
create trigger collection_item_deleted before delete on bibliographic_items
begin
    update collection_state set revision = revision + 1 where id = 1;
end;
create trigger collection_concept_deleted before delete on concepts
begin
    update collection_state set revision = revision + 1 where id = 1;
end;
create trigger collection_assignment_requires_memberships before insert on item_relation_assignments
begin
    select case when not exists (
        select 1 from collection_relations r
        join collection_memberships a on a.collection_id = r.source_id and a.item_id = new.item_id
        join collection_memberships b on b.collection_id = r.target_id and b.item_id = new.item_id
        where r.id = new.relation_id
    ) then raise(abort, 'collection endpoints require membership') end;
end;
create trigger collection_membership_removes_assignments before delete on collection_memberships
begin
    delete from item_relation_assignments
    where item_id = old.item_id and relation_id in (
        select id from collection_relations where source_id = old.collection_id or target_id = old.collection_id
    );
end;
