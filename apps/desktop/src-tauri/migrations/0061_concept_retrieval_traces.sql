create virtual table concept_search using fts5(
    concept_id unindexed,
    terms,
    label,
    explanation,
    qualifiers,
    tokenize = 'unicode61 remove_diacritics 2'
);

insert into concept_search(concept_id, terms, label, explanation, qualifiers)
select c.id,
       coalesce(group_concat(t.text, ' '), ''),
       c.label,
       c.explanation,
       trim(c.disambiguation || ' ' || c.domain || ' ' || c.tradition || ' ' || c.period)
from concepts c
left join concept_terms ct on ct.concept_id = c.id
left join terms t on t.id = ct.term_id
group by c.id;

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

create trigger concept_search_term_link_insert
after insert on concept_terms begin
    delete from concept_search where concept_id = new.concept_id;
    insert into concept_search(concept_id, terms, label, explanation, qualifiers)
    select c.id, coalesce(group_concat(t.text, ' '), ''), c.label, c.explanation,
           trim(c.disambiguation || ' ' || c.domain || ' ' || c.tradition || ' ' || c.period)
    from concepts c
    left join concept_terms ct on ct.concept_id = c.id
    left join terms t on t.id = ct.term_id
    where c.id = new.concept_id
    group by c.id;
end;

create trigger concept_search_term_link_delete
after delete on concept_terms begin
    delete from concept_search where concept_id = old.concept_id;
    insert into concept_search(concept_id, terms, label, explanation, qualifiers)
    select c.id, coalesce(group_concat(t.text, ' '), ''), c.label, c.explanation,
           trim(c.disambiguation || ' ' || c.domain || ' ' || c.tradition || ' ' || c.period)
    from concepts c
    left join concept_terms ct on ct.concept_id = c.id
    left join terms t on t.id = ct.term_id
    where c.id = old.concept_id
    group by c.id;
end;

create trigger concept_search_term_update
after update of text on terms begin
    delete from concept_search
    where concept_id in (select concept_id from concept_terms where term_id = new.id);
    insert into concept_search(concept_id, terms, label, explanation, qualifiers)
    select c.id, coalesce(group_concat(t.text, ' '), ''), c.label, c.explanation,
           trim(c.disambiguation || ' ' || c.domain || ' ' || c.tradition || ' ' || c.period)
    from concepts c
    join concept_terms changed on changed.concept_id = c.id and changed.term_id = new.id
    left join concept_terms ct on ct.concept_id = c.id
    left join terms t on t.id = ct.term_id
    group by c.id;
end;

create table concept_retrieval_traces (
    id text primary key,
    paper_id text references bibliographic_items(id) on delete restrict,
    query_sha256 text not null check(length(query_sha256) = 64),
    embedding_space_id text references embedding_spaces(id) on delete restrict,
    index_generation_id text references embedding_index_generations(id) on delete restrict,
    status text not null check(status in ('previewed', 'sent', 'expired', 'failed')),
    config_json text not null check(json_valid(config_json) and json_type(config_json) = 'object'),
    context_text text not null check(length(context_text) <= 120000),
    context_sha256 text not null check(length(context_sha256) = 64),
    seed_count integer not null check(seed_count >= 0),
    edge_count integer not null check(edge_count >= 0),
    source_count integer not null check(source_count >= 0),
    external_paper_count integer not null check(external_paper_count >= 0),
    error_code text check(error_code is null or length(error_code) <= 120),
    created_at text not null,
    expires_at text not null,
    sent_at text,
    check((embedding_space_id is null) = (index_generation_id is null)),
    check(status != 'sent' or sent_at is not null)
);

create table concept_retrieval_trace_seeds (
    trace_id text not null references concept_retrieval_traces(id) on delete cascade,
    concept_id text not null references concepts(id) on delete restrict,
    concept_revision integer not null check(concept_revision >= 1),
    label text not null,
    preferred_term text not null,
    matched_by_json text not null check(json_valid(matched_by_json) and json_type(matched_by_json) = 'array'),
    ordinal integer not null check(ordinal >= 0),
    lexical_rank integer check(lexical_rank is null or lexical_rank >= 1),
    vector_rank integer check(vector_rank is null or vector_rank >= 1),
    vector_distance real check(vector_distance is null or vector_distance >= 0),
    rrf_score real not null check(rrf_score > 0),
    primary key(trace_id, concept_id),
    unique(trace_id, ordinal),
    check(lexical_rank is not null or vector_rank is not null)
);

create table concept_retrieval_trace_edges (
    trace_id text not null references concept_retrieval_traces(id) on delete cascade,
    relation_assertion_id text not null references relation_assertions(id) on delete restrict,
    relation_revision integer not null check(relation_revision >= 1),
    subject_concept_id text not null references concepts(id) on delete restrict,
    subject_label text not null,
    predicate_kind text not null,
    object_concept_id text not null references concepts(id) on delete restrict,
    object_label text not null,
    hop integer not null check(hop between 1 and 2),
    ordinal integer not null check(ordinal >= 0),
    primary key(trace_id, relation_assertion_id),
    unique(trace_id, ordinal)
);

create table concept_retrieval_trace_sources (
    trace_id text not null references concept_retrieval_traces(id) on delete cascade,
    anchor_id text not null references source_anchors(id) on delete restrict,
    concept_id text not null references concepts(id) on delete restrict,
    paper_id text not null references bibliographic_items(id) on delete restrict,
    source_kind text not null check(source_kind in ('definition_claim', 'term_usage')),
    source_subject_id text not null,
    source_subject_revision integer not null check(source_subject_revision >= 1),
    paper_title text not null,
    concept_label text not null,
    page_index integer not null check(page_index >= 0),
    page_label text,
    quote_exact text not null,
    context_text text not null,
    ordinal integer not null check(ordinal >= 0),
    selected integer not null check(selected in (0, 1)),
    omission_reason text check(omission_reason is null or omission_reason in ('source_limit', 'duplicate_anchor')),
    primary key(trace_id, anchor_id, concept_id, source_kind, source_subject_id),
    unique(trace_id, ordinal),
    check(selected = 1 or omission_reason is not null)
);

create index concept_retrieval_traces_created_idx
    on concept_retrieval_traces(created_at desc);
create index concept_retrieval_seeds_trace_idx
    on concept_retrieval_trace_seeds(trace_id, ordinal);
create index concept_retrieval_edges_trace_idx
    on concept_retrieval_trace_edges(trace_id, ordinal);
create index concept_retrieval_sources_trace_idx
    on concept_retrieval_trace_sources(trace_id, selected, ordinal);

alter table paper_ai_reading_sessions
    add column concept_retrieval_default integer not null default 0
        check(concept_retrieval_default in (0, 1));

alter table paper_ai_reading_turns
    add column concept_retrieval_enabled integer not null default 0
        check(concept_retrieval_enabled in (0, 1));

alter table paper_ai_reading_turns
    add column concept_retrieval_trace_id text
        references concept_retrieval_traces(id) on delete restrict;

create unique index paper_ai_reading_turn_trace_idx
    on paper_ai_reading_turns(concept_retrieval_trace_id)
    where concept_retrieval_trace_id is not null;
