-- Explanations are optional; deleted concepts retain source/history identities.
-- The migration runner disables foreign keys only for this atomic table rebuild.
create table concepts_backup_0170 as select id,label,explanation,disambiguation,domain,tradition,period,provenance_kind,revision,created_at,updated_at from concepts;
drop table concepts;
create table concepts (
    id text primary key,
    label text not null default '' check(length(label) <= 500),
    explanation text not null default '',
    disambiguation text not null default '' check(length(disambiguation) <= 1000),
    domain text not null default '' check(length(domain) <= 500),
    tradition text not null default '' check(length(tradition) <= 500),
    period text not null default '' check(length(period) <= 200),
    provenance_kind text not null default 'human_original'
        check(provenance_kind in ('human_original', 'ai_accepted_unchanged', 'ai_accepted_after_edit')),
    revision integer not null default 1 check(revision >= 1),
    created_at text not null,
    updated_at text not null,
    deleted_at text
);
insert into concepts (id,label,explanation,disambiguation,domain,tradition,period,provenance_kind,revision,created_at,updated_at) select id,label,explanation,disambiguation,domain,tradition,period,provenance_kind,revision,created_at,updated_at from concepts_backup_0170;
drop table concepts_backup_0170;
create index concepts_updated_idx on concepts(updated_at desc);

create trigger concept_search_concept_insert
after insert on concepts begin
    insert into concept_search(concept_id, terms, label, explanation, qualifiers)
    values (new.id, '', new.label, new.explanation,
            trim(new.disambiguation || ' ' || new.domain || ' ' || new.tradition || ' ' || new.period));
end;

create trigger concept_search_concept_update
after update of label, explanation, disambiguation, domain, tradition, period on concepts begin
    delete from concept_search where concept_id = new.id;
    insert into concept_search(concept_id, terms, label, explanation, qualifiers)
    select c.id, coalesce(group_concat(t.text, ' '), ''), c.label, c.explanation,
           trim(c.disambiguation || ' ' || c.domain || ' ' || c.tradition || ' ' || c.period)
    from concepts c
    left join concept_terms ct on ct.concept_id = c.id
    left join terms t on t.id = ct.term_id
    where c.id = new.id
    group by c.id;
end;

create trigger concept_search_concept_delete
after delete on concepts begin
    delete from concept_search where concept_id = old.id;
end;

create trigger collection_concept_deleted before delete on concepts
begin
    update collection_state set revision = revision + 1 where id = 1;
end;

create trigger nal_concept_created after insert on concepts begin
    update nal_state set revision = revision + 1 where id = 1;
end;

create trigger nal_concept_updated after update on concepts begin
    update nal_state set revision = revision + 1 where id = 1;
end;

create trigger nal_concept_deleted before delete on concepts begin
    delete from nal_statements where exists (
        select 1 from json_tree(subject) j where j.key = 'id' and j.value = old.id
        and json_extract(subject, substr(j.fullkey,1,length(j.fullkey)-3) || '.kind') = 'concept'
    ) or exists (
        select 1 from json_tree(predicate) j where j.key = 'id' and j.value = old.id
        and json_extract(predicate, substr(j.fullkey,1,length(j.fullkey)-3) || '.kind') = 'concept'
    );
    update nal_state set revision = revision + 1 where id = 1;
end;

create index concepts_name_lookup on concepts(trim(label));
create trigger concepts_unique_name_insert before insert on concepts
when new.deleted_at is null and exists (select 1 from concepts where deleted_at is null and trim(label)=trim(new.label))
begin
    select raise(abort, '概念名称已存在，请使用不同名称，或用 ` 添加语境');
end;
create trigger concepts_unique_name_update before update of label,deleted_at on concepts
when new.deleted_at is null and (trim(new.label)<>trim(old.label) or old.deleted_at is not null)
 and exists (select 1 from concepts where id<>old.id and deleted_at is null and trim(label)=trim(new.label))
begin
    select raise(abort, '概念名称已存在，请使用不同名称，或用 ` 添加语境');
end;
create trigger nal_concept_soft_deleted after update of deleted_at on concepts
when old.deleted_at is null and new.deleted_at is not null
begin
    delete from nal_annotations where concept_id=new.id;
    delete from nal_relation_rules where relation_concept_id=new.id;
    delete from nal_statements where exists (
        select 1 from json_tree(subject) j where j.key='id' and j.value=new.id
        and json_extract(subject, substr(j.fullkey,1,length(j.fullkey)-3)||'.kind')='concept'
    ) or exists (
        select 1 from json_tree(predicate) j where j.key='id' and j.value=new.id
        and json_extract(predicate, substr(j.fullkey,1,length(j.fullkey)-3)||'.kind')='concept'
    );
end;
