use super::*;
use crate::agent::writing_instructions::{self, InstructionEnvironment};
use crate::agent::{instructions::InstructionRootIdentity, repository::instruction_trust};
use std::{fs, os::unix::fs::MetadataExt};

#[path = "instructions/overrides.rs"]
mod overrides;

#[tokio::test]
async fn trust_controls_new_materialization_and_restore_keeps_original_snapshot() {
    let f = fixture(true);
    let service = WritingSubmissionService::from_app_store(&f.store);
    let root = f
        .store
        .manuscript_directory()
        .join(f.manuscript.to_string())
        .join("source")
        .canonicalize()
        .unwrap();
    // Untrusted content is not parsed or promoted to instructions.
    fs::write(root.join("AGENTS.md"), [0xff]).unwrap();
    assert!(matches!(
        writing_instructions::materialize_in(
            &f.store.connection().unwrap(),
            InstructionEnvironment {
                manuscripts: f.store.manuscript_directory(),
                user_configuration: f.store.agent_configuration_directory(),
                source_directory: None,
            },
            f.manuscript,
            &[],
            "review",
            [None, None],
        ),
        Ok(context) if context.instructions.is_empty()
            && context.instruction_configs.is_empty() && context.skills.is_empty()
    ));
    assert!(
        service
            .prepare(&f.models, request(&f, Uuid::new_v4()))
            .await
            .unwrap()
            .record
            .input
            .instructions
            .is_empty()
    );
    fs::write(root.join("AGENTS.md"), "Use British spelling.").unwrap();
    let metadata = fs::metadata(&root).unwrap();
    let identity = InstructionRootIdentity {
        canonical_path: &root,
        device: metadata.dev(),
        inode: metadata.ino(),
    };
    instruction_trust::update(
        &mut f.store.connection().unwrap(),
        f.manuscript,
        0,
        &identity,
        true,
    )
    .unwrap();
    assert!(matches!(
        writing_instructions::materialize_in(
            &f.store.connection().unwrap(),
            InstructionEnvironment {
                manuscripts: f.store.manuscript_directory(),
                user_configuration: f.store.agent_configuration_directory(),
                source_directory: None,
            },
            f.manuscript,
            &[],
            "review",
            [None, None],
        ),
        Err(crate::agent::instructions::InstructionReadError::Unavailable)
    ));
    let user_root = f.store.agent_configuration_directory();
    fs::create_dir_all(user_root.join("agent")).unwrap();
    fs::write(
        user_root.join("agent/config.toml"),
        "schema_version = 1\n[instructions]\nmax_bytes = 5\nfallback_filenames = ['USER.md']\n",
    )
    .unwrap();
    fs::create_dir_all(root.join(".agents/skills/review")).unwrap();
    let skill_path = root.join(".agents/skills/review/SKILL.md");
    fs::write(
        &skill_path,
        "---\ndescription: Review prose\n---\nOriginal skill body",
    )
    .unwrap();
    let user_preview = service
        .prepare(&f.models, request(&f, Uuid::new_v4()))
        .await
        .unwrap();
    assert_eq!(user_preview.record.input.instructions[0].text, "Use B");
    assert_eq!(user_preview.record.input.skills.len(), 1);
    assert_eq!(
        user_preview.record.input.skills[0]
            .descriptor
            .identity
            .source_id,
        ".agents/skills/review/SKILL.md"
    );
    let skill_revision = user_preview.record.input.skills[0]
        .descriptor
        .identity
        .revision
        .clone();
    fs::write(
        &skill_path,
        "---\ndescription: Review prose\n---\nChanged skill body",
    )
    .unwrap();

    assert_eq!(
        user_preview.record.input.instruction_configs[0].layers[1]
            .origin
            .tier,
        research_agent_core::config::ConfigTier::User
    );
    fs::create_dir(root.join(".research-assistant")).unwrap();
    let config_path = root.join(".research-assistant/agent.toml");
    fs::write(
        &config_path,
        "schema_version = 1\n[instructions]\nmax_bytes = 4096\nfallback_filenames = ['RULES.md']\n",
    )
    .unwrap();
    fs::rename(root.join("AGENTS.md"), root.join("RULES.md")).unwrap();
    let id = Uuid::new_v4();
    let prepared = service.prepare(&f.models, request(&f, id)).await.unwrap();
    assert_ne!(
        prepared.record.input.skills[0].descriptor.identity.revision,
        skill_revision
    );

    assert_eq!(
        prepared.record.input.instructions[0].text,
        "Use British spelling."
    );
    assert_eq!(
        prepared.record.input.instructions[0].environment_id,
        format!("manuscript:{}", f.manuscript)
    );
    assert_eq!(prepared.record.input.instruction_configs.len(), 1);
    assert_eq!(
        prepared.record.input.instruction_configs[0].layers[2]
            .instructions
            .max_bytes,
        Some(4096)
    );
    let section = root.join("sections");
    fs::create_dir_all(section.join(".research-assistant")).unwrap();
    let section_text = "Frozen section";
    let section_hash = format!("{:x}", sha2::Sha256::digest(section_text.as_bytes()));
    fs::write(section.join("section.tex"), section_text).unwrap();
    f.store
        .connection()
        .unwrap()
        .execute(
            "insert into manuscript_files(manuscript_id,relative_path,file_kind,byte_size,sha256,modified_at) values (?1,'sections/section.tex','tex',?2,?3,'now')",
            rusqlite::params![f.manuscript.to_string(), section_text.len(), section_hash],
        )
        .unwrap();
    fs::write(
        section.join(".research-assistant/agent.toml"),
        "schema_version = 1\n[instructions]\nmax_bytes = 7\nfallback_filenames = ['SECTION.md']\n",
    )
    .unwrap();
    fs::write(section.join("SECTION.md"), "Section rules").unwrap();
    let mut nested = request(&f, Uuid::new_v4());
    nested.context.files.push(crate::agent::WritingContextFile {
        relative_path: "sections/section.tex".into(),
        expected_disk_sha256: section_hash,
        buffer: None,
        selection: None,
    });
    let nested = service.prepare(&f.models, nested).await.unwrap();
    let directory_layer = nested
        .record
        .input
        .instruction_configs
        .iter()
        .flat_map(|snapshot| &snapshot.layers)
        .find(|layer| layer.origin.tier == research_agent_core::config::ConfigTier::Directory)
        .expect("nested directory config is frozen");
    assert_eq!(directory_layer.instructions.max_bytes, Some(7));
    assert_eq!(
        directory_layer.origin.source_id,
        "sections/.research-assistant/agent.toml"
    );
    fs::write(
        &config_path,
        "schema_version = 1\n[instructions]\nmax_bytes = 0\nfallback_filenames = ['RULES.md']\n",
    )
    .unwrap();
    let changed = service
        .prepare(&f.models, request(&f, Uuid::new_v4()))
        .await
        .unwrap();
    assert!(changed.record.input.instructions[0].text.is_empty());
    assert_ne!(
        changed.record.input.instruction_configs,
        prepared.record.input.instruction_configs
    );
    fs::write(root.join("RULES.md"), "Changed convention").unwrap();
    // A workspace override cannot conceal malformed user settings. Restoring
    // the earlier receipt below must not parse this newly invalid file.
    fs::write(
        user_root.join("agent/config.toml"),
        "schema_version = 'invalid'",
    )
    .unwrap();
    assert!(
        service
            .prepare(&f.models, request(&f, Uuid::new_v4()))
            .await
            .is_err()
    );
    instruction_trust::update(
        &mut f.store.connection().unwrap(),
        f.manuscript,
        1,
        &identity,
        false,
    )
    .unwrap();
    let restored = service
        .restore(
            &f.models,
            WritingSubmissionReceipt {
                conversation_id: f.conversation,
                request_id: id,
                manuscript_id: f.manuscript,
            },
        )
        .await
        .unwrap();
    assert_eq!(restored.record, prepared.record);
    // Revoking manuscript trust does not make malformed application-wide
    // configuration valid. Repair that independent error before checking that
    // a new submission excludes the now-untrusted manuscript instructions.
    let rejected_id = Uuid::new_v4();
    assert_prepare_rejection(
        &f,
        rejected_id,
        service.prepare(&f.models, request(&f, rejected_id)).await,
        "DATA_SQL_INVALID",
        "writing_instructions_scope_invalid",
    );
    fs::remove_file(user_root.join("agent/config.toml")).unwrap();
    assert!(
        service
            .prepare(&f.models, request(&f, Uuid::new_v4()))
            .await
            .unwrap()
            .record
            .input
            .instructions
            .is_empty()
    );
}

