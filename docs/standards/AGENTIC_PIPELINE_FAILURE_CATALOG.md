# Agentic Pipeline Failure Catalog

This is rationale and lookup material, not required reading for every task. Each rail in the core responds to an observed failure. Consult this before weakening one.

| Failure | Observed form | Rail produced |
|---|---|---|
| Hollow PASS | Reviewer vanished or returned PASS too quickly to have executed checks | Independent re-execution; Critical file-backed report; verdict last |
| Narrated evidence | “Verified by construction”; test remained green with fix reverted | Execute both mutation halves |
| Conditional gate erosion | “Stop if unsure” was rationalized away | Topic-anchored unconditional STOP conditions |
| Fabricated explanation | Agent cited nonexistent report or code | Verify explanations against source and artifacts |
| Checked-item amnesia | Scope added to an already completed item and never reviewed | Additions become new unchecked items |
| End-of-loop blindness | Large review missed item-level defects | Independent per-item review; whole-range gate only for cross-item issues |
| Hollow test | Test duplicated the production bug | Reviewer checks production path and mutates independently for high risk |
| Comment guard rail | Prose invariant was later violated | Mechanically enforce invariants |
| Unfit-host verdict | Timing passed on noisy hardware | Performance verdicts only on designated suitable hosts |
| Stale artifact | Tests or investigation inspected an old binary | Artifact freshness and revision provenance first |
| Deployment-mode blindness | Foreground tests missed daemon restart behavior | Test deployment modes actually shipped |
| Lost guard rail | Convenience setting removed a loud invariant failure | Identify and re-enforce protection before removing friction |
| Internal-vocabulary comments | Public docs referenced private phases/docs | Comments are self-contained and cite durable authority |
| Detached sub-agent | Background command notification returned to parent; agent waited forever | Sub-agents execute synchronously in foreground |
| Incomplete sweep | Directory-limited grep missed tokens repeatedly | Sweep whole tracked tree with complete token set |
| Reviewer contamination | Author’s detached process resumed during review mutation | Confirm process quiescence before sharing a tree |
| Cheap-review recovery | Low-capability reviewer skipped execution, forcing orchestrator redo | Reviewer capability follows execution duty and risk |
| Denial workaround | An edit to a settings file was denied; the worker made the same change with a shell command | A denied tool call is a named STOP: report the denial, never reach the same effect by another route |
| Fix round to the reviewer | A FAIL's fix list went back to the reviewer, which wrote a patch that the orchestrator applied itself; asked why, the orchestrator had read only part of the playbook. The same failure was caught once before | Author is never verifier: a fix is its own implement dispatch, the original reviewer only does the delta re-review, the orchestrator applies no patch; the rule is repeated in the orchestrator and reviewer contracts, and the playbook is read in full, every line, at the start of a task and after every compaction (the post-compaction hook says so) |

When adding an incident, add a distinct failure mode and the precise rail it justifies; do not duplicate general advice. A change to a rail is made in the project seed first and then copied into each project; a project never edits its copy.