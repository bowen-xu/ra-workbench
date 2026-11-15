-- Classification is explicit and tied to concept identity, not its label.
create table nal_concept_kinds (
    concept_id text primary key references concepts(id) on delete cascade,
    kind text not null check(kind in ('ordinary','relation'))
);
-- Backfill roles established by the pre-type relation model; names alone do not infer a role.
insert into nal_concept_kinds(concept_id,kind)
select distinct c.id,'relation' from concepts c where exists (
    select 1 from nal_statements s
    where json_extract(s.subject,'$.kind')='product'
      and json_extract(s.predicate,'$.kind')='concept'
      and json_extract(s.predicate,'$.id')=c.id
) or exists (select 1 from nal_relation_rules r where r.relation_concept_id=c.id);
