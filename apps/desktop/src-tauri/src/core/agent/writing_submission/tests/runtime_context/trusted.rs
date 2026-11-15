use super::*;
use crate::agent::{instructions::InstructionRootIdentity, repository::instruction_trust};
use std::{fs, os::unix::fs::MetadataExt};

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_uses_core_trusted_instructions_and_skills_without_static_writing_sources() {
    let mut f = fixture(true);
    let binding = binding(packages());
    f.store = f
        .store
        .without_static_writing_context_sources()
        .with_native_writing_context(binding.clone());
    assert!(f.store.writing_resource_source().is_none());
    let root = f.store.writing_source_directory().unwrap()(
        f.store.manuscript_directory(),
        &f.manuscript.to_string(),
    )
    .unwrap()
    .canonicalize()
    .unwrap();
    let service = WritingSubmissionService::from_app_store(&f.store);

    // An untrusted invalid instruction file remains uninspected.
    fs::write(root.join("AGENTS.md"), [0xff]).unwrap();
    let untrusted = service
        .prepare(&f.models, request(&f, Uuid::new_v4()))
        .await
        .unwrap();
    assert!(untrusted.record.input.instructions.is_empty());
    assert!(untrusted.record.input.instruction_configs.is_empty());
    assert!(untrusted.record.input.skills.is_empty());

    fs::write(root.join("AGENTS.md"), "Use British spelling.").unwrap();
    fs::create_dir_all(root.join(".agents/skills/review")).unwrap();
    let skill = root.join(".agents/skills/review/SKILL.md");
    fs::write(
        &skill,
        "---\ndescription: Review prose\n---\nOriginal skill body",
    )
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
    let mut input = request(&f, Uuid::new_v4());
    input.text = "$review Explain".into();
    let prepared = service.prepare(&f.models, input.clone()).await.unwrap();
    assert_eq!(
        prepared.record.input.instructions[0].text,
        "Use British spelling."
    );
    assert_eq!(
        prepared.record.input.instructions[0].environment_id,
        format!("manuscript:{}", f.manuscript)
    );
    assert_eq!(prepared.record.input.instruction_configs.len(), 1);
    assert_eq!(prepared.record.input.skills.len(), 1);
    assert_eq!(
        prepared.record.input.skills[0]
            .descriptor
            .identity
            .source_id,
        ".agents/skills/review/SKILL.md"
    );
    let revision = prepared.record.input.skills[0]
        .descriptor
        .identity
        .revision
        .clone();

    fs::write(
        &skill,
        "---\ndescription: Review prose\n---\nChanged skill body",
    )
    .unwrap();
    fs::write(root.join("AGENTS.md"), "Changed instructions.").unwrap();
    let fresh = service
        .prepare(&f.models, request(&f, Uuid::new_v4()))
        .await
        .unwrap();
    assert_ne!(
        fresh.record.input.skills[0].descriptor.identity.revision,
        revision
    );
    assert_eq!(
        fresh.record.input.instructions[0].text,
        "Changed instructions."
    );

    binding.service.shutdown().await.unwrap();
    fs::remove_file(&skill).unwrap();
    fs::remove_file(root.join("AGENTS.md")).unwrap();
    fs::remove_file(root.join("main.tex")).unwrap();
    let replay = service.prepare(&f.models, input).await.unwrap();
    assert_eq!(replay.record, prepared.record);
    assert!(service.store.writing_resource_source().is_none());
}
