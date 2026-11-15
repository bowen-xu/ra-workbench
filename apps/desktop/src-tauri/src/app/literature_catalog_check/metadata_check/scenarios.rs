use super::*;

pub(super) async fn run(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    note: &Value,
    calls: &mut Vec<Value>,
) -> Result<Value, Error> {
    item(
        apply(
            store,
            service,
            key,
            note,
            Request::Confirm {
                item_id: fixture::MAIN.into(),
                field_names: vec!["title".into(), "title".into()],
            },
            calls,
        )
        .await?,
    )?;
    if readback::provenance(store, fixture::MAIN, "title")?
        != ("crossref".into(), Some("original-reference".into()), true)
    {
        return Err("Confirm changed the original provenance source/reference".into());
    }
    let scalar = item(
        apply(
            store,
            service,
            key,
            note,
            Request::Update {
                item_id: fixture::MAIN.into(),
                patch: Patch::Title("  Metadata scalar title  ".into()),
            },
            calls,
        )
        .await?,
    )?;
    if scalar.title != "Metadata scalar title"
        || readback::provenance(store, fixture::MAIN, "title")? != ("user".into(), None, true)
    {
        return Err("Update lost title normalization or confirmed-source policy".into());
    }
    item(
        apply(
            store,
            service,
            key,
            note,
            Request::Update {
                item_id: fixture::MAIN.into(),
                patch: Patch::Volume("  Volume  ".into()),
            },
            calls,
        )
        .await?,
    )?;
    if readback::provenance(store, fixture::MAIN, "volume")? != ("user_edit".into(), None, false) {
        return Err("Update lost the actual unconfirmed-source branch".into());
    }
    let creators = item(
        apply(
            store,
            service,
            key,
            note,
            Request::Update {
                item_id: fixture::MAIN.into(),
                patch: Patch::Creators(vec![CatalogCreator {
                    family_name: " Family ".into(),
                    given_name: " Given ".into(),
                    literal_name: " Literal ".into(),
                    role: " editor ".into(),
                }]),
            },
            calls,
        )
        .await?,
    )?;
    if creators.creators.len() != 1
        || creators.creators[0]
            != (CatalogCreator {
                family_name: "Family".into(),
                given_name: "Given".into(),
                literal_name: "Literal".into(),
                role: " editor ".into(),
            })
    {
        return Err("Update changed creator trim/role semantics".into());
    }
    let tags = item(
        apply(
            store,
            service,
            key,
            note,
            Request::Update {
                item_id: fixture::MAIN.into(),
                patch: Patch::Tags(vec![" beta ".into(), "alpha".into(), " ".into()]),
            },
            calls,
        )
        .await?,
    )?;
    if tags.tags != ["alpha", "beta"] {
        return Err("Update lost actual tag relations".into());
    }
    let doi = item(
        apply(
            store,
            service,
            key,
            note,
            Request::Update {
                item_id: fixture::MAIN.into(),
                patch: Patch::Doi(" \t ".into()),
            },
            calls,
        )
        .await?,
    )?;
    if !doi.doi.is_empty()
        || readback::value::<i64>(
            store,
            "select count(*) from identifiers where item_id='metadata-cli-main' and kind='doi'",
        )? != 0
    {
        return Err("empty DOI update inserted a normalized-empty identifier".into());
    }
    let full = full(store, service, key, note, calls).await?;
    let fields = fields(store, service, key, note, calls).await?;
    fixture::candidate(
        store,
        "metadata-cli-reject-null",
        Some(fixture::MAIN),
        "null",
        false,
    )?;
    let response = apply(
        store,
        service,
        key,
        note,
        Request::Reject {
            candidate_id: "metadata-cli-reject-null".into(),
        },
        calls,
    )
    .await?;
    if response != (Response::Done {})
        || readback::candidate_state(store, "metadata-cli-reject-null")? != "rejected"
    {
        return Err("Reject parsed a non-Draft JSON or lost the null-result branch".into());
    }
    Ok(
        json!({"scalarRelationsEmptyDoi":true,"confirmSourceReferencePreserved":true,
        "full":full,"fields":fields,"rejectNonDraftJsonNullResult":true}),
    )
}

async fn full(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    note: &Value,
    calls: &mut Vec<Value>,
) -> Result<Value, Error> {
    let candidate = fixture::draft(" Full candidate ");
    fixture::candidate(
        store,
        "metadata-cli-full",
        None,
        &serde_json::to_string(&candidate)?,
        true,
    )?;
    let mut draft = candidate;
    draft.id = Some("metadata-cli-caller-must-not-create".into());
    draft.source_kind = Some("user".into());
    draft.title = " Full user ".into();
    let result = item(
        apply(
            store,
            service,
            key,
            note,
            Request::FullApply {
                candidate_id: "metadata-cli-full".into(),
                draft: Box::new(draft),
            },
            calls,
        )
        .await?,
    )?;
    if result.id == "metadata-cli-caller-must-not-create"
        || result.title != "Full user"
        || readback::value::<i64>(
            store,
            "select count(*) from bibliographic_items where id='metadata-cli-caller-must-not-create'",
        )? != 0
        || readback::candidate_state(store, "metadata-cli-full")? != "applied"
        || readback::provenance(store, &result.id, "title")?
            != ("user".into(), Some("candidate-reference".into()), true)
        || readback::provenance(store, &result.id, "itemType")?
            != ("crossref".into(), Some("candidate-reference".into()), true)
        || readback::value::<String>(
            store,
            "select state from import_jobs where id='metadata-cli-full-job'",
        )? != "completed"
        || readback::value::<String>(
            store,
            "select item_id from import_results where id='metadata-cli-full-result'",
        )? != result.id
    {
        return Err(
            "FullApply lost header ownership, raw field-source comparison or acceptance".into(),
        );
    }
    Ok(json!({"itemId":result.id,"noneHeaderCreatedNewItem":true,"callerIdSourceOverridden":true}))
}

