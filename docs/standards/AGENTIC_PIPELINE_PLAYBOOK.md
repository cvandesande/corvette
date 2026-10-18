# Agentic Pipeline Playbook

**Thesis:** agents are capable implementers but unreliable witnesses. Accept work from inspectable changes and executed evidence, with independence and rigor proportional to the cost of being wrong.

This is the universal core. Load only the applicable companion:

- Orchestration: `roles/ORCHESTRATOR.md`
- Implementation: `roles/IMPLEMENTER.md`
- Independent review: `roles/REVIEWER.md`
- Formal research: `roles/RESEARCHER.md`
- Local worker assignment: `LOCAL_WORKER_POLICY.md`
- Implementer/reviewer code style: `HUMAN_REVIEW_STYLE_GUIDE.md`
- Project instructions: the repository's `AGENTS.md`. Task and report shapes: `templates/`
- Rationale and incidents: `AGENTIC_PIPELINE_FAILURE_CATALOG.md`

Project instructions override workflow details but may not weaken safety, human gates, or required evidence without explicit human approval.

## 1. Select the workflow first

Choose the lightest profile whose triggers cover the work. Record the profile and one-sentence rationale. Discovery may escalate it; lowering it after work starts requires human approval.

| Profile | Triggers | Required path |
|---|---|---|
| **Routine** | Known contract, localized behavior, small blast radius, dependency-free refactor | Verify premise; implement; test behavior; run relevant format/lint/tests; inspect final diff |
| **Significant** | Multiple components, changed contract, uncertain upstream behavior, or consequential decomposition | Research as needed; design and plan human gates; scoped implement/review cycles; integration gate |
| **Critical** | Security/authentication, unsafe/FFI ownership, concurrency, persistence/data loss, destructive or deployment behavior, or public-API compatibility | Full pipeline; independent fact and code review; mutation evidence; isolated re-execution; whole-range gate |

Ambiguity selects the higher profile. Risk overrides diff size: a one-line authentication change is Critical; a large mechanically verified rename may remain Routine.

## 2. Universal rails

These apply to every profile and role:

1. **Human direction:** agents do not decide product direction, architecture tradeoffs, scope expansion, publication, deployment mutation, or irreversible actions.
2. **No external mutation:** do not push, publish, modify deployed systems, or perform destructive actions without the applicable human gate.
3. **Premise before work:** verify unfamiliar contracts from primary sources at the pinned revision and verify repository claims by symbol/source.
4. **Search before adding:** inspect nearby code and project-wide helpers before introducing functions, types, dependencies, or patterns.
5. **Focused changes:** preserve checks and error reporting; do not weaken warnings, tests, or safety controls to obtain a pass.
6. **Behavioral evidence:** test observable production behavior. An agent claim, status counter, or static reading is not execution evidence.
7. **Honest reporting:** report exact commands, outcomes, omissions, environmental failures, and tree state. Never claim a check without its output.
8. **Topic-anchored STOP:** stop when work reaches a named gate, including a denied tool call (report the denial; never reach the same effect by another route); never use confidence-conditioned wording such as “if unsure.”
9. **Independent acceptance:** an author cannot supply the independent verdict on its own work. Check author-not-verifier at every dispatch: if an agent authored anything in the range under review, or will later verify what you are about to assign it, use a fresh agent.
10. **Repository authority:** read and obey all applicable project instruction files and the documents they require.

Record API-contract discoveries in the project’s designated contract or design document when one exists.

## 3. Lifecycle and human gates

Significant and Critical work follows:

```text
RESEARCH (as needed) → DESIGN → FACT REVIEW → HUMAN DESIGN GATE
    → IMPLEMENTATION PLAN → HUMAN PLAN GATE
    → scoped IMPLEMENT → INDEPENDENT REVIEW cycles
    → whole-range integration/evidence gate → HUMAN SHIP/PROMOTE GATE
```

Humans gate direction. Agents gate factual and implementation evidence. Neither substitutes for the other.

Human approval is required for:

- design decisions and decisions of record;
- implementation-plan scope, decomposition, invariants, and model/cost plan;
- scope expansion that changes design or requires judgment;
- lowering a selected risk profile;
- pushes, publication, deployment mutation, destructive actions, and other irreversible effects;
- shipping or promotion.

A newly discovered issue that needs no design or human judgment may be proposed as a new scoped item. It is never silently folded into an already reviewed item.

## 4. Research, design, and planning

Formal research is required when Significant or Critical work depends on an external contract, unfamiliar subsystem, or unresolved technical premise. Use primary sources at the pinned version. Give every load-bearing claim a source and, where practical, a minimal executable reproduction. Mark anything not established as `UNVERIFIED`; “not found” is a result. The orchestrator consumes the findings artifact rather than duplicating the research.

A Significant or Critical design document presents why, options, tradeoffs, scope ceilings, and named decision points. A decision point contains a complete question, materially different options, tradeoffs, and a recommendation, but leaves the decision pending. Only the human fills the decision. An independent fact reviewer checks accuracy and framing, not direction. Approved answers become quoted decisions of record.

The implementation plan orders work by dependency and risk. Each item must fit one agent context and contain:

- **Premise:** facts that must be true before editing;
- **Do:** bounded objective and observable completion condition;
- **Verify:** exact behavioral and integration checks;
- **Mutation:** how the new test is shown to bite, when required;
- **Scope guard:** explicit non-goals and topic-anchored STOP conditions.

