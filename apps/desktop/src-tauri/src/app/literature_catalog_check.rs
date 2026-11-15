//! Exercise the production catalog service with an independent package and Core DB.
//! Only synthetic data in a freshly allocated repository .tmp directory is used.
use crate::{
    core_sources,
    runtime_events::RuntimeEvents,
    runtime_plugins::{RuntimePluginCatalog, literature_trace::service::LiteratureTraceService},
    storage::{AppStore, AppStorePaths, AppStorePorts},
};
use research_workbench_native_sdk::catalog::{CatalogItem, CatalogRequest};
use research_workbench_native_sdk::{
    literature_trace_protocol::{SnapshotInput, TraceDecision},
    retrieval_trace::TraceRejection,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{path::Path, sync::Arc};
type Error = Box<dyn std::error::Error>;
#[path = "literature_catalog_check/catalog_reads.rs"]
mod catalog_reads;
#[path = "literature_catalog_check/context_metadata_fixture.rs"]
mod context_metadata_fixture;
use catalog_reads::check;
#[path = "literature_catalog_check/attachment_check/entry.rs"]
mod attachment_check;
#[path = "literature_catalog_check/doi_check.rs"]
mod doi_check;
#[path = "literature_catalog_check/duplicate_merge_check.rs"]
mod duplicate_merge_check;
#[path = "literature_catalog_check/duplicate_review_check.rs"]
mod duplicate_review_check;
#[path = "literature_catalog_check/duplicate_undo_check.rs"]
mod duplicate_undo_check;
#[path = "literature_catalog_check/fixture.rs"]
mod fixture;
#[path = "literature_catalog_check/import_jobs_check.rs"]
mod import_jobs_check;
#[path = "literature_catalog_check/lifecycle_check.rs"]
mod lifecycle_check;
#[path = "literature_catalog_check/metadata_check.rs"]
mod metadata_check;
#[path = "literature_catalog_check/nal_annotation_check.rs"]
mod nal_annotation_check;
#[path = "literature_catalog_check/nal_bootstrap_check.rs"]
mod nal_bootstrap_check;
#[path = "literature_catalog_check/nal_bootstrap_check/fixture.rs"]
mod nal_bootstrap_fixture;
#[path = "literature_catalog_check/nal_bootstrap_check/readback.rs"]
mod nal_bootstrap_readback;
#[path = "literature_catalog_check/nal_mutation_check.rs"]
mod nal_mutation_check;
#[path = "literature_catalog_check/nal_query_check.rs"]
mod nal_query_check;
#[path = "literature_catalog_check/reference_records_check.rs"]
mod reference_records_check;
#[path = "literature_catalog_check/save_check.rs"]
mod save_check;
#[path = "literature_catalog_check/structured_hbs_fixture.rs"]
mod structured_hbs_fixture;
#[path = "literature_catalog_check/structured_source_check.rs"]
mod structured_source_check;
#[path = "literature_catalog_check/trash_cleanup_check.rs"]
mod trash_cleanup_check;
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if let Some(request) = context_metadata_fixture::argument(&arguments)? {
        return context_metadata_fixture::run(request);
    }
    if let Some(request) = structured_source_check::argument(&arguments)? {
        return structured_source_check::run(request);
    }

    if let Some(package_directory) = structured_hbs_fixture::package_argument(&arguments)? {
        return structured_hbs_fixture::run(package_directory);
    }
    let save_mode = arguments.len() == 2 && arguments[1] == "--save";
    let reopen = arguments.len() == 3 && arguments[1] == "--reopen";
    let nal_queries = arguments.len() == 2 && arguments[1] == "--nal-queries";
    let reopen_nal_queries = arguments.len() == 3 && arguments[1] == "--reopen-nal-queries";
    let nal_bootstrap = arguments.len() == 2 && arguments[1] == "--nal-bootstrap";
    let reopen_nal_bootstrap = arguments.len() == 3 && arguments[1] == "--reopen-nal-bootstrap";
    let nal_annotation = arguments.len() == 2 && arguments[1] == "--nal-annotation";
    let reopen_nal_annotation = arguments.len() == 3 && arguments[1] == "--reopen-nal-annotation";
    let nal_mutation = arguments.len() == 2 && arguments[1] == "--nal-mutation";
    let reopen_nal_mutation = arguments.len() == 3 && arguments[1] == "--reopen-nal-mutation";
    let duplicate_review = arguments.len() == 2 && arguments[1] == "--duplicate-review";
    let reopen_duplicate_review =
        arguments.len() == 3 && arguments[1] == "--reopen-duplicate-review";
    let duplicate_undo = arguments.len() == 2 && arguments[1] == "--duplicate-undo";
    let reopen_duplicate_undo = arguments.len() == 3 && arguments[1] == "--reopen-duplicate-undo";
    let undo_mode = duplicate_undo || reopen_duplicate_undo;
    let duplicate_merge = arguments.len() == 2 && arguments[1] == "--duplicate-merge";
    let reopen_duplicate_merge = arguments.len() == 3 && arguments[1] == "--reopen-duplicate-merge";
    let merge_mode = duplicate_merge || reopen_duplicate_merge;
    let lifecycle = arguments.len() == 2 && arguments[1] == "--lifecycle";
    let reopen_lifecycle = arguments.len() == 3 && arguments[1] == "--reopen-lifecycle";
    let lifecycle_mode = lifecycle || reopen_lifecycle;
    let trash_cleanup = arguments.len() == 2 && arguments[1] == "--trash-cleanup";
    let reopen_trash_cleanup = arguments.len() == 3 && arguments[1] == "--reopen-trash-cleanup";
    let cleanup_mode = trash_cleanup || reopen_trash_cleanup;
    let metadata_mode = metadata_check::Mode::parse(&arguments);
    let import_jobs_mode = import_jobs_check::Mode::parse(&arguments);
    let doi_mode = doi_check::Mode::parse(&arguments);
    let reference_mode = reference_records_check::Mode::parse(&arguments);
    let attachment_mode = attachment_check::Mode::parse(&arguments);
    let reopen_any = reopen
        || reopen_nal_queries
        || reopen_nal_bootstrap
        || reopen_nal_annotation
        || reopen_nal_mutation
        || reopen_duplicate_review
        || reopen_duplicate_undo
        || reopen_duplicate_merge
        || reopen_lifecycle
        || reopen_trash_cleanup
        || metadata_mode.reopen()
        || import_jobs_mode.reopen()
        || doi_mode.reopen()
        || reference_mode.reopen()
        || attachment_mode.reopen();
    if arguments.len() != 1
        && !save_mode
        && !reopen_any
        && !nal_queries
        && !nal_bootstrap
        && !nal_annotation
        && !nal_mutation
        && !duplicate_review
        && !duplicate_undo
        && !duplicate_merge
        && !lifecycle
        && !trash_cleanup
        && !metadata_mode.fresh()
        && !import_jobs_mode.fresh()
        && !doi_mode.fresh()
        && !reference_mode.fresh()
        && !attachment_mode.fresh()
    {
        return Err(
            "usage: literature-catalog-check <controlled-package-directory> [--save | --reopen <synthetic-run> | --nal-queries | --reopen-nal-queries <synthetic-run> | --nal-bootstrap | --reopen-nal-bootstrap <synthetic-run> | --nal-annotation | --reopen-nal-annotation <synthetic-run> | --nal-mutation | --reopen-nal-mutation <synthetic-run> | --duplicate-review | --reopen-duplicate-review <synthetic-run> | --duplicate-undo | --reopen-duplicate-undo <synthetic-run> | --duplicate-merge | --reopen-duplicate-merge <synthetic-run> | --lifecycle | --reopen-lifecycle <synthetic-run> | --trash-cleanup | --reopen-trash-cleanup <synthetic-run> | --metadata | --reopen-metadata <synthetic-run> | --import-jobs | --reopen-import-jobs <synthetic-run> | --doi | --reopen-doi <synthetic-run> | --reference-records | --reopen-reference-records <synthetic-run> | --attachments | --reopen-attachments <synthetic-run>]".into(),
        );
    }
    let duplicate_mode = duplicate_review || reopen_duplicate_review;

    metadata_mode.check_features()?;
    import_jobs_mode.check_features()?;
    doi_mode.check_features()?;
    reference_mode.check_features()?;
    attachment_mode.check_features()?;
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()?;
    let parent = root.join(".tmp/runtime-plugins");
    std::fs::create_dir_all(&parent)?;
    if !parent.canonicalize()?.starts_with(root.join(".tmp")) {
        return Err("catalog acceptance requires the repository .tmp directory".into());
    }
    let query_mode = nal_queries || reopen_nal_queries;
    let bootstrap_mode = nal_bootstrap || reopen_nal_bootstrap;
    let annotation_mode = nal_annotation || reopen_nal_annotation;
    let mutation_mode = nal_mutation || reopen_nal_mutation;
    let directory = if reopen_any {
        let directory = std::path::PathBuf::from(&arguments[2]).canonicalize()?;
        let prefix = import_jobs_mode
            .prefix()
            .or_else(|| metadata_mode.prefix())
            .or_else(|| doi_mode.prefix())
            .or_else(|| reference_mode.prefix())
            .or_else(|| attachment_mode.prefix())
            .unwrap_or(if cleanup_mode {
                "l02-real-trash-cleanup-"
            } else if lifecycle_mode {
                "l02-real-lifecycle-"
            } else if merge_mode {
                "l02-real-duplicate-merge-"
            } else if undo_mode {
                "l02-real-duplicate-undo-"
            } else if duplicate_mode {
                "l02-real-duplicate-review-"
            } else if mutation_mode {
                "l06-real-mutation-"
            } else if annotation_mode {
                "l06-real-annotation-"
            } else if bootstrap_mode {
                "l06-real-bootstrap-"
            } else if query_mode {
                "l06-real-query-"
            } else {
                "l02-real-core-"
            });
        if directory.parent() != Some(parent.as_path())
            || !directory
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with(prefix))
        {
            return Err("reopen only accepts a synthetic catalog run in repository .tmp".into());
        }
        let (name, kind) = import_jobs_mode
            .marker()
            .or_else(|| metadata_mode.marker())
            .or_else(|| doi_mode.marker())
            .or_else(|| reference_mode.marker())
            .or_else(|| attachment_mode.marker())
            .unwrap_or(if cleanup_mode {
                ("synthetic-trash-cleanup.json", "trash-cleanup-check")
            } else if lifecycle_mode {
                ("synthetic-lifecycle.json", "lifecycle-check")
            } else if merge_mode {
                ("synthetic-duplicate-merge.json", "duplicate-merge-check")
            } else if undo_mode {
                ("synthetic-duplicate-undo.json", "duplicate-undo-check")
            } else if duplicate_mode {
                ("synthetic-duplicate-review.json", "duplicate-review-check")
            } else if mutation_mode {
                ("synthetic-nal-mutation.json", "nal-mutation-check")
            } else if annotation_mode {
                ("synthetic-nal-annotation.json", "nal-annotation-check")
            } else if bootstrap_mode {
                ("synthetic-nal-bootstrap.json", "nal-bootstrap-check")
            } else if query_mode {
                ("synthetic-nal-queries.json", "nal-query-collections-check")
            } else {
                ("synthetic-save.json", "catalog-save-check")
            });
        let marker: serde_json::Value =
            serde_json::from_slice(&std::fs::read(directory.join(name))?)?;
        if marker != json!({"kind":kind,"version":1}) {
            return Err("synthetic save marker is missing".into());
        }
        directory
    } else {
        tempfile::Builder::new()
            .prefix(
                import_jobs_mode
                    .prefix()
                    .or_else(|| metadata_mode.prefix())
                    .or_else(|| doi_mode.prefix())
                    .or_else(|| reference_mode.prefix())
                    .or_else(|| attachment_mode.prefix())
                    .unwrap_or(if cleanup_mode {
                        "l02-real-trash-cleanup-"
                    } else if lifecycle_mode {
                        "l02-real-lifecycle-"
                    } else if merge_mode {
                        "l02-real-duplicate-merge-"
                    } else if undo_mode {
                        "l02-real-duplicate-undo-"
                    } else if duplicate_mode {
                        "l02-real-duplicate-review-"
                    } else if mutation_mode {
                        "l06-real-mutation-"
                    } else if annotation_mode {
                        "l06-real-annotation-"
                    } else if bootstrap_mode {
                        "l06-real-bootstrap-"
                    } else if query_mode {
                        "l06-real-query-"
                    } else {
                        "l02-real-core-"
                    }),
            )
            .tempdir_in(&parent)?
            .keep()
    };
    let import_recovery_start = import_jobs_mode.before_open(&directory)?;
    let store = AppStore::open_with_paths(
        AppStorePaths {
            data_directory: directory.join("store"),
            agent_configuration_directory: directory.join("configuration"),
        },
        AppStorePorts::with_events(RuntimeEvents::new(|_, _| Ok(())), core_sources::sources()),
    )?;
    if !reopen_any
        && !nal_queries
        && !nal_bootstrap
        && !nal_annotation
        && !nal_mutation
        && !duplicate_review
        && !duplicate_undo
        && !duplicate_merge
        && !lifecycle
        && !trash_cleanup
        && !metadata_mode.fresh()
        && !import_jobs_mode.fresh()
        && !doi_mode.fresh()
        && !reference_mode.fresh()
        && !attachment_mode.fresh()
    {
        fixture::seed(&store)?;
    }
    let catalog = Arc::new(RuntimePluginCatalog::new(Some(arguments[0].clone().into())));
    let listing = catalog.discover()?;
    if listing.packages.len() != 1 {
        return Err("catalog acceptance requires exactly one independent package".into());
    }
    let key = listing.packages[0].package_key.clone();
    let cleanup_artifacts =
        if cleanup_mode || metadata_mode.selected() || import_jobs_mode.selected() {
            Some(trash_cleanup_check::artifacts(
                &catalog,
                &listing.packages[0],
            )?)
        } else {
            None
        };
    let service = Arc::new(LiteratureTraceService::new(catalog));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(async {
        let result = if attachment_mode.selected() {
            attachment_mode.run(&store, &service, &key, &directory).await
        } else if reference_mode.selected() {
            reference_mode.run(&store, &service, &key, &directory).await
        } else if doi_mode.selected() {
            doi_mode.run(&store, &service, &key, &directory).await
        } else if import_jobs_mode.selected() {
            import_jobs_mode.run(&store, &service, &key, &directory, import_recovery_start.as_deref()).await
        } else if metadata_mode.selected() {
            metadata_mode.run(&store, &service, &key, &directory).await
        } else if reopen_trash_cleanup {
            trash_cleanup_check::reopen(&store, &service, &key, &directory).await
        } else if trash_cleanup {
            trash_cleanup_check::run(&store, &service, &key).await
        } else if reopen_lifecycle {
            lifecycle_check::reopen(&store, &service, &key, &directory).await
        } else if lifecycle {
            lifecycle_check::run(&store, &service, &key).await
        } else if reopen_duplicate_merge {
            duplicate_merge_check::reopen(&store, &service, &key, &directory).await
        } else if duplicate_merge {
            duplicate_merge_check::run(&store, &service, &key).await
        } else if reopen_duplicate_undo {
            duplicate_undo_check::reopen(&store, &service, &key, &directory).await
        } else if duplicate_undo {
            duplicate_undo_check::run(&store, &service, &key).await
        } else if reopen_duplicate_review {
            duplicate_review_check::reopen(&store, &service, &key, &directory).await
        } else if duplicate_review {
            duplicate_review_check::run(&store, &service, &key).await
        } else if reopen_nal_mutation {
            nal_mutation_check::reopen(&store, &service, &key, &directory).await
        } else if nal_mutation {
            nal_mutation_check::run(&store, &service, &key).await
        } else if reopen_nal_annotation {
            nal_annotation_check::reopen(&store, &service, &key, &directory).await
        } else if nal_annotation {
            nal_annotation_check::run(&store, &service, &key).await
        } else if reopen_nal_bootstrap {
            nal_bootstrap_check::reopen(&store, &service, &key, &directory).await
        } else if nal_bootstrap {
            nal_bootstrap_check::run(&store, &service, &key).await
        } else if reopen_nal_queries {
            nal_query_check::reopen(&store, &service, &key, &directory).await
        } else if nal_queries {
            nal_query_check::run(&store, &service, &key).await
        } else if reopen {
            save_check::reopen(&store, &service, &key, &directory).await
        } else if save_mode {
            save_check::run(&store, &service, &key).await
        } else {
            check(&store, &service, &key).await
        };
        let cleanup = service.shutdown().await;
        match result {
            Ok(mut receipt) => {
                cleanup?;
                receipt["status"] = if cleanup_mode { json!("passed-trash-cleanup-native-core-only") }
                    else if lifecycle_mode { json!("passed-lifecycle-native-core-only") }
                    else if merge_mode { json!("passed-duplicate-merge-native-core-only") }
                    else if undo_mode { json!("passed-duplicate-undo-native-core-only") }
                    else if duplicate_mode { json!("passed-duplicate-review-native-core-only") }
                    else if mutation_mode { json!("passed-nal-mutation-native-core-only") }
                    else if annotation_mode { json!("passed-nal-annotation-native-core-only") }
                    else if bootstrap_mode { json!("passed-nal-bootstrap-native-core-only") }
                    else if query_mode { json!("passed-nal-queries-native-core-only") }
                    else { json!("passed-catalog-native-core-only") };
                receipt["packageKey"] = json!(key);
                receipt["shutdown"] = json!("passed");
                if let Some(artifacts) = &cleanup_artifacts {
                    receipt["artifacts"] = artifacts.clone();
                }
                receipt["scope"] = if cleanup_mode {
                    json!("L02 expiry cleanup and trash list backend only; no frontend or complete C03 acceptance")
                } else if lifecycle_mode {
                    json!("L02 six lifecycle backend entries only; no frontend or full L02/C03 acceptance")
                } else if merge_mode {
                    json!("L02 full merge and original undo backend only; no frontend or complete C03 acceptance")
                } else if undo_mode {
                    json!("L02 duplicate undo backend only; no merge, frontend or complete C03 acceptance")
                } else if duplicate_mode {
                    json!("L02 duplicate candidates/decision/history backend only; no merge/undo, frontend or complete C03 acceptance")
                } else if mutation_mode {
                    json!("NAL mutate backend only; no frontend or complete L06/C03 acceptance")
                } else if annotation_mode {
                    json!("NAL ensure backend only; no frontend/mutate or complete L06/C03 acceptance")
                } else if bootstrap_mode {
                    json!("NAL bootstrap backend only; no frontend/mutate/annotation or complete L06/C03 acceptance")
                } else if query_mode {
                    json!("NAL query collections backend only; no complete NAL bootstrap, UI or C03 acceptance")
                } else if save_mode || reopen {
                    json!("Manual catalog save only; no frontend, other write chains or complete C03 acceptance")
                } else { json!("L02 list/get only; no frontend, write-chain or complete C03 acceptance") };
                metadata_mode.annotate(&mut receipt);
                import_jobs_mode.annotate(&mut receipt);
                doi_mode.annotate(&mut receipt);
                reference_mode.annotate(&mut receipt);
                attachment_mode.annotate(&mut receipt);
                Ok(receipt)
            }
            Err(error) => Err(error),
        }
    });
    store.shared_agent_connection().shutdown()?;
    let receipt = result?;
    metadata_mode.write_marker(&directory)?;
    import_jobs_mode.write_marker(&directory)?;
    doi_mode.write_marker(&directory)?;
    reference_mode.write_marker(&directory)?;
    attachment_mode.write_marker(&directory)?;
    if trash_cleanup {
        use std::io::Write;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join("synthetic-trash-cleanup.json"))?
            .write_all(&serde_json::to_vec(
                &json!({"kind":"trash-cleanup-check","version":1}),
            )?)?;
    }
    if lifecycle {
        use std::io::Write;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join("synthetic-lifecycle.json"))?
            .write_all(&serde_json::to_vec(
                &json!({"kind":"lifecycle-check","version":1}),
            )?)?;
    }
    if duplicate_merge {
        use std::io::Write;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join("synthetic-duplicate-merge.json"))?
            .write_all(&serde_json::to_vec(
                &json!({"kind":"duplicate-merge-check","version":1}),
            )?)?;
    }
    if duplicate_undo {
        use std::io::Write;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join("synthetic-duplicate-undo.json"))?
            .write_all(&serde_json::to_vec(
                &json!({"kind":"duplicate-undo-check","version":1}),
            )?)?;
    }
    if duplicate_review {
        use std::io::Write;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join("synthetic-duplicate-review.json"))?
            .write_all(&serde_json::to_vec(
                &json!({"kind":"duplicate-review-check","version":1}),
            )?)?;
    }
    if save_mode {
        std::fs::write(
            directory.join("synthetic-save.json"),
            serde_json::to_vec(&json!({"kind":"catalog-save-check","version":1}))?,
        )?;
    }
    if nal_queries {
        std::fs::write(
            directory.join("synthetic-nal-queries.json"),
            serde_json::to_vec(&json!({"kind":"nal-query-collections-check","version":1}))?,
        )?;
    }
    if nal_bootstrap {
        use std::io::Write;
        let mut marker = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join("synthetic-nal-bootstrap.json"))?;
        marker.write_all(&serde_json::to_vec(
            &json!({"kind":"nal-bootstrap-check","version":1}),
        )?)?;
    }
    if nal_annotation {
        use std::io::Write;
        let mut marker = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join("synthetic-nal-annotation.json"))?;
        marker.write_all(&serde_json::to_vec(
            &json!({"kind":"nal-annotation-check","version":1}),
        )?)?;
    }
    if nal_mutation {
        use std::io::Write;
        let mut marker = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join("synthetic-nal-mutation.json"))?;
        marker.write_all(&serde_json::to_vec(
            &json!({"kind":"nal-mutation-check","version":1}),
        )?)?;
    }
    let path = directory.join(if reopen_any {
        format!("reopen-{}.json", uuid::Uuid::new_v4())
    } else {
        "receipt.json".into()
    });
    if bootstrap_mode
        || annotation_mode
        || mutation_mode
        || duplicate_mode
        || undo_mode
        || merge_mode
        || lifecycle_mode
        || cleanup_mode
        || metadata_mode.selected()
        || import_jobs_mode.selected()
        || reference_mode.selected()
        || attachment_mode.selected()
    {
        use std::io::Write;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?
            .write_all(&serde_json::to_vec_pretty(&receipt)?)?;
    } else {
        std::fs::write(&path, serde_json::to_vec_pretty(&receipt)?)?;
    }
    println!("{}", json!({"receipt":path,"status":receipt["status"]}));
    Ok(())
}
fn ids(items: &[CatalogItem]) -> Vec<&str> {
    items.iter().map(|item| item.id.as_str()).collect()
}