async fn fields(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    note: &Value,
    calls: &mut Vec<Value>,
) -> Result<Value, Error> {
    let mut draft = fixture::draft("Fields baseline");
    draft.volume = " Candidate Volume ".into();
    draft.issue = " Candidate Issue ".into();
    fixture::candidate(
        store,
        "metadata-cli-review",
        Some(fixture::FIELDS),
        &serde_json::to_string(&draft)?,
        false,
    )?;
    store.connection()?.execute_batch("create temp trigger synthetic_metadata_cli_order after insert on metadata_provenance when new.item_id='metadata-cli-fields' begin insert into synthetic_metadata_cli_events(phase) values(new.field_name);end;")?;
    let reviewed = item(
        apply(
            store,
            service,
            key,
            note,
            Request::FieldsApply {
                candidate_id: "metadata-cli-review".into(),
                decisions: vec![
                    Decision {
                        field_name: "volume".into(),
                        choice: Choice::Candidate,
                    },
                    Decision {
                        field_name: "issue".into(),
                        choice: Choice::Candidate,
                    },
                ],
                candidate_trust: Trust::Reviewable,
            },
            calls,
        )
        .await?,
    )?;
    store
        .connection()?
        .execute_batch("drop trigger synthetic_metadata_cli_order;")?;
    if reviewed.volume != "Candidate Volume"
        || reviewed.issue != "Candidate Issue"
        || readback::value::<String>(
            store,
            "select group_concat(phase,',') from (select phase from synthetic_metadata_cli_events order by seq)",
        )? != "volume,issue"
        || readback::provenance(store, fixture::FIELDS, "volume")?
            != ("crossref".into(), Some("candidate-reference".into()), false)
    {
        return Err("FieldsApply changed input order or reviewable source policy".into());
    }
    let mut trusted = draft_from_item(&reviewed)?;
    trusted.pages = " Trusted pages ".into();
    fixture::candidate(
        store,
        "metadata-cli-trusted",
        Some(fixture::FIELDS),
        &serde_json::to_string(&trusted)?,
        false,
    )?;
    let current = item(
        apply(
            store,
            service,
            key,
            note,
            Request::FieldsApply {
                candidate_id: "metadata-cli-trusted".into(),
                decisions: vec![Decision {
                    field_name: "pages".into(),
                    choice: Choice::Candidate,
                }],
                candidate_trust: Trust::Trusted,
            },
            calls,
        )
        .await?,
    )?;
    if current.pages != "Trusted pages"
        || readback::provenance(store, fixture::FIELDS, "pages")?
            != ("crossref".into(), Some("candidate-reference".into()), true)
    {
        return Err("FieldsApply lost trusted confirmation".into());
    }
    let mut alternative = draft_from_item(&current)?;
    alternative.item_type = "unsupported-but-unselected".into();
    alternative.title = " Selected title ".into();
    alternative.pages = "Unselected alternative pages".into();
    fixture::candidate(
        store,
        "metadata-cli-current",
        Some(fixture::FIELDS),
        &serde_json::to_string(&alternative)?,
        false,
    )?;
    let selected = item(
        apply(
            store,
            service,
            key,
            note,
            Request::FieldsApply {
                candidate_id: "metadata-cli-current".into(),
                decisions: vec![
                    Decision {
                        field_name: "title".into(),
                        choice: Choice::Candidate,
                    },
                    Decision {
                        field_name: "itemType".into(),
                        choice: Choice::Current,
                    },
                    Decision {
                        field_name: "pages".into(),
                        choice: Choice::Current,
                    },
                ],
                candidate_trust: Trust::Reviewable,
            },
            calls,
        )
        .await?,
    )?;
    if selected.item_type != "journalArticle"
        || selected.title != "Selected title"
        || selected.pages != "Trusted pages"
        || readback::provenance(store, fixture::FIELDS, "itemType")? != ("user".into(), None, true)
        || readback::provenance(store, fixture::FIELDS, "pages")?
            != ("crossref".into(), Some("candidate-reference".into()), true)
    {
        return Err(
            "Current choice validated an unselected invalid candidate or lost fallback provenance"
                .into(),
        );
    }
    Ok(
        json!({"inputOrder":["volume","issue"],"reviewableTrustedCurrent":true,"unselectedInvalidItemTypeAccepted":true}),
    )
}
