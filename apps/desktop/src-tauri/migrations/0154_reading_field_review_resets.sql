-- Field review resets are append-only metadata; they preserve reading text and prior audit rows.
create table ai_overview_field_review_resets (
    id text primary key,
    field_confirmation_id text not null unique references ai_overview_field_confirmations(id) on delete restrict,
    idempotency_key text not null unique check(length(idempotency_key) between 1 and 200),
    created_at text not null
);
create trigger ai_overview_field_review_resets_no_update
before update on ai_overview_field_review_resets begin
    select raise(abort, 'reading field review resets are immutable');
end;
create trigger ai_overview_field_review_resets_no_delete
before delete on ai_overview_field_review_resets begin
    select raise(abort, 'reading field review resets are immutable');
end;
