//! Checks the inversion register against the tests it names.
//!
//! The register in `docs/zero-copy-benchmark-baselines.md` records which test
//! each roadmap item flips. A register that names a renamed or deleted test
//! would keep passing while enforcing nothing, so these tests parse the
//! register and look for every named function in the file it names. The parser
//! is also exercised against synthetic registers, because the real document
//! cannot express a malformed entry.

/// Marker opening the machine-readable part of the inversion register.
const REGISTER_START: &str = "<!-- inversion-register:start -->";
/// Marker closing the machine-readable part of the inversion register.
const REGISTER_END: &str = "<!-- inversion-register:end -->";

/// One test the register expects to flip when a roadmap item lands.
#[derive(Debug, PartialEq, Eq)]
struct Inversion {
    /// Name of the test function.
    test: String,
    /// Repository-relative path of the file that defines it.
    file: String,
}

/// Joins each `- ` list item in `body` with its wrapped continuation lines.
///
/// The register is wrapped at 80 columns, so a test name and its file usually
/// sit on different lines. Joining them is what lets the span extraction below
/// see both; reading a line at a time finds one span and silently drops the
/// entry.
fn register_items(body: &str) -> Vec<String> {
    let mut items: Vec<String> = Vec::new();
    for line in body.lines() {
        if let Some(item) = line.strip_prefix("- ") {
            items.push(item.to_owned());
        } else if line.starts_with("  ")
            && let Some(current) = items.last_mut()
        {
            current.push(' ');
            current.push_str(line.trim());
        }
    }
    items
}

/// Extract the register's entries.
///
/// Each entry is a Markdown list item whose first two backticked spans are the
/// test name and its file, so the prose after them can change without breaking
/// the check. Wrapped entries are joined before the spans are read.
///
/// # Errors
///
/// Returns an error when a marker is missing, when the register lists no
/// entries, or when an entry does not carry both spans. A parser that dropped
/// such an entry would let the check pass while enforcing nothing.
fn register_entries(register: &str) -> Result<Vec<Inversion>, String> {
    let Some((_, after_start)) = register.split_once(REGISTER_START) else {
        return Err(format!(
            "the register is missing its `{REGISTER_START}` marker"
        ));
    };
    let Some((body, _)) = after_start.split_once(REGISTER_END) else {
        return Err(format!(
            "the register is missing its `{REGISTER_END}` marker"
        ));
    };

    let entries: Vec<Inversion> = register_items(body)
        .into_iter()
        .map(|item| {
            let spans: Vec<&str> = item.split('`').skip(1).step_by(2).collect();
            match (spans.first(), spans.get(1)) {
                (Some(test), Some(file)) => Ok(Inversion {
                    test: (*test).to_owned(),
                    file: (*file).to_owned(),
                }),
                _ => Err(format!(
                    "register entry `{item}` does not name a test and a file"
                )),
            }
        })
        .collect::<Result<_, _>>()?;

    if entries.is_empty() {
        return Err("the register lists no entries".to_owned());
    }
    Ok(entries)
}

/// Report every register entry whose test is missing from its named file.
///
/// # Errors
///
/// Returns the parse failure from [`register_entries`] rather than reporting an
/// empty set of missing tests, which would read as a clean register.
fn missing_register_entries(
    register: &str,
    sources: &[(&str, &str)],
) -> Result<Vec<String>, String> {
    let missing = register_entries(register)?
        .into_iter()
        .filter(|entry| {
            let defines_test = sources
                .iter()
                .find(|(path, _)| *path == entry.file)
                .is_some_and(|(_, source)| source.contains(&format!("fn {}(", entry.test)));
            !defines_test
        })
        .map(|entry| format!("{} in {}", entry.test, entry.file))
        .collect();
    Ok(missing)
}

/// The register's own file, read at compile time so deleting it is a build
/// error rather than a silently skipped check.
const BASELINES_DOC: &str = include_str!("../../../docs/zero-copy-benchmark-baselines.md");

/// The copied-byte test file, as the register names it.
const THIS_SOURCE: &str = include_str!("../copied_bytes.rs");

/// The allocation target's hand-off checks, as the register names them.
const HANDOFF_SOURCE: &str = include_str!("../pipeline_allocation_invariants/checks/handoff.rs");

#[test]
fn inversion_register_names_real_tests() {
    let sources = [
        ("wireframe_testing/tests/copied_bytes.rs", THIS_SOURCE),
        (
            "wireframe_testing/tests/pipeline_allocation_invariants/checks/handoff.rs",
            HANDOFF_SOURCE,
        ),
    ];
    let entries = register_entries(BASELINES_DOC).expect("the register must parse");
    assert_eq!(
        entries.len(),
        6,
        "the register must list all six migration-sensitive assertions; a parser that drops \
         entries would otherwise pass this check"
    );
    assert!(
        entries
            .iter()
            .any(|entry| entry.test == "outbound_path_copies_the_payload_twice_per_message"),
        "the register must name the outbound two-copy assertion"
    );
    assert!(
        entries
            .iter()
            .any(|entry| entry.test == "seams_charge_one_payload_per_message_at_any_size"),
        "the register must name the generated per-message seam assertion"
    );
    assert!(
        entries
            .iter()
            .any(|entry| entry.test == "production_outbound_allocates_one_payload"),
        "the register must name the outbound allocation oracle"
    );

    let missing = missing_register_entries(BASELINES_DOC, &sources).expect("the register parses");
    assert!(
        missing.is_empty(),
        "the inversion register names tests that do not exist: {missing:?}"
    );
}

#[test]
fn a_register_entry_naming_a_missing_test_is_reported() {
    let register = format!(
        "{REGISTER_START}\n- `absent_test` in `some/file.rs` — flips when the item \
         lands\n{REGISTER_END}\n"
    );
    let sources = [("some/file.rs", "fn present_test() {}")];

    assert_eq!(
        missing_register_entries(&register, &sources).expect("the register parses"),
        vec!["absent_test in some/file.rs".to_owned()],
        "a register entry naming a missing test must be reported"
    );
}

#[test]
fn a_wrapped_register_entry_is_parsed() {
    // The baseline document wraps each entry after `in`, so this is the format
    // the check actually has to read.
    let register = format!(
        "{REGISTER_START}\n- `present_test` in\n  `some/file.rs` — flips when the item \
         lands\n{REGISTER_END}\n"
    );
    let sources = [("some/file.rs", "fn present_test() {}")];

    assert!(
        missing_register_entries(&register, &sources)
            .expect("the register parses")
            .is_empty(),
        "a wrapped entry must be joined before its spans are read"
    );
}

#[test]
fn a_register_without_markers_is_rejected() {
    assert!(
        register_entries("- `present_test` in `some/file.rs`").is_err(),
        "a register without markers must be an error, not an empty entry list"
    );
}

#[test]
fn an_entry_without_both_spans_is_rejected() {
    let register = format!("{REGISTER_START}\n- `present_test` only\n{REGISTER_END}\n");
    assert!(
        register_entries(&register).is_err(),
        "an entry that does not name a test and a file must be reported"
    );
}
