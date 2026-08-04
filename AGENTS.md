# Repository instructions

Before creating or modifying code, tests, scripts, configuration, or developer
documentation, read `docs/standards/HUMAN_REVIEW_STYLE_GUIDE.md` in full. Do not
begin implementation after reading only part of the guide or a summary of it.

Treat that guide as the repository's default standard for naming, functions,
control flow, state, contracts, error handling, comments, tests, change hygiene,
and file layout. More specific conventions already established in a file or
subtree take precedence, as the guide itself specifies.

The orchestrator does not read this guide and does not author code. It may edit
Markdown prose and apply wording a human has approved verbatim. Anything
touching code, tests, scripts, configuration, or build files is delegated to an
agent that reads the guide in full — regardless of how small the change is.

Before starting implementation work, classify it using
`docs/standards/AGENTIC_PIPELINE_PLAYBOOK.md`. Read the playbook in full for
Significant or Critical work; do not begin after reading only part of it or a
summary. Those profiles govern research, human design and plan gates,
implement/review cycles, and the evidence bar for claims about tests, mutation
coverage, and re-execution. Routine work follows the playbook's universal rails
and the style guide above without manufacturing the heavier pipeline artifacts.

GitHub issues are the source of truth for material tracked work: roadmap
features, defects or investigations that need scheduling, and follow-ups the
project intends to revisit. Create the issue before implementation begins.
Directly requested Routine work does not need a bookkeeping issue, and an
unscheduled observation remains a note until the project accepts it as planned
work. Issue #1 is the roadmap index; keep its linked issues current and close
them when their stated outcome is complete. `docs/design/` owns durable
architectural decisions and API contracts; it is not a backlog.

Establish upstream API contracts and implementation details by reading Frigate's
own source at the revision the deployment is running, not from documentation, a
newer release, or inference. Obtain that source if you do not already have it.

The deployed system is read-only to agents. Reading from it is expected: port-forward
to the running Frigate, go2rtc, and nginx and verify route behavior and contracts
against what is actually serving. Mutating it is a human action. If the work would
apply a manifest, edit configuration inside the pod, replace an image or its digest,
or restart a workload, stop and ask; propose the change and let a human apply it. Do
not proceed on judgment.

When a change uncovers a Frigate API incompatibility, ambiguous contract,
surprising behavior, or upstream defect that should influence Corvette's own API,
record the finding and the intended Corvette contract in
`docs/design/api-contracts.md`, and link it from the relevant GitHub issue. Keep
the note with the relevant API contract rather than leaving the rationale only
in an issue, commit message, or compatibility workaround.

When changing this repository:

- Inspect nearby code and existing project-wide helpers before introducing a new
  name, abstraction, dependency, or error-handling pattern.
- Keep changes focused on one intent and avoid unrelated renames, formatting, or
  cleanup.
- Add or update tests for behavioral changes. Verify that a new regression test
  fails when the defect is reintroduced whenever practical.
- Run the formatters, linters, and tests relevant to every changed file. Use the
  Rust toolchain pinned by `rust-toolchain.toml`; `nix develop` provides it.
- Before committing, compare the staged change with its GitHub issue and
  `docs/design/`. Update or close the issue when its outcome or next step has
  changed; update the design documents when an architectural decision or API
  contract has changed. Otherwise explicitly confirm that both remain accurate.
- Review the final diff against the full style guide before declaring the work
  complete. Report checks that could not be run and why.

Do not weaken tests, lint levels, safety checks, or error reporting merely to
make a check pass. Do not add exceptions to mechanical checks without an adjacent
rationale that explains why the flagged construct is correct here.

Every Clippy bypass, including item-level `allow` attributes and project lint
configuration changes, requires explicit human review. Use a bypass only when
there is no reasonable code or design change that resolves the warning.
