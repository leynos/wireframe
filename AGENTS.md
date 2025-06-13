# Assistant Instructions

## Code Style and Structure

- **Code is for humans.** Write your code with clarity and empathy—assume a
  tired teammate will need to debug it at 3 a.m.
- **Comment *why*, not *what*.** Explain assumptions, edge cases, trade-offs, or
  complexity. Don't echo the obvious.
- **Clarity over cleverness.** Be concise, but favour explicit over terse or
  obscure idioms. Prefer code that's easy to follow.
- **Use functions and composition.** Avoid repetition by extracting reusable
  logic. Prefer generators or comprehensions, and declarative code to imperative
  repetition when readable.
- **Small, meaningful functions.** Functions must be small, clear in purpose,
  single responsibility, and obey command/query segregation.
- **Clear commit messages.** Commit messages should be descriptive, explaining
  what was changed and why.
- **Name things precisely.** Use clear, descriptive variable and function names.
  For booleans, prefer names with `is`, `has`, or `should`.
- **Structure logically.** Each file should encapsulate a coherent module. Group
  related code (e.g., models + utilities + fixtures) close together.
- **Group by feature, not layer.** Colocate views, logic, fixtures, and helpers
  related to a domain concept rather than splitting by type.

## Documentation Maintenance

- **Reference:** Use the markdown files within the `docs/` directory as a
  knowledge base and source of truth for project requirements, dependency
  choices, and architectural decisions.
- **Update:** When new decisions are made, requirements change, libraries are
  added/removed, or architectural patterns evolve, **proactively update** the
  relevant file(s) in the `docs/` directory to reflect the latest state. Ensure
  the documentation remains accurate and current.

## Change Quality & Committing

- **Atomicity:** Aim for small, focused, atomic changes. Each change (and
  subsequent commit) should represent a single logical unit of work.
- **Quality Gates:** Before considering a change complete or proposing a commit,
  ensure it meets the following criteria:
  - Passes all relevant unit and behavioral tests according to the guidelines
    above.
  - Passes lint checks
  - Adheres to formatting standards tested using a formatting validator.
- **Committing:**
  - Only changes that meet all the quality gates above should be committed.
  - Write clear, descriptive commit messages summarizing the change, following
    these formatting guidelines:
    - **Imperative Mood:** Use the imperative mood in the subject line (e.g.,
      "Fix bug", "Add feature" instead of "Fixed bug", "Added feature").
    - **Subject Line:** The first line should be a concise summary of the change
      (ideally 50 characters or less).
    - **Body:** Separate the subject from the body with a blank line. Subsequent
      lines should explain the *what* and *why* of the change in more detail,
      including rationale, goals, and scope. Wrap the body at 72 characters.
    - **Formatting:** Use Markdown for any formatted text (like bullet points or
      code snippets) within the commit message body.
  - Do not commit changes that fail any of the quality gates.

## Refactoring Heuristics & Workflow

- **Recognizing Refactoring Needs:** Regularly assess the codebase for potential
  refactoring opportunities. Consider refactoring when you observe:
  - **Long Methods/Functions:** Functions or methods that are excessively long
    or try to do too many things.
  - **Duplicated Code:** Identical or very similar code blocks appearing in
    multiple places.
  - **Complex Conditionals:** Deeply nested or overly complex `if`/`else` or
    `switch` statements (high cyclomatic complexity).
  - **Large Code Blocks for Single Values:** Significant chunks of logic
    dedicated solely to calculating or deriving a single value.
  - **Primitive Obsession / Data Clumps:** Groups of simple variables (strings,
    numbers, booleans) that are frequently passed around together, often
    indicating a missing class or object structure.
  - **Excessive Parameters:** Functions or methods requiring a very long list of
    parameters.
  - **Feature Envy:** Methods that seem more interested in the data of another
    class/object than their own.
  - **Shotgun Surgery:** A single change requiring small modifications in many
    different classes or functions.
- **Post-Commit Review:** After committing a functional change or bug fix (that
  meets all quality gates), review the changed code and surrounding areas using
  the heuristics above.
- **Separate Atomic Refactors:** If refactoring is deemed necessary:
  - Perform the refactoring as a **separate, atomic commit** *after* the
    functional change commit.
  - Ensure the refactoring adheres to the testing guidelines (behavioral tests
    pass before and after, unit tests added for new units).
  - Ensure the refactoring commit itself passes all quality gates.

## Rust Specific Guidance

This repository is written in Rust and uses Cargo for building and dependency
management. Contributors should follow these best practices when working on the
project:

- Run cargo fmt, cargo clippy -- -D warnings, and RUSTFLAGS="-D warnings" cargo
  test before committing.
- Clippy warnings MUST be disallowed.
- Fix any warnings emitted during tests in the code itself rather than silencing them.
- Where a function is too long, extract meaningfully named helper functions
  adhering to separation of concerns and CQRS.
- Where a function has too many parameters, group related parameters in
  meaningfully named structs.
- Where a function is returning a large error consider using `Arc` to reduce the
  amount of data returned.
- Write unit and behavioural tests for new functionality. Run both before and
  after making any change.
- Document public APIs using Rustdoc comments (`///`) so documentation can be
  generated with cargo doc.
- Prefer immutable data and avoid unnecessary `mut` bindings.
- Handle errors with the `Result` type instead of panicking where feasible.
- Use explicit version ranges in `Cargo.toml` and keep dependencies up-to-date.
- Avoid `unsafe` code unless absolutely necessary and document any usage
  clearly.

## Markdown Guidance

- Validate Markdown files using `markdownlint`.
- Validate Markdown Mermaid diagrams using the `nixie` CLI. The tool is
  already installed; run `nixie` directly instead of using `npx`.

### Key Takeaway

These practices help maintain a high-quality codebase and facilitate
collaboration.
