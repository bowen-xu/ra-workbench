//! Independently check the production review before retaining an acceptance fixture.
use super::{FixtureCase, INTRODUCTION, MAIN};
use research_agent_core::workspace::{FileChange, FileChangeKind, FileVersion};
use sha2::{Digest, Sha256};

pub(super) fn assert_inventory(case: FixtureCase, changes: &[FileChange]) {
    let revised = MAIN
        .replace("Original abstract.", "Agent revised abstract.")
        .replace("Original conclusion.", "Agent revised conclusion.");
    let large = vec![0; 5 * 1024 * 1024];
    let main_after = match case {
        FixtureCase::EditorDelete => None,
        FixtureCase::EditorBinary => Some(version(&[0xff, 0xfe], false)),
        FixtureCase::EditorOversized => Some(version(&large, false)),
        FixtureCase::EditorMode => Some(version(revised.as_bytes(), true)),
        _ => Some(version(revised.as_bytes(), false)),
    };
    let mut expected = vec![(
        "main.tex",
        Some(version(MAIN.as_bytes(), false)),
        main_after,
    )];
    if case == FixtureCase::Standard {
        expected.push((
            "sections/introduction.tex",
            Some(version(INTRODUCTION.as_bytes(), false)),
            Some(version(
                INTRODUCTION
                    .replace("Original introduction.", "Agent revised introduction.")
                    .as_bytes(),
                false,
            )),
        ));
    } else {
        expected.push((
            "aaa.tex",
            None,
            Some(version(
                b"\\section{Synthetic addition}\nAdded by the approved native patch tool.\n",
                false,
            )),
        ));
    }
    if case == FixtureCase::MixedFiles {
        expected.extend([
            ("large.txt", None, Some(version(&large, false))),
            ("new.bin", None, Some(version(&[0xfe, 0xff], false))),
            (
                "sections/introduction.tex",
                Some(version(INTRODUCTION.as_bytes(), false)),
                None,
            ),
            (
                "synthetic.bin",
                Some(version(&[0xff, 0x00, 0xfe], false)),
                Some(version(&[0xff, 0xfe], false)),
            ),
        ]);
    }
    expected.sort_by_key(|(path, _, _)| *path);
    assert_eq!(changes.len(), expected.len());
    for (actual, (path, before, after)) in changes.iter().zip(expected) {
        assert_eq!(actual.path, path);
        assert_eq!(actual.before, before, "{path}: frozen original bytes/mode");
        assert_eq!(actual.after, after, "{path}: actual approved tool result");
        assert_eq!(
            actual.kind,
            match (&before, &after) {
                (None, _) => FileChangeKind::Added,
                (_, None) => FileChangeKind::Deleted,
                _ => FileChangeKind::Modified,
            }
        );
        assert_eq!(actual.source.is_some(), before.is_some(), "{path}: source");
    }
}

fn version(bytes: &[u8], executable: bool) -> FileVersion {
    FileVersion {
        sha256: format!("{:x}", Sha256::digest(bytes)),
        byte_size: bytes.len() as u64,
        executable,
    }
}
