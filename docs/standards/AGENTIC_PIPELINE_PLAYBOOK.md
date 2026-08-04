# The Agentic Pipeline Playbook

*Distilled from the ngx-otel-rust project — a production NGINX module built almost
entirely through orchestrated agent pipelines, hardened across three hostile-review
campaigns (32 findings, all fixed or deferred with recorded rationale). Every rule was
paid for by a concrete failure; the failures are kept (see §14) because the rules don't
stick without them.*

**Thesis:** agents are excellent implementers and terrible witnesses. The pipeline never
accepts an agent's testimony about its own work — only artifacts, executed runs, and
independent re-execution.

> **How to read this.** Two kinds of content.
> **Rails** — invariants, gates, the evidence bar, the failure catalog — are enumerated
> exhaustively on purpose: a gap is a hole.
> **Shapes** — briefs, reports, evidence files, comments, findings — are taught by one
> short canonical example, because an example portrays the target better than prose
> describing it.
> When in doubt: *could one good example leave a dangerous gap?* If yes, enumerate.
> If no, show the example.

---

## 0. Lifecycle at a glance

```
RESEARCH ──► DESIGN DOC ──► DOC FACT-REVIEW ──► [HUMAN GATE: answer the DPs]
                                                      │
                                                      ▼
                              IMPLEMENTATION PLAN ──► [HUMAN GATE]
                                                      │
                                  invariants + decisions-of-record locked here
                                                      ▼
                          DECOMPOSE into items (batch the mechanical ones)
                                                      ▼
                  ┌── per item: IMPLEMENT agent ──► REVIEW agent ──► PASS? ──┐
                  │         (FAIL → targeted fix → delta re-review,          │
                  │          escalate model if the item is hard)             │
                  └──────────────────────◄───────────────────────────────────┘
                                                      ▼
                    BATCHED GATE: sanitizers/CI at tip + whole-range review
                              (cross-item concerns ONLY)
                                                      ▼
                                          [HUMAN GATE] ship/promote
```

Humans gate **direction** (design, plan, scope changes, irreversible actions).
Agents gate **correctness** (premise, facts, tests, mutation, re-execution).
Neither substitutes for the other — an agent that approves a direction has replaced the
human gate rather than fed it.

---

## 1. Research phase

Goal: a *sourced* findings document, not a vibe. Wrong premises poison every downstream
phase, so this is where fabrication costs the most.

- **The orchestrator never researches.** Research is always delegated, regardless of
  apparent size — the orchestrator reads the findings document, never the sources. An
  orchestrator that has read the primary sources has spent the context that gating and
  triage depend on, and has disqualified itself under §2 from being the neutral reader
  of the design doc.
- **Primary sources only** — archives, man pages, vendor docs, the dependency's actual
  source at the *pinned* version. Verify by **symbol**, not by upstream docs for a newer
  version.
- **"Not found" is a result.** Record what you could NOT verify, so nobody attributes it
  later.
- **Every load-bearing claim gets a tiny executable repro**, or it's marked UNVERIFIED.
- **Read the incumbent's source for the hard parts.** "Matches the proven design" beats
  novelty in review.
- **Output:** `<FEATURE>_RESEARCH.md`, risks ranked and ID'd so the plan can cite them.

Canonical finding (the shape of one entry):

```
F-3  TLS hostname verification
  CLAIM    crate exposes SSL_set1_host at the pinned version
  VERIFIED grep of vendored src/ssl.rs:1442 — symbol present
  REPRO    repros/f3_set1_host.rs — connects, asserts name-mismatch rejected
  RISK     R-2 (med): no primary source on SAN-vs-CN precedence;
           do NOT attribute SAN-priority behavior — UNVERIFIED
```

## 2. Design doc → human review

- One document for a human decision-maker: the WHY, the options, the tradeoffs, and
  **named decision points** (DP-A, DP-B, …). The human's answers become **decisions of
  record**, quoted verbatim in the plan.