The plan also records 5–8 absolute, mechanically checkable invariants; model/effort assignment; agent concurrency ceiling; and item isolation. Prefer compile errors, lints, build checks, and tests over prose assertions. Human plan approval is the last gate before implementation.

## 5. Delegation and model policy

Delegate only work large and independent enough to repay context reconstruction. Batch mechanical changes. Serialize agents sharing a working tree; parallel mutation requires isolated worktrees and non-colliding build directories, ports, and state.

Model assignment follows task uncertainty, risk, demonstrated capability, and independence—not a universal vendor-model floor.

- Frontier models own orchestration, design, and brief writing. All other work goes to the local worker by default: implementation, research, and independent review, including Critical items (standing rule, human-set 2026-10-03).
- The local worker receives a brief whose architecture, invariants, and acceptance criteria are settled. A flaw in the brief returns to planning, not to the worker.
- Escalate an item to a frontier model after repeated failure, as `LOCAL_WORKER_POLICY.md` defines.
- Review runs as a separate worker invocation that sees the diff and the brief but not the implementer's reasoning. It must execute the complete prescribed protocol. A reviewer that skips execution or fabricates evidence is not cheaper. The orchestrator re-runs the checks it relies on.
- The fact-review pass on a written design doc uses Opus specifically, not the default frontier tier (standing rule, human-set 2026-09-05).
- Raise effort before changing model tier when the model is capable but under-thinking. Escalate after a hard review failure or when subtlety was identified in the approved plan.
- Re-benchmark assignments when models, quantization, server options, or tool behavior change. Optimize total cost, including retries and failed reviews—not first-pass token count.

Local profile names and ceilings live in `LOCAL_WORKER_POLICY.md`; mutable runtime details do not belong in this core.

## 6. Evidence matrix

| Requirement | Routine | Significant | Critical |
|---|---:|---:|---:|
| Premise verified by source/symbol | Yes | Yes | Yes |
| Written design and plan gates | If direction changes | Yes | Yes |
| Behavioral tests and relevant checks | Yes | Yes | Yes |
| Independent review agent | Risk-based | Yes | Yes |
| Mutation cycle | Risk-based | When oracle is not self-evident or regression is practical | Yes |
| Reviewer isolated at exact revision | No | Risk-based | Yes |
| Durable file-backed evidence | No | Plan-based | Yes |
| Reviewer-chosen additional mutation | No | Risk-based | Security-critical |
| Whole-range cross-item gate | No | Yes | Yes |

A mutation cycle executes both halves: introduce the target defect and show the new test fail; restore production code and show it pass. “Verified by construction” is not evidence.

For any Critical claim that tool X ran against artifact Y, durable evidence proves: the tool was present in the exact artifact; named checks ran inside it; zero findings were measured only after those facts; and the tested commit is embedded in the evidence.

When an environmental condition cannot be forced, decompose proof into a behavioral assertion for what can be forced plus a unit test demonstrating the harm without the fix. Document the limitation.

## 7. Execution and review rules

Sub-agents run their own commands synchronously in the foreground with explicit timeouts. Completion notifications from unmanaged detached work return to the parent harness, not the sub-agent that started it. Therefore sub-agents must not use `&`, `nohup`, disown, monitor/timer tools, spawned-task waits, or polling loops. Redirecting output is allowed only while the command remains foreground; existing logs are read directly rather than re-awaited. A frontier orchestrator can use a managed background-agent harness when it provides launch, compact status, cancellation, lifecycle cleanup, one-task enforcement, and process-quiescence evidence.

Before sharing a mutable tree with another agent, ensure the prior agent has finished and no detached build/test process remains. A completion message alone does not prove quiescence.

Reviews inspect the full claimed range, re-verify premise and invariants, assess whether tests drive production behavior, execute the required checks and mutations, inspect documentation/contract effects, and report concrete findings by file and line or symbol. A verdict relayed by the author is invalid. Critical review leaves a file-backed incremental report with `PASS` or `FAIL` as its final line.

A failed review goes back as a separate implement dispatch with a targeted fix brief. The reviewer never writes the fix or any patch, and the orchestrator never applies one. The original reviewer then does the delta re-review when practical. The whole-range gate checks only cross-item integration, scope, evidence provenance, and documentation coherence; it does not re-litigate independently passed items.

## 8. Completion

A completed implementation reports structured data:

- item, base, and head;
- premise evidence and reuse-search outcome;
- shipped behavior versus approved scope;
- files and symbols changed;
- exact checks and outcomes;
- mutation and durable-evidence locations when required;
- unresolved risks and out-of-scope findings;
- clean-tree and unpushed status;
- harness-reported model, effort/profile, tool calls, and approximate tokens when the plan requires cost telemetry.

Before acceptance, the orchestrator inspects the actual diff and command output, confirms required evidence and revision provenance, checks tree/process state, and receives the reviewer’s own verdict. Explanations and “environmental failure” classifications are claims to verify, not exemptions.

Production-quality completion follows `HUMAN_REVIEW_STYLE_GUIDE.md`; implementers and reviewers load it through their role contracts rather than duplicating its rules in briefs.

Use the templates for exact brief and report shapes. Consult the failure catalog when changing or challenging a rail.