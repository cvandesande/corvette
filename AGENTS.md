# Repository instructions

Before creating or modifying code, tests, scripts, configuration, or developer
documentation, read `docs/standards/HUMAN_REVIEW_STYLE_GUIDE.md` in full. Do not
begin implementation after reading only part of the guide or a summary of it.

Treat that guide as the repository's default standard for naming, functions,
control flow, state, contracts, error handling, comments, tests, change hygiene,
and file layout. More specific conventions already established in a file or
subtree take precedence, as the guide itself specifies.

Frigate's source is available locally at `~/github/frigate`. Consult that checkout
for upstream API contracts and implementation details before using an external
copy or inferring Frigate behavior.

When changing this repository:

- Inspect nearby code and existing project-wide helpers before introducing a new
  name, abstraction, dependency, or error-handling pattern.
- Keep changes focused on one intent and avoid unrelated renames, formatting, or
  cleanup.
- Add or update tests for behavioral changes. Verify that a new regression test
  fails when the defect is reintroduced whenever practical.
- Run the formatters, linters, and tests relevant to every changed file. Use the
  Rust toolchain pinned by `rust-toolchain.toml`; `nix develop` provides it.
- Before committing, compare the staged change with `docs/roadmap.md`. Update
  completed status, current progress, and the next planned slice when the change
  affects them; otherwise explicitly confirm that the roadmap remains accurate.
- Review the final diff against the full style guide before declaring the work
  complete. Report checks that could not be run and why.

Do not weaken tests, lint levels, safety checks, or error reporting merely to
make a check pass. Do not add exceptions to mechanical checks without an adjacent
rationale that explains why the flagged construct is correct here.

Every Clippy bypass, including item-level `allow` attributes and project lint
configuration changes, requires explicit human review. Use a bypass only when
there is no reasonable code or design change that resolves the warning.
