//! Syntax-aware resolution of Rust module declarations.
//!
//! Registries declare scenario and fixture modules either directly or through a
//! `#[path]` attribute. This module resolves both forms against a snapshot so
//! evaluation can compare declared targets with the expected file layout.

use camino::{Utf8Path, Utf8PathBuf};

use super::snapshot::{RepoSnapshot, normalize_path};

/// One external `mod name;` declaration resolved against a snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ModuleDecl {
    /// Declared module name.
    pub(crate) name: String,
    /// Whether the declaration uses plain `pub` visibility.
    pub(crate) is_public: bool,
    /// Repository-relative file the declaration resolves to.
    pub(crate) target: Utf8PathBuf,
}

/// Parse and resolve the external module declarations in `path`.
///
/// Inline modules are ignored because their contents live in the declaring
/// file and have no separate target to register.
///
/// # Errors
///
/// Returns a message when the file is absent from the snapshot or is not valid
/// Rust syntax.
pub(crate) fn module_declarations(
    snapshot: &RepoSnapshot,
    path: &Utf8Path,
) -> Result<Vec<ModuleDecl>, String> {
    let contents = snapshot
        .get(path)
        .ok_or_else(|| format!("cannot read `{path}`"))?;
    let file =
        syn::parse_file(contents).map_err(|error| format!("cannot parse `{path}`: {error}"))?;
    Ok(file
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Mod(module) => module_declaration(snapshot, path, module),
            _ => None,
        })
        .collect())
}

/// Resolve one external module declaration, ignoring inline modules.
fn module_declaration(
    snapshot: &RepoSnapshot,
    declaring_file: &Utf8Path,
    module: &syn::ItemMod,
) -> Option<ModuleDecl> {
    if module.content.is_some() {
        return None;
    }
    let name = module.ident.to_string();
    Some(ModuleDecl {
        is_public: matches!(module.vis, syn::Visibility::Public(_)),
        target: resolve_target(snapshot, declaring_file, &name, path_attribute(module)),
        name,
    })
}

/// Extract the literal value of a `#[path = "..."]` attribute.
fn path_attribute(module: &syn::ItemMod) -> Option<Utf8PathBuf> {
    module.attrs.iter().find_map(|attribute| {
        if !attribute.path().is_ident("path") {
            return None;
        }
        match &attribute.meta {
            syn::Meta::NameValue(name_value) => literal_path(&name_value.value),
            _ => None,
        }
    })
}

/// Convert a literal string expression into a path.
fn literal_path(expression: &syn::Expr) -> Option<Utf8PathBuf> {
    match expression {
        syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(value),
            ..
        }) => Some(Utf8PathBuf::from(value.value())),
        _ => None,
    }
}

/// Resolve the file a module declaration targets.
fn resolve_target(
    snapshot: &RepoSnapshot,
    declaring_file: &Utf8Path,
    name: &str,
    path_attribute: Option<Utf8PathBuf>,
) -> Utf8PathBuf {
    if let Some(relative) = path_attribute {
        return normalize_path(&declaring_directory(declaring_file).join(relative));
    }
    let module_directory = module_directory(declaring_file);
    let flat = module_directory.join(format!("{name}.rs"));
    if snapshot.contains(&flat) {
        return flat;
    }
    let nested = module_directory.join(name).join("mod.rs");
    if snapshot.contains(&nested) {
        return nested;
    }
    flat
}

/// Return the directory containing the declaring file.
fn declaring_directory(file: &Utf8Path) -> &Utf8Path {
    file.parent().unwrap_or_else(|| Utf8Path::new(""))
}

/// Return the directory Rust searches for modules declared in `file`.
///
/// `mod.rs` and crate-root files search their own directory; any other module
/// file searches a sibling directory named after its stem.
fn module_directory(file: &Utf8Path) -> Utf8PathBuf {
    let parent = declaring_directory(file);
    if file.file_name() == Some("mod.rs") {
        return parent.to_path_buf();
    }
    file.file_stem()
        .map_or_else(|| parent.to_path_buf(), |stem| parent.join(stem))
}
