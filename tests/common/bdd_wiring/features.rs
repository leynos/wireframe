//! Feature titles and `#[scenario]` binding extraction.

use camino::Utf8PathBuf;
use syn::{Attribute, Item, Lit, Meta, Token};

/// One parsed `#[scenario]` attribute from a scenario module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScenarioBinding {
    /// Literal `path` argument, when the attribute provided one.
    pub(crate) path: Option<Utf8PathBuf>,
    /// Literal `name` argument, when the attribute provided one.
    pub(crate) name: Option<String>,
    /// Reason the binding cannot be checked, when it is unsupported.
    pub(crate) problem: Option<String>,
}

/// Return the trimmed scenario titles from Gherkin source.
pub(crate) fn feature_titles(contents: &str) -> Vec<String> {
    contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| {
            line.strip_prefix("Scenario Outline:")
                .or_else(|| line.strip_prefix("Scenario:"))
                .map(|title| title.trim().to_owned())
        })
        .collect()
}

/// Parse every top-level `#[scenario]` binding in a scenario module.
pub(crate) fn scenario_bindings(file: &syn::File) -> Vec<ScenarioBinding> {
    let mut bindings = Vec::new();
    for item in &file.items {
        let Item::Fn(function) = item else {
            continue;
        };
        for attribute in function
            .attrs
            .iter()
            .filter(|attribute| is_scenario_attribute(attribute))
        {
            bindings.push(parse_binding(attribute));
        }
    }
    bindings
}

/// Report whether an attribute's final path segment is `scenario`.
fn is_scenario_attribute(attribute: &Attribute) -> bool {
    attribute
        .path()
        .segments
        .last()
        .is_some_and(|segment| segment.ident == "scenario")
}

/// Parse one `#[scenario(...)]` attribute.
fn parse_binding(attribute: &Attribute) -> ScenarioBinding {
    let mut binding = ScenarioBinding {
        path: None,
        name: None,
        problem: None,
    };
    if !matches!(attribute.meta, Meta::List(_)) {
        binding.problem = Some("scenario attribute must use `path` and `name`".to_owned());
        return binding;
    }
    let parse_result = attribute.parse_nested_meta(|meta| parse_argument(&meta, &mut binding));
    if let Err(error) = parse_result {
        binding
            .problem
            .get_or_insert_with(|| format!("cannot parse scenario attribute: {error}"));
    }
    if binding.name.is_none() {
        binding
            .problem
            .get_or_insert_with(|| "scenario binding must provide a `name`".to_owned());
    }
    if binding.path.is_none() {
        binding
            .problem
            .get_or_insert_with(|| "scenario binding must provide a `path`".to_owned());
    }
    binding
}

/// Parse one named argument from a `#[scenario]` attribute.
fn parse_argument(
    meta: &syn::meta::ParseNestedMeta<'_>,
    binding: &mut ScenarioBinding,
) -> syn::Result<()> {
    if meta.path.is_ident("path") {
        parse_path_argument(meta, binding)
    } else if meta.path.is_ident("name") {
        parse_name_argument(meta, binding)
    } else if meta.path.is_ident("index") {
        parse_index_argument(meta, binding)
    } else {
        consume_value(meta)
    }
}

/// Parse the `path` argument, recording a non-literal value as a problem.
fn parse_path_argument(
    meta: &syn::meta::ParseNestedMeta<'_>,
    binding: &mut ScenarioBinding,
) -> syn::Result<()> {
    match take_string(meta)? {
        Some(value) => binding.path = Some(Utf8PathBuf::from(value)),
        None => {
            binding
                .problem
                .get_or_insert_with(|| "`path` must be a string literal".to_owned());
        }
    }
    Ok(())
}

/// Parse the `name` argument, recording a non-literal value as a problem.
fn parse_name_argument(
    meta: &syn::meta::ParseNestedMeta<'_>,
    binding: &mut ScenarioBinding,
) -> syn::Result<()> {
    match take_string(meta)? {
        Some(value) => binding.name = Some(value),
        None => {
            binding
                .problem
                .get_or_insert_with(|| "`name` must be a string literal".to_owned());
        }
    }
    Ok(())
}

/// Reject an `index` selector, which cannot be compared with feature titles.
fn parse_index_argument(
    meta: &syn::meta::ParseNestedMeta<'_>,
    binding: &mut ScenarioBinding,
) -> syn::Result<()> {
    consume_value(meta)?;
    binding
        .problem
        .get_or_insert_with(|| "`index` selectors are not supported; name the scenario".to_owned());
    Ok(())
}

/// Consume an argument value and return it when it is a string literal.
fn take_string(meta: &syn::meta::ParseNestedMeta<'_>) -> syn::Result<Option<String>> {
    let value: syn::Expr = meta.value()?.parse()?;
    Ok(match value {
        syn::Expr::Lit(syn::ExprLit {
            lit: Lit::Str(literal),
            ..
        }) => Some(literal.value()),
        _ => None,
    })
}

/// Consume an argument value without interpreting it.
fn consume_value(meta: &syn::meta::ParseNestedMeta<'_>) -> syn::Result<()> {
    if meta.input.peek(Token![=]) {
        let _value: syn::Expr = meta.value()?.parse()?;
    }
    Ok(())
}
