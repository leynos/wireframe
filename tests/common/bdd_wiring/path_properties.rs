//! Property tests for the path invariants the wiring check relies on.
//!
//! Generated inputs sample bounded component sequences and module layouts that
//! the fixed mutation table cannot enumerate. Each case is drawn from a
//! generated space rather than checked exhaustively, so a passing run raises
//! confidence without proving every possible shape.

use camino::{Utf8Component, Utf8PathBuf};
use proptest::prelude::*;

use super::{
    snapshot::{RepoSnapshot, normalize_path},
    syntax::resolve_target,
};

/// Generate a lowercase path segment that is a valid file name.
fn name_strategy() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::char::range('a', 'z'), 1..=6)
        .prop_map(|letters| letters.into_iter().collect())
}

/// Generate one path segment, including the relative markers `.` and `..`.
fn segment_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        6 => name_strategy(),
        2 => Just(String::from(".")),
        2 => Just(String::from("..")),
    ]
}

/// Generate a path that may be absolute and may contain `.` and `..`.
fn path_strategy() -> impl Strategy<Value = Utf8PathBuf> {
    (
        any::<bool>(),
        prop::collection::vec(segment_strategy(), 0..6),
    )
        .prop_map(|(is_absolute, segments)| {
            let prefix = if is_absolute { "/" } else { "" };
            Utf8PathBuf::from(format!("{prefix}{}", segments.join("/")))
        })
}

/// A declaring module file paired with the directories Rust resolves from it.
#[derive(Debug, Clone)]
struct DeclaringFile {
    /// Repository-relative path of the file containing the declaration.
    file: Utf8PathBuf,
    /// Directory containing the declaring file, which anchors `#[path]`.
    declaring_directory: Utf8PathBuf,
    /// Directory in which a declaration without `#[path]` is resolved.
    module_directory: Utf8PathBuf,
}

/// Generate both documented declaring-file shapes: `mod.rs` and `<stem>.rs`.
fn declaring_file_strategy() -> impl Strategy<Value = DeclaringFile> {
    (name_strategy(), name_strategy(), any::<bool>()).prop_map(|(leaf, stem, is_mod_rs)| {
        let directory = Utf8PathBuf::from("tests").join(leaf);
        // A file named `mod.rs` searches its own directory whatever produced
        // it, so a `mod` stem takes the root-file shape to keep the generated
        // declaring file and module directory consistent.
        let uses_mod_rs = is_mod_rs || stem == "mod";
        if uses_mod_rs {
            DeclaringFile {
                file: directory.join("mod.rs"),
                declaring_directory: directory.clone(),
                module_directory: directory,
            }
        } else {
            DeclaringFile {
                file: directory.join(format!("{stem}.rs")),
                module_directory: directory.join(&stem),
                declaring_directory: directory,
            }
        }
    })
}

/// An empty snapshot: attribute targets never depend on existing files.
fn empty_snapshot() -> RepoSnapshot {
    RepoSnapshot::from_files(Vec::<(Utf8PathBuf, String)>::new())
}

proptest! {
    /// Normalized paths contain only plain segments.
    #[test]
    fn normalized_paths_have_only_normal_components(path in path_strategy()) {
        let normalized = normalize_path(&path);
        prop_assert!(
            normalized
                .components()
                .all(|component| matches!(component, Utf8Component::Normal(_))),
            "`{path}` normalized to `{normalized}` with a non-normal component"
        );
    }

    /// Normalization reaches a fixed point after one pass.
    #[test]
    fn normalization_is_idempotent(path in path_strategy()) {
        let once = normalize_path(&path);
        let twice = normalize_path(&once);
        prop_assert_eq!(twice, once);
    }

    /// A `#[path]` target is the normalized declaring directory plus attribute.
    #[test]
    fn path_attribute_targets_are_normalized(
        declaring in declaring_file_strategy(),
        relative in path_strategy(),
    ) {
        let expected = normalize_path(&declaring.declaring_directory.join(&relative));
        let target = resolve_target(&empty_snapshot(), &declaring.file, "family", Some(relative));
        prop_assert_eq!(&target, &expected);
        prop_assert_eq!(normalize_path(&target), target);
    }

    /// Resolution prefers an existing flat file, then a nested `mod.rs`.
    #[test]
    fn module_targets_follow_the_flat_then_nested_fallback(
        declaring in declaring_file_strategy(),
        name in name_strategy(),
        has_flat in any::<bool>(),
        has_nested in any::<bool>(),
    ) {
        let flat = declaring.module_directory.join(format!("{name}.rs"));
        let nested = declaring.module_directory.join(&name).join("mod.rs");
        let mut files = Vec::new();
        if has_flat {
            files.push((flat.clone(), String::new()));
        }
        if has_nested {
            files.push((nested.clone(), String::new()));
        }
        let target = resolve_target(&RepoSnapshot::from_files(files), &declaring.file, &name, None);
        let expected = if has_flat { flat } else if has_nested { nested } else { flat };
        prop_assert_eq!(target, expected);
    }
}
