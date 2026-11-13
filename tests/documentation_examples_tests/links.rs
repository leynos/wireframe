//! Contracts for relative links in user-facing documentation.
//!
//! These documents are the on-ramp, so a dead link strands a reader. Each
//! relative link must name a file in this repository and, where it carries a
//! fragment, a heading in that file.

use std::collections::HashSet;

use camino::Utf8Path;
use cap_std::{ambient_authority, fs_utf8::Dir};
use rstest::rstest;

use super::documentation_examples::DOCUMENTS;

/// Convert a Markdown heading to its GitHub anchor.
fn github_anchor(heading: &str) -> String {
    heading
        .chars()
        .filter_map(|ch| match ch {
            ' ' | '-' => Some('-'),
            '_' => Some('_'),
            _ if ch.is_alphanumeric() => Some(ch.to_ascii_lowercase()),
            _ => None,
        })
        .collect()
}

/// Collect the anchors of every ATX heading outside fenced code.
fn anchors(contents: &str) -> HashSet<String> {
    let mut in_fence = false;
    let mut found = HashSet::new();
    for line in contents.lines() {
        if line.starts_with("```") || line.starts_with("~~~") {
            in_fence = !in_fence;
        } else if !in_fence && line.starts_with('#') {
            found.insert(github_anchor(line.trim_start_matches('#').trim()));
        }
    }
    found
}

/// Remove fenced code and inline code, which may contain link-like text.
fn prose_without_code(contents: &str) -> String {
    let mut in_fence = false;
    let mut prose = String::new();
    for line in contents.lines() {
        if line.starts_with("```") || line.starts_with("~~~") {
            in_fence = !in_fence;
        } else if !in_fence {
            let segments = line.split('`').step_by(2);
            prose.extend(segments);
            prose.push('\n');
        }
    }
    prose
}

/// Extract inline-link and reference-definition targets from Markdown.
fn link_targets(contents: &str) -> Vec<String> {
    // A wrapped link splits across lines, so the text is matched as one string.
    let prose = prose_without_code(contents).replace('\n', " ");
    let inline = prose.split("](").skip(1).filter_map(|tail| {
        let target = tail.split(')').next()?.split_whitespace().next()?;
        Some(target.to_owned())
    });
    let definitions = contents.lines().filter_map(|line| {
        let rest = line.strip_prefix('[')?;
        let (label, target) = rest.split_once("]: ")?;
        (!label.starts_with('^')).then(|| target.split_whitespace().next().map(str::to_owned))?
    });
    inline.chain(definitions).collect()
}

fn is_external(target: &str) -> bool {
    target.starts_with("http://") || target.starts_with("https://") || target.starts_with("mailto:")
}

/// Resolve `target` against `source`, collapsing `..` segments.
fn resolve(source: &str, target: &str) -> String {
    let base = Utf8Path::new(source)
        .parent()
        .unwrap_or_else(|| Utf8Path::new(""));
    let mut parts: Vec<&str> = base.iter().collect();
    for segment in target.split('/') {
        match segment {
            "." | "" => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}

fn documented_contents(path: &str) -> Option<&'static str> {
    DOCUMENTS
        .iter()
        .find(|(document, _)| *document == path)
        .map(|(_, contents)| *contents)
}

#[test]
fn relative_links_resolve_to_files_and_headings() {
    let root = Dir::open_ambient_dir(env!("CARGO_MANIFEST_DIR"), ambient_authority())
        .expect("the repository root should open");
    let mut broken = Vec::new();
    for &(source, contents) in DOCUMENTS {
        for target in link_targets(contents) {
            if is_external(&target) {
                continue;
            }
            let (path, fragment) = target.split_once('#').unwrap_or((&target, ""));
            let resolved = if path.is_empty() {
                source.to_owned()
            } else {
                resolve(source, path)
            };
            if !root.exists(&resolved) {
                broken.push(format!("{source}: {target} (no file)"));
                continue;
            }
            if fragment.is_empty() {
                continue;
            }
            let target_text = documented_contents(&resolved).map_or_else(
                || root.read_to_string(&resolved).unwrap_or_default(),
                str::to_owned,
            );
            if !anchors(&target_text).contains(fragment) {
                broken.push(format!("{source}: {target} (no heading)"));
            }
        }
    }
    assert!(broken.is_empty(), "broken links:\n{}", broken.join("\n"));
}

#[rstest]
#[case("Your first server", "your-first-server")]
#[case("Handlers, `Packet`, and more", "handlers-packet-and-more")]
#[case("Read 1 KiB - fast", "read-1-kib---fast")]
fn anchors_follow_github_rules(#[case] heading: &str, #[case] expected: &str) {
    assert_eq!(github_anchor(heading), expected);
}

#[test]
fn wrapped_links_and_reference_definitions_are_collected() {
    let contents = concat!(
        "See the [wrapped\n",
        "link](docs/a.md#b) and `[code](not-a-link.md)`.\n\n",
        "[ref]: docs/c.md\n",
        "[^1]: docs/footnote.md\n",
        "```text\n",
        "[fenced](not-a-link.md)\n",
        "```\n",
    );
    let targets = link_targets(contents);
    assert_eq!(targets, ["docs/a.md#b", "docs/c.md"]);
}
