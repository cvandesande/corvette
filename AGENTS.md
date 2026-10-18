# Repository instructions

## What Corvette is

Corvette is a from-scratch Rust replacement for Frigate, not an application built
on Frigate. Frigate runs today only as the temporary backing for capabilities
Corvette has not yet built. Specify every capability as a Corvette contract under
`/corvette/api/v1/` and record it in `docs/design/api-contracts.md`. Name Frigate
only as the current implementation behind a contract. A design, plan or UI that
uses a Frigate route or Frigate behaviour as its contract is wrong, however
convenient. Frigate's source and the deployed Frigate are sources of facts about
the current backing, not of requirements.

## Working rules

The pipeline, style guide, and role contracts are in `docs/standards/`: read
`docs/standards/AGENTIC_PIPELINE_PLAYBOOK.md` and
`docs/standards/HUMAN_REVIEW_STYLE_GUIDE.md` for what each role loads. Where
`docs/standards/STYLE.md` and the style guide differ, `STYLE.md` takes precedence.
This file holds only what is specific to this repository.

The orchestrator does not read the style guide and does not author code. It may
edit Markdown prose and apply wording a human has approved verbatim. Anything
touching code, tests, scripts, configuration, or build files is delegated to an
agent that reads the style guide in full — regardless of how small the change is.

In conversation, name a step by what it does; never by a plan code (a step,
decision or finding label made of letters and digits).

The playbook sets which model does which work. A plan's own per-item cost
assignment (as in `.agents/*/PLAN-*.md`) takes precedence over it.

Detect browser capability or the observed outcome, never the browser name; an
exception needs a stated reason.

GitHub issues are the source of truth for material tracked work: roadmap
features, defects or investigations that need scheduling, and follow-ups the
project intends to revisit. Create the issue before implementation begins.
Directly requested Routine work does not need a bookkeeping issue, and an
unscheduled observation remains a note until the project accepts it as planned
work. Issue #1 is the roadmap index; keep its linked issues current and close
them when their stated outcome is complete. `docs/design/` owns durable
architectural decisions and API contracts; it is not a backlog.

Establish the current backing's behaviour and implementation details by reading
Frigate's own source at the revision the deployment is running, not from
documentation, a newer release, or inference. Obtain that source if you do not
already have it.

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

<!-- seed:begin -->
## Standards (managed by the project seed; change the seed, not this section)

The pipeline, role contracts and templates are in `docs/standards/`.

Before any non-trivial task, and again after context compaction, read all of `docs/standards/AGENTIC_PIPELINE_PLAYBOOK.md`, every line, never an excerpt. Before changing files, classify the task as Routine, Significant, or Critical using the playbook. Record the profile and one-sentence rationale.

When compacting, preserve the current item and its brief path, the risk profile, the list of modified files, the checks run so far, and any open human gate.

A new process failure changes the standards at the source: fix the rail in the project seed repository, review and commit it there, then run its `bootstrap.sh --update` for this project. Do not edit a seed-owned file or this section in this project.
<!-- seed:worker:begin -->

Send implementation, research, and independent review to the `local-worker` agent, with a brief compiled from `docs/standards/templates/IMPLEMENTATION_BRIEF.md`. `docs/standards/LOCAL_WORKER_POLICY.md` sets assignment, concurrency, and escalation.
<!-- seed:worker:end -->
<!-- seed:end -->
