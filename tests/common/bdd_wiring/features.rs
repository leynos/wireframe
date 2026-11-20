//! Feature titles and `#[scenario]` binding extraction.
//!
//! `bindings` and `check` consume these helpers to enforce that every scenario
//! title in a feature file, including those grouped under a `Rule`, has exactly
//! one matching binding in its family's scenario module. Extracting the titles
//! with the harness's own parser keeps that enforcement aligned with what
//! `rstest-bdd` validates at compile time.

use syn::{Attribute, Item, Lit, Meta, Token};

/// One parsed `#[scenario]` attribute from a scenario module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScenarioBinding {
    /// Literal `path` argument, when the attribute provided one.
    pub(crate) path: Option<String>,
    /// Literal `name` argument, when the attribute provided one.
    pub(crate) name: Option<String>,
    /// Reason the binding cannot be checked, when it is unsupported.
    pub(crate) problem: Option<String>,
}

/// Parse Gherkin source with the harness's parser and return scenario titles.
///
/// `rstest-bdd` parses feature files with the `gherkin` crate, so using the
/// same parser keeps the check aligned with compile-time validation. That
/// parser recognizes every scenario heading (`Scenario`, `Example`,
/// `Scenario Outline`, and `Scenario Template`) and keeps doc strings, data
/// tables, and comments out of the title list. Scenarios grouped under a
/// `Rule` are collected too, because the parser stores them on the rule rather
/// than on the feature.
///
/// # Errors
///
/// Returns a message when the source is not valid Gherkin.
pub(crate) fn feature_titles(contents: &str) -> Result<Vec<String>, String> {
    let feature = gherkin::Feature::parse(contents, gherkin::GherkinEnv::default())
        .map_err(|error| format!("cannot parse feature file: {error}"))?;
    let rule_scenarios = feature.rules.into_iter().flat_map(|rule| rule.scenarios);
    Ok(feature
        .scenarios
        .into_iter()
        .chain(rule_scenarios)
        .map(|scenario| scenario.name)
        .collect())
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
        set_string_argument(meta, &mut binding.path, "path", &mut binding.problem)
    } else if meta.path.is_ident("name") {
        set_string_argument(meta, &mut binding.name, "name", &mut binding.problem)
    } else if meta.path.is_ident("index") {
        parse_index_argument(meta, binding)
    } else {
        consume_value(meta)
    }
}

/// Store a string argument, recording a non-literal value as a problem.
fn set_string_argument(
    meta: &syn::meta::ParseNestedMeta<'_>,
    field: &mut Option<String>,
    name: &str,
    problem: &mut Option<String>,
) -> syn::Result<()> {
    match take_string(meta)? {
        Some(value) => *field = Some(value),
        None => {
            problem.get_or_insert_with(|| format!("`{name}` must be a string literal"));
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