- **Scope ceilings live here** ("match incumbent X's surface under our naming, nothing
  more" kills scope creep before it has a budget).
- **WHY docs stay separate from WHAT docs.** The repo is self-describing for WHAT it
  does; the design doc holds WHY.
- **The doc may be agent-drafted; the decisions may not.** Drafting from the research is
  high-context, low-judgment work and belongs off the orchestrator's context. Answering a
  DP is direction, and direction is the human's. A drafting agent writes each DP complete
  — question, options, tradeoff, and its own recommendation — and leaves
  `DECISION  pending`. It never fills that line, and neither does any reviewer. A DP an
  agent answered is a decision nobody made, and every downstream phase quotes it verbatim.
- **The drafting agent is not one of the researchers.** An agent drafting from its own
  findings carries its own blind spots into the doc with nothing left to catch them; §8's
  independence rule applies here for the same reason it applies to code.
- **The DP section is self-contained and comes first.** The human answers from that
  section alone, so each DP states enough to be decided without reading the body. A DP
  whose framing needs the body is not finished.
- **A doc review agent checks facts, never direction.** Independent of the drafter, its
  verdict is PASS/FAIL on accuracy alone: every load-bearing claim resolves at its cited
  source, nothing the research marked UNVERIFIED is restated as fact, and each DP's
  options are materially different with real tradeoffs. FAIL returns the doc to the
  drafter. PASS means only that the doc is safe to put in front of the human — it is not
  an approval, and the doc reaches the human only after it.

Canonical decision point, as drafted and as decided:

```
DP-B  3xx handling: follow internally, or surface status to the caller?
  Options   (a) follow internally   (b) return upstream status to caller
  Trade     (a) ergonomic, hides hop count;  (b) explicit, matches mod_X
  RECOMMEND (b) — the incumbent's callers already own redirect policy
  DECISION  pending                          ← an agent stops here

  DECISION (human, 2026-05-02): (b). Rationale: parity with incumbent;
           caller owns redirect policy.  ← quoted verbatim into PLAN §A2
```

## 3. Implementation plan → human review

- **Phases ordered by decision, not intuition** — and record the rationale ("C before A:
  C de-risks the FFI surface A depends on"). Re-orderings later are fine but update the
  decision of record.
- Decompose into **items** sized for one agent context (≈ one reviewable commit-series,
  one day to one week of agent effort).
- Each item carries five fields — **Premise / Do / Verify / Mutation / Scope guard**.
  *(Filled canonical item: see Appendix — it's clearer than five abstract definitions.)*
- **Effort + model assignment per item**, so cost is visible at approval (§6).
- **A ceiling on agents in flight at once** (§5), so the fan-out is approved rather than
  discovered.
- **The human reviews the PLAN, not just the design** — the last cheap moment to catch a
  wrong decomposition.

## 4. Invariants — decided at planning, enforced forever

Invariants are the rules **no agent may break in any item**, stated once, pasted into
every brief. A good set is:

- **Small (5–8), absolute, mechanically checkable.** "No allocation on the hot path",
  "zero new code in the master process", "wire bytes unchanged across refactors",
  "dependency Y is frozen — work within existing bindings", "fail closed, never silent
  fallback".
- **Code-enforced, not comment-asserted.** *The single most-repeated hostile-review
  meta-finding.* If an invariant can be a compile error, lint deny, build check, or test
  — make it one.
- **Greppable.** Phrase invariants so a grep can falsify them. Every review ends with the
  invariant sweep, even for items that "obviously can't" violate them — that's where
  violations hide.

Enforced vs asserted — the whole difference:

```
✗ asserted:  // INVARIANT: no allocation on the hot path
✓ enforced:  a build-script check that fails the build on alloc-shaped calls
             under src/hot/, e.g.
               git grep -nE '\b(Box::new|Vec::|String::from|to_owned)\b' src/hot/ \
                 && { echo "alloc on hot path"; exit 1; }
```

A guard rail that lives in prose doesn't exist for the next contributor or the next agent.

## 5. Decomposition and batching

- **Risky items get the full per-item pipeline.** Per-item review caught, same-day, three
  FAILs that end-of-loop review had historically missed — a reviewer holding ONE item's
  context catches what a ten-item reviewer cannot.
- **Mechanical items get batched:** one implement agent, ONE review over the combined
  diff. Four chores ≠ four pipelines. (Measured: a 4-chore batch ran in ~2 agent-runs,
  not 8.)
- **Mid-stream additions are NEW unchecked items.** Never edit a checked item to add
  scope — checked items are write-only memory; nobody re-reads them. (A regression hid
  behind a green checklist exactly this way.)
- **Serialize items sharing a working tree.** Parallelize only with genuine isolation
  (worktrees), and only when items can't collide on build dirs/ports/state.
- **Delegation has a floor and a ceiling.** Spawn an agent only for work that is genuinely
  independent and large enough to repay the overhead — a wide multi-file investigation, an
  item whose research and implementation are separable. Work the orchestrator could finish
  in a handful of tool calls costs more delegated than done: the agent re-establishes
  context, re-explores, and reports back, and the orchestrator then reads the report. When
  one agent can do the job, use one. Current models delegate more readily than the ones
  this playbook was first written against, so the plan states the maximum number of agents
  in flight at once, and exceeding it is a human decision rather than an orchestrator
  judgment call. The floor applies to work *within* a phase, not to the phases
  themselves. Research, design drafting, implementation, and review are always
  delegated; the orchestrator's own tool calls are for deciding what to work on, triage,
  gating, and verifying evidence.
- **The review agent is the sanctioned exception, and it does not generalize.** It exists
  for independence, not for double-checking: an implementer's testimony about its own work
  is not evidence (§8), and no improvement in a model's self-checking changes that. That
  argument covers a reviewer reading someone else's work. It does not license spawning
  agents to re-verify work that already carries executed evidence, and it is not a reason
  to add "verify your work" instructions to an implement brief — §7 duty 4 asks for an
  executed mutation cycle, which produces an artifact, not a reassurance.

Naming — keep the two axes from colliding (write it in the handoff; left implicit it
reliably confuses the next reader — it did here):

```
Phase 1, 2, 3 …      product roadmap (the external / proposal view)
Phase A, B, C …      work breakdown of ONE feature
  items A0, A1, B1   letter+number; each new feature gets its own letter series
Numbers never mean feature-work; letters never mean roadmap.
```

## 6. Model and effort selection — the cost-based policy

The expensive fixed cost is the **strong-model review** after every implement. Optimize
around that fact, across both levers: how capable a model runs the agent, and how hard it
is told to work.

- **Effort is the first lever; model tier is the second.** Current models hold quality at
  low and medium effort for a fraction of the tokens and latency, so the cheapest correct
  configuration is often the strong model at reduced effort rather than Sonnet at full
  effort. Reach for the effort step before the tier change, and step up to the
  highest settings for demanding agentic and coding items rather than leaving them there
  by default.
- **Re-sweep effort whenever the model generation changes.** An effort default carried
  across a model release is a guess, not a measurement — the tradeoff curve moves with the
  model, and the level that was barely adequate before may now be wasteful. Sweep on real
  items from the current phase, never a synthetic case.
- **Shallow work is an effort problem before it is a prompt problem.** When an agent
  under-thinks a hard item, raise its effort rather than adding prose telling it to think
  carefully.
- **Sonnet is the floor — nothing below it runs in this pipeline, in any role.** The tier
  beneath it was tried and withdrawn: observed twice that its reviewers rationalize
  execution away and verdict on static reading alone — once fabricating run evidence
  outright, once claiming the platform blocked a run its brief explicitly provided the
  runner for. With the floor in place the cost question stops being *which cheap model*
  and becomes *how much effort*, which is why effort leads this section.
- **Implement = Sonnet at the effort the item needs**, by default.
- **Upgrade implement to the strong model only when:**
  - (a) the review FAILed **and** the item is genuinely hard (subtle concurrency, FFI
    ownership, design-level fix) — a mechanical miss just gets a Sonnet re-fix + **delta**
    re-review; or
  - (b) the item is **pre-identified as subtle**. Strong-first is *cheaper* there: a
    failed Sonnet attempt burns a full strong-model review on doomed work plus a second
    implement. Rule of thumb: go strong-first when Sonnet's first-pass odds are below
    ~60–70% — but raise Sonnet's effort first, since that is the cheaper half-step.
- **Review tier follows RISK, not habit.** Production / security / unsafe-FFI → strong,
  full rigor. Docs / housekeeping → Sonnet review (no mutation cycle to re-execute, no
  hollow-test risk).
- **A review with an execution duty gets real effort.** The floor settles the model; the
  same failure reappears as an effort setting too low to sustain a run. A reviewer that
  must execute something (suite, harness, mutation, evidence re-run) is not where to
  economize — the orchestrator re-running the skipped half after the fact is recovery,
  not a plan.
- **Measure, don't guess.** Every agent ends with `model / effort / tool-calls /
  approx-tokens`, and the orchestrator records the *harness's* numbers (agent self-counts
  run 30%+ off in both directions). After two items you know where the money goes — and
  without the effort field the sweep above has nothing to compare.

## 7. The implement agent

The brief contains ONLY: that item's section, the invariants, the hard rules, the env
gotchas. Not the whole plan — context is cost and distraction.

Duties, in order:

1. **Verify the premise by symbol/source before any code.** False or by-design premise →
   STOP-AND-ASK with findings + a proposed re-spec. (Two of ten items in one campaign
   were re-specced this way, both correctly; the original spec would have shipped
   nonsense.)
2. **Search before writing.** Before implementing any new function, type, or helper,
   search the codebase for an existing one that does this or nearly does. Exact fit →
   use it. Near fit whose generalization stays within this item's scope → elevate it,
   as a separate commit from the behavioral change. Near fit whose generalization
   touches shared surface → NOTE it as a proposed item, don't do it. Record the search
   outcome in the return (one line: reused X / elevated X / none found).
3. **Implement within scope.** Discovering a NEW issue-worthy defect or debt outside
   scope → STOP-AND-ASK with the finding (topic-anchored; batch multiple discoveries
   into one STOP). The orchestrator triages immediately: needs a redesign or a human
   decision → filed as an issue for the human queue; otherwise the orchestrator may
   rule FIX-NOW, which creates a NEW item with its own scope/verify/review coverage —
   never a silent expansion of the current item. Observations that wouldn't become
   tracker issues stay NOTEs in the return.
4. **Write the test, then run the mutation cycle** (below). *"Verified by construction"
   is banned vocabulary.*
5. **Verify on each platform the matrix requires, proportional to the diff** (doc-only ≠
   full matrix; production code = full matrix).
6. **Commit in reviewable units; never push.** Pushing is an orchestrator action taken
   only on explicit human approval — agents leave their commits local and report
   `git log origin/main..HEAD`. Confirm secondary checkouts landed where the item
   requires it.
7. **Report as DATA, not prose** (shape in Appendix).
8. **End with the exit checklist, pasted and checked** (below). Agents drift on
   *protocol* long before *capability*; a pasted checklist catches the drift mechanically
   and costs nothing. Don't escalate models to fix protocol lapses — fix the brief.

Mutation cycle — both halves, executed, committed:

```
$ # break it
$ sed -i 's/if mismatch { reject }/if false { reject }/' src/tls.rs
$ cargo test tls::rejects_name_mismatch
test tls::rejects_name_mismatch ... FAILED          ← the new test bites
$ git checkout src/tls.rs
$ cargo test tls::rejects_name_mismatch
test tls::rejects_name_mismatch ... ok              ← restored, passes
# both runs pasted into evidence/A1-mutation.log, commit 9f3c1a2 embedded
```

Exit checklist — paste it, check each line, do not paraphrase:

```
[ ] NOT pushed — prove it: `git log origin/main..HEAD` lists exactly your commits
[ ] remote/secondary checkouts HEAD-verified
[ ] evidence file committed at tip, commit hash embedded
[ ] all trees clean (`git status`), scratch removed
[ ] cost line present:  model=<m> effort=<e> tool-calls=<n> approx-tokens=<t>
```

## 8. The review agent

**Independent, never the author, and it re-executes — never trusts.**

- Runs in an **isolated worktree** at the exact commit under review.
- **Foreground execution only — every brief says so explicitly, naming the tools.**
  **The mechanism, so agents stop rediscovering it the hard way:** a completion
  notification (a backgrounded shell exit, a Monitor/timer firing, a spawned task
  finishing) is delivered to the *parent loop* — the orchestrator — never to the
  sub-agent that started the work. A sub-agent that hands off to any asynchronous
  signal has severed its own only wake-up path: it will "wait for the monitor" forever,
  because the monitor reports to someone else. There is no in-agent event it can block
  on. This is structural, not a tuning problem — no timeout, retry, or politeness
  fixes it. Therefore, inside a sub-agent, ALL of these are banned, by mechanism name:
  - no `&`-backgrounded shell commands, no `nohup`/disown, no `cmd &` + poll loop;
  - no Monitor tool, no timer/wakeup/cron, no ScheduleWakeup;
  - no spawning a task/agent and awaiting its notification;
  - no "I'll resume when X signals" of any kind.

  What to do instead: one **foreground** command with a generous explicit timeout
  (chunk per test binary if a single run would exceed it); OR redirect to a file in the
  same foreground step and read the file the instant the command returns —
  `cargo test … > run.log 2>&1; tail -50 run.log` is one synchronous step, not a
  detach. If a prior step already wrote a log, READ THE LOG OFF DISK — never re-run or
  wait. If the harness auto-backgrounds a long command, that is not a signal to wait:
  read its output file off disk immediately. This applies to implement agents equally;
  it lives here because reviews run the longest suites. (Observed repeatedly across
  batches, including *after* briefs that named the prohibition — most recently a
  sub-agent that spawned a Monitor loop and stalled reporting "waiting for the monitor
  to signal the run finished." The brief line alone does not stick; the mechanism
  explanation is the load-bearing part.)
- **File-backed incremental report** (`<item>-review-report.md`): a timestamped section
  after each check, VERDICT as the last line. (This exists because a reviewer once
  *vanished*, leaving only a "PASS" stamped too soon to be real.)
  - **A verdict with no surviving evidence artifact is VOID** — relaunch, don't accept.
  - The orchestrator sanity-checks verdict wall-clock against the prescribed work.
- **Checklist core** (tune per item): **1** scope — full diff vs claimed files, invariant
  grep, nothing swept in; **2** premise — re-verify the author's factual claims at the
  source; **3** re-execute the mutation cycle, plus — for security-critical items — **one
  additional mutation of the reviewer's own choosing** (reviewer-chosen mutations are what
  killed the hollow-test class); **4** test quality — do tests drive PRODUCTION code, or a
  test-local reimplementation of the same bug? **5** domain deep audit against
  authoritative semantics (man pages, spec), not the author's comments; **6** full suite
  on the required platforms, leave every tree clean and SAY SO.
- **FAIL → targeted fix → DELTA re-review** (the original reviewer continues with
  context), not a from-scratch review. Escalate the implement model per §6.

A finding is concrete or it isn't a finding:

```
✗  "I'm not comfortable with the error handling here."
✓  src/io.rs:88 — read() error mapped to Ok(0); the caller treats 0 as EOF,
   so a transient EINTR silently truncates the stream. Repro: tests/io_eintr.rs
   (added) FAILs pre-fix, passes post-fix.  → FAIL
```

## 9. Hard gates (the non-negotiables)

- **STOP-AND-ASK is unconditional and topic-anchored.** Agents *rationalize
  confidence-conditioned gates away* — observed repeatedly, never observed with
  topic-anchored ones. Irreversible actions (deletions, pushes to shared state, public
  posts) are always topic-anchored gates.

  ```
  ✗  "STOP if you're unsure."            (the agent is never unsure)
  ✓  "If the change touches wire bytes, the public API, or deletes files
      → STOP and ask. Do not proceed on judgment."
  ```

- **Evidence bar for any "ran X under tool Y" claim** — all four, committed to the repo:
  (1) prove the tool is in the exact artifact tested (e.g. symbol counts on the loaded
  binary); (2) named PASS lines from inside the run; (3) zero-counts counted only after
  1+2; (4) the commit hash embedded in the evidence file.
- **Un-forceable conditions get decomposed proofs.** When a timing race or environment
  can't be forced in a test, split into (a) a behavioral assertion that gates what CAN be
  forced + (b) a unit test proving the harm exists without the fix. Document the
  decomposition.
- **The whole-range gate at phase end checks CROSS-ITEM concerns only:** integration
  between items, scope creep across the range, sanitizer/CI artifacts + provenance, docs
  coherence. It does NOT re-litigate items that passed per-item review — that duplication
  is pure cost.
- **Pipelines never self-review, never publish externally** (docs sites, upstream PRs,
  announcements), **never render performance verdicts from unfit hardware.** Those are
  orchestrator/human actions.

## 10. Production-quality bars (what "done" means)

- **Automated verification from day one** — format + lint warnings-as-errors + tests —
  but the WHERE is a security decision, not a default. Hosted CI on a public repo is an
  attack surface (PR-triggered execution, third-party actions, runner-token scopes,
  secret exfiltration); enabling it is a human gate, and the default answer can
  legitimately be NO. A local enforced pre-commit hook + the pipeline's own
  review/sanitizer gates can BE the verification story. If CI is enabled, model the config
  on hardened precedents in the same ecosystem, not a generic template. Either way: if
  lint is red on the base, fix the backlog BEFORE building on it (a red base silently
  trains everyone to ignore the gate).
- **No papering over.** Suppressions (`#[allow]`, `noqa`, `eslint-disable`,
  `type: ignore`), sleeps in tests, retries masking flakiness, broad exception catches —
  each needs an inline justification and is a review finding by default. An unexplained
  suppression is the comment-instead-of-enforcement anti-pattern in miniature.
- **Unsafe/dangerous constructs are documented per-site and the documentation is
  ENFORCED** (deny undocumented unsafe at the linter — several hundred documented sites
  beat one "we're careful" paragraph).
- **The repo is self-describing:** README + a contract document for the observable
  surface + working examples. Every item that changes the surface updates them in the SAME
  commit-series (anti-rot is a review check, not a cleanup phase).
- **Honest claims:** docs state what the system does NOT do (delivery guarantees,
  saturation ceilings, platform limits) as prominently as features. Hostile review finds
  overclaims; cheaper never to write them.
- **Human readability is a review dimension:** a competent newcomer should follow any file
  top-to-bottom.

**Comment discipline** — write for the reader who has ONLY the artifact. Two audiences:
`///` `//!` are PUBLISHED API docs (render on docs.rs / hover; write for a stranger);
`//` are notes to the next maintainer of *this* code. Neither may reference anything
outside the published artifact. Cite durable PUBLIC authority by stable id (spec URL +
anchor, RFC number, in-repo symbol/test) — **never** an internal doc, ticket, codename,
work-item id, or roadmap phase.

```
✗  // see RALPH_DESIGN.md "Normative basis"; Phase A1b
✓  // Per RFC 6125 §6.4.1, match SAN before CN. CN fallback kept only for
   //   the legacy path exercised by tests/tls_legacy.rs.
```

If a comment was proxying for real authority ("see INTERNAL_DOC.md"), replace it with the
authority or inline the rationale. Never fabricate a URL/anchor; if you can't ground it,
state the rule in prose and drop the link. Scale detail by **surprise**, not word count —
"more detail" means more spec/domain RATIONALE, not more internal breadcrumbs.

## 11. Operating agents economically

Token cost ≈ model price × tool calls × transcript length: **every tool call re-processes
the whole transcript.** (The default cache TTL is 5 minutes; a 1-hour TTL exists and helps
stable cross-agent prefixes — opt in explicitly and confirm it's live via
`cache_read_input_tokens`, since the default has silently regressed before. Caching
reduces the re-processing bill but does not remove it.) Hence:

- **Long jobs (builds, suites): ORCHESTRATOR-ONLY detach; sub-agents run foreground.**
  This is the one place the two roles diverge, and conflating them is what strands
  sub-agents (§8): the orchestrator IS the parent loop, so a marker-poll loop wakes it
  fine; a sub-agent is not, so the identical pattern hangs it forever. Keep the two
  straight.
  - *Orchestrator* may detach and poll a FILE marker boundedly, reading the log ONCE
    with tail/grep (never cat whole logs; never poll the process list):

    ```
    $ (cargo test --all > /tmp/A1-suite.log 2>&1; echo done > /tmp/A1-suite.marker) &
    $ for i in $(seq 1 60); do [ -f /tmp/A1-suite.marker ] && break; sleep 10; done
    $ tail -5 /tmp/A1-suite.log        # read once, bounded
    ```
  - *Sub-agents* do NOT use the pattern above. One foreground command with a timeout,
    or redirect-and-read-immediately in the same synchronous step (§8). The `&` and the
    `sleep`-poll loop are exactly the shape that hangs them.

- **Tool-call budgets in every brief** (~80 mechanical / ~150 subtle): exceeding it means
  write a handoff brief with exact state and STOP. A grinding agent at call 120 pays
  full-transcript price per call for diminishing returns.
- **Split long work into sequential short agents** with handoff briefs, not one marathon
  context.
- **Never re-wake a finished agent** — even a no-op costs a full transcript read.
- **Progress markers** (`/tmp/<item>-progress.log`, one line per phase) give the
  orchestrator visibility without transcript reads.
- **Briefs carry the env gotchas that have actually burned agents** (self-matching pgrep
  patterns, tmpfs scratch dirs, quoting traps, stale build caches). One paragraph of
  gotchas is cheaper than one wedged agent.

## 12. Verify-before-trust (the orchestrator's own discipline)

- Check agent claims against source/output — including *explanations* (agents have
  fabricated nonexistent reports and code constructs in explanations) and "environmental
  failure" classifications (require a re-run). Verification runs against the
  **committed evidence artifact**, not a re-reading of the sources — §1 bars the
  orchestrator from the sources, and a claim with no surviving artifact is VOID
  (relaunch, don't accept). The escape hatch is deliberate and narrow: when a specific
  claim looks wrong, the orchestrator may read the exact source it rests on to settle
  that claim. Spot-checking one suspect claim is not a licence to re-derive the
  findings.
- Read actual test output, never just counters or STATUS files. Sanity-check every
  verdict: does the wall-clock support the prescribed work? Does the evidence artifact
  exist and match?
- **A completion notification is not a quiescence guarantee.** An agent that ended its
  turn with detached jobs pending can RESUME after you've dispatched its reviewer — two
  agents then share one checkout and the reviewer's evidence is contaminated (observed: a
  reviewer's live mutation raced the implementer's late suite runs). Before dispatching
  any agent onto a shared tree, confirm the previous agent's processes are dead on every
  shared host (pgrep its build/test/port footprint), not just that its report arrived. If
  contamination is found after a verdict: don't void wholesale — accept the reasoning,
  re-execute the DISPOSITIVE evidence clean-room, close on that.
- **Never accept a verdict relayed second-hand** (an implementer reporting "the review
  passed"). Verdicts come from the reviewer's own artifact, period.
- The orchestrator errs too (e.g. an over-broad delete glob) — fix immediately and
  visibly; the same evidence standards apply.
- Keep a **handoff document** as the single source of truth: current state first, then
  rules, backlog, gotchas, where things live. Condense periodically; archive closed
  narrative. Sessions die; the handoff survives.

## 13. Human touchpoints (automate nothing past these)

1. **Design-doc review** — answers the decision points. The human reads the DP section
   alone; the body is backstopped by the doc review agent's PASS (§2), which the doc
   needs before it reaches the human at all.
2. **Plan review** — approves decomposition, invariants, model/cost plan.
3. **STOP-AND-ASK escalations** — premise corrections, irreversible actions, mid-phase
   scope changes. Exception of record: newly discovered issues that need NO redesign
   and NO human decision may be triaged fix-now by the orchestrator (§7 duty 3)
   without a human touchpoint; anything needing a redesign or a judgment call still
   escalates to the human.
4. **Phase-gate results** — human sees the whole-range verdict + cost actuals before the
   next phase launches.
5. **External publication** — anything leaving the repo is human-triggered.

## 14. Anti-pattern catalog (each one happened)

*Reference table — kept dense on purpose. This is the institutional memory; each row is a
distinct lesson, and you read it by lookup, not top-to-bottom.*

| Anti-pattern | What it looked like | The rule it produced |
|---|---|---|
| Hollow PASS | Reviewer vanished; only a "PASS" line stamped impossibly early | File-backed incremental reports; evidence-less verdicts are VOID |
| Narrated evidence | "Mutation verified by construction"; tests that passed with the fix reverted | Executed runs only, both halves pasted; reviewers re-execute |
| Conditional gate erosion | "STOP if unsure" → the agent was never unsure | Topic-anchored unconditional gates |
| Fabricated explanation | Agent cited a sanitizer report and a code construct that didn't exist | Verify explanations against source, not just results |
| Checked-item amnesia | Scope added to an already-checked item; never executed | Mid-stream additions are NEW unchecked items |
| End-of-loop review blindness | Multi-item review missed per-item FAILs | Per-item pipeline for risky items |
| Test duplicating the bug | Unit test encoded the same off-by-one as the code; "CLOSED" was false | Reviewer checks tests against ground truth, not against the code |
| Comment-as-guard-rail | Invariants asserted in comments; violated silently later | Code-enforced invariants (lints, build checks, tests) |
| Unfit-host verdicts | Timing gates "passed" on a noisy dev box | Designated bench host; everything else renders no timing verdicts |
| Stale-artifact tests | Suite green against last week's binary | Freshness asserts in harnesses; rebuild + mtime/symbol check when mutating |
| Stale-artifact "bugs" | A feature looked broken; three elaborate platform hypotheses were built — the deployed binary simply predated the feature | **Artifact freshness is CHECK #1 in any feature-absent investigation** (strings/nm/mtime vs land time). Cheapest check, checked last, every time |
| Daemon-mode blindness | Chaos suite only ran foreground; a crash-respawn bug hid in daemon mode | Test the deployment modes you ship, not the ones convenient to script |
| Lost guard rail | A convenience config silently removed a loud failure protecting a cache invariant | When removing friction, ask what it was enforcing; re-enforce in code |
| Internal-vocab comment | Public `///` docs cited `RALPH_*.md`/"Normative basis"; ~235 `Phase X` tags meant nothing to an external reader; one cited the wrong spec anchor | Comments self-contained for the artifact's reader; cite durable PUBLIC authority only; enforce broken-link deny + a vocab grep guard |
| Sub-agent waits on an async signal | A sub-agent backgrounded a run / spawned a Monitor and stalled "waiting for the monitor to signal the run finished" — the notification goes to the parent loop, never back to it | Notifications reach the orchestrator, never the sub-agent that started the work; inside a sub-agent no `&`/Monitor/timer/spawned-task wait is survivable. Brief line + **mechanism** explanation (§8); detach pattern is orchestrator-only (§11) |
| Directory-scoped completeness grep | A "repo-wide" vocab cleanup took **six** rounds — each FAIL traced to the grep being scoped to `src/ tests/` (missed `build/`, `proto/`, configs) with a token set that kept growing | A completeness sweep greps the **whole tracked tree** with the **full token set enumerated up front**; the reviewer re-runs the grep AND challenges every classification; watch identifiers, filenames, and load-bearing strings. "Looks done" ≠ "grep over everything is empty" |

---

## Appendix: canonical item brief (filled)

A filled brief teaches the skeleton better than the skeleton does. This one is the unit
the implement agent receives.

```
IMPLEMENT agent — item A1 of PLAN-tls-redirect.
Repo: ngx-otel   Branch: feat/A1-hostname   Base: 9f3c1a2
Model: sonnet   Effort: medium   (assigned in PLAN §A1, approved at the plan gate)
READ FIRST: PLAN §A1, INVARIANTS block, RESEARCH F-3.

## Scope (A1 only)
1. Enforce SAN-before-CN hostname match in src/tls.rs::verify_peer_name.
2. Add tests/tls_name_match.rs: SAN hit, CN fallback, mismatch reject.
Do NOT touch: redirect logic (A2), the FFI bindings (frozen), wire bytes.

## Invariants (binding — pasted)
- fail closed, never silent fallback
- dependency `boring` is frozen — work within existing bindings
- no new code in the master process

## Verify (executed)
- cargo test tls_name_match     (linux-glibc, linux-musl)
- cargo clippy -- -D warnings

## Mutation
Force `if false { reject }` in the mismatch branch → tls_name_match::mismatch
must FAIL; restore → PASS. Paste both runs to evidence/A1-mutation.log.

## Hard rules
- Verify F-3 by symbol before coding; if SSL_set1_host is absent at the
  pinned version → STOP-AND-ASK with a re-spec.
- Search before writing any new function/type/helper: reuse an exact fit;
  elevate a near fit only within scope (own commit); shared-surface
  generalization → NOTE as proposed item. Report the outcome.
- STOP if scope reaches wire bytes, the public API, or deletes files.
- Budget ~80 tool calls. Over budget → handoff brief + STOP.
- Foreground commands only — no backgrounded shell runs, no monitor/timer
  tasks, no waiting on notifications (they never reach sub-agents). Long
  suites: one foreground command, generous timeout, chunk per binary;
  existing logs are read off disk, never re-awaited.
- Progress: one line per phase to /tmp/A1-progress.log.
- Comment discipline (§10) on any new/edited comment.

## Return (DATA, not prose) — then the §7 exit checklist, pasted and checked.
```

Canonical return (what comes back):

```
item=A1   base=9f3c1a2   head=b71e4d0
shipped-vs-spec: 1✓ 2✓  (no deviations)
tests: tls_name_match {san_hit, cn_fallback, mismatch_reject} — 3 new
evidence: evidence/A1-mutation.log (commit b71e4d0); suite logs linked
premise: F-3 confirmed — SSL_set1_host present at src/ssl.rs:1442
reuse: none found — new helper verify_peer_name (searched verify_*, *_hostname)
STOP items: none
out-of-scope: CN fallback has no IP-SAN test — propose new item A1b
model=sonnet  effort=medium  tool-calls=37  approx-tokens=210k
exit: [x] unpushed  [x] checkouts  [x] evidence@tip  [x] trees clean  [x] cost line
```

The **review brief mirrors it**: range, file-backed report (VERDICT last), checks 1–6
(scope / premise / mutation re-execute + one of its own / test quality / deep audit /
suites + clean trees), budget, return format.
