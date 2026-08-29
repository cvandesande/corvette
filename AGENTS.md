# Repository instructions

The tool-neutral pipeline, style guide, and role contracts are canonical at
`/home/cvandesande/.config/ai-agent/standards/` — see
`docs/standards/AGENTIC_PIPELINE_PLAYBOOK.md` and
`docs/standards/HUMAN_REVIEW_STYLE_GUIDE.md` for what each role loads from there.
This file holds only what is specific to this repository.

The orchestrator does not read the style guide and does not author code. It may
edit Markdown prose and apply wording a human has approved verbatim. Anything
touching code, tests, scripts, configuration, or build files is delegated to an
agent that reads the style guide in full — regardless of how small the change is.

Unless a plan or brief already sets a model or effort level for an item, implementer
and reviewer agents default to Sonnet, with the effort level chosen per task from its
presumed difficulty rather than a fixed value. A plan's own per-item cost assignment
(as in `.agents/*/PLAN-*.md`) takes precedence over this default.

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

The deployed system defaults to read-only for agents. Reading from it is expected:
port-forward to the running Frigate, go2rtc, and nginx and verify route behavior and
contracts against what is actually serving. Mutating it — applying a manifest, editing
configuration inside the pod, replacing an image or its digest, or restarting a
workload — requires stopping and proposing the change first. An agent may carry out
the mutation itself only once a human has explicitly authorized that specific action
in the conversation; absent that, let a human apply it. Do not proceed on judgment,
and do not treat authorization for one action or session as standing permission for
later ones.

When a change uncovers a Frigate API incompatibility, ambiguous contract,
surprising behavior, or upstream defect that should influence Corvette's own API,
record the finding and the intended Corvette contract in
`docs/design/api-contracts.md`, and link it from the relevant GitHub issue. Keep
the note with the relevant API contract rather than leaving the rationale only
in an issue, commit message, or compatibility workaround.

Use the Rust toolchain pinned by `rust-toolchain.toml`; `nix develop` provides it.

Before committing, compare the staged change with its GitHub issue and
`docs/design/`. Update or close the issue when its outcome or next step has
changed; update the design documents when an architectural decision or API
contract has changed. Otherwise explicitly confirm that both remain accurate.

Every Clippy bypass, including item-level `allow` attributes and project lint
configuration changes, requires explicit human review. Use a bypass only when
there is no reasonable code or design change that resolves the warning.