#[test]
fn sibling_configuration_does_not_replace_ancestor_or_neighbor_rules() {
    let f = fixture(true);
    let root = f
        .store
        .manuscript_directory()
        .join(f.manuscript.to_string())
        .join("source")
        .canonicalize()
        .unwrap();
    let metadata = fs::metadata(&root).unwrap();
    instruction_trust::update(
        &mut f.store.connection().unwrap(),
        f.manuscript,
        0,
        &InstructionRootIdentity {
            canonical_path: &root,
            device: metadata.dev(),
            inode: metadata.ino(),
        },
        true,
    )
    .unwrap();
    for directory in ["", "a", "b"] {
        fs::create_dir_all(root.join(directory).join(".research-assistant")).unwrap();
    }
    fs::write(
        root.join(".research-assistant/agent.toml"),
        "schema_version=1\n[instructions]\nmax_bytes=4096\nfallback_filenames=['ROOT.md']",
    )
    .unwrap();
    fs::write(root.join("ROOT.md"), "ROOT").unwrap();
    fs::write(
        root.join("a/.research-assistant/agent.toml"),
        "schema_version=1\n[instructions]\nmax_bytes=7\nfallback_filenames=['A.md']",
    )
    .unwrap();
    fs::write(root.join("a/A.md"), "AAAAAA").unwrap();
    fs::write(
        root.join("b/.research-assistant/agent.toml"),
        "schema_version=1\n[instructions]\nfallback_filenames=['B.md']",
    )
    .unwrap();
    fs::write(root.join("b/B.md"), "BBBBBB").unwrap();
    // These decoys must never be selected through a sibling/child override.
    fs::write(root.join("B.md"), "wrong root").unwrap();
    fs::write(root.join("a/B.md"), "wrong sibling").unwrap();
    let mut sources: Vec<_> = ["a/main.tex", "b/main.tex"]
        .into_iter()
        .map(|path| research_agent_core::submission::ContextText {
            source_id: path.into(),
            resource_id: path.into(),
            revision: "1".into(),
            kind: research_agent_core::submission::ContextSourceKind::Manuscript,
            anchor: None,
            text: "source".into(),
        })
        .collect();
    let materialize = |sources: &[_]| {
        writing_instructions::materialize_in(
            &f.store.connection().unwrap(),
            InstructionEnvironment {
                manuscripts: f.store.manuscript_directory(),
                user_configuration: f.store.agent_configuration_directory(),
                source_directory: f.store.writing_source_directory(),
            },
            f.manuscript,
            sources,
            "review",
            [None, None],
        )
        .unwrap()
    };
    let frozen = materialize(&sources);
    assert_eq!(
        frozen
            .instructions
            .iter()
            .map(|entry| (entry.source_id.as_str(), entry.text.as_str()))
            .collect::<Vec<_>>(),
        [("ROOT.md", "ROOT"), ("a/A.md", "AAA"), ("b/B.md", "BBBBBB")]
    );
    let a = &frozen.instruction_configs[1];
    let b = &frozen.instruction_configs[2];
    assert!(
        !a.layers
            .iter()
            .any(|layer| layer.origin.source_id.starts_with("b/"))
    );
    assert!(
        !b.layers
            .iter()
            .any(|layer| layer.origin.source_id.starts_with("a/"))
    );
    sources.reverse();
    let reversed = materialize(&sources);
    assert_eq!(frozen.instructions, reversed.instructions);
    assert_eq!(frozen.instruction_configs, reversed.instruction_configs);
    fs::write(
        root.join(".research-assistant/agent.toml"),
        "schema_version=1\n[instructions]\nmax_bytes=7\nfallback_filenames=['ROOT.md']",
    )
    .unwrap();
    let bounded = materialize(&sources);
    assert_eq!(
        bounded
            .instructions
            .iter()
            .map(|entry| entry.text.len())
            .sum::<usize>(),
        7
    );
    assert_eq!(
        bounded.instructions[2].disposition,
        research_agent_core::instructions::InstructionDisposition::BudgetExcluded
    );
}

fn assert_prepare_rejection(
    f: &Fixture,
    operation: Uuid,
    result: Result<PreparedWritingSubmission, WritingSubmissionError>,
    code: &str,
    cause: &str,
) {
    let Err(WritingSubmissionError::Storage(error)) = result else {
        panic!("typed preparation rejection required")
    };
    let failure = error.as_inner();
    assert_eq!(failure.code(), code);
    let detail = failure.diagnostic().unwrap();
    assert_eq!(detail.operation_id, operation);
    assert_eq!(detail.phase, "writing-prepare-materialize");
    assert_eq!(detail.safe_cause, cause);
    let db = f.store.connection().unwrap();
    for sql in [
        "select count(*) from agent_submissions where request_id=?1",
        "select count(*) from data_sql_operation_receipts where operation_id=?1",
    ] {
        assert_eq!(
            db.query_row(sql, [operation.to_string()], |row| row.get::<_, u64>(0))
                .unwrap(),
            0
        );
    }
}
