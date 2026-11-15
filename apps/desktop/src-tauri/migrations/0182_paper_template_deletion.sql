-- A paper-owned template must never become a global template after paper purge.
-- Remove dependents in the same DELETE statement before the owner FK cascades.
create trigger bibliographic_structured_resources_deleted before delete on bibliographic_items begin
    delete from ai_resource_template_runs where paper_id=old.id;
    delete from structured_resources where paper_id=old.id;
    delete from structured_resources where id in (
        select template_id from structured_paper_template_owners where paper_id=old.id
    );
end;
