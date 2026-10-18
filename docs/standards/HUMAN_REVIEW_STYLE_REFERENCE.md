# Code Style for Human Review

A language-agnostic style guide whose single optimization target is the human who did not
write the code: the reviewer approving it, the debugger reading it at 3am, and the
contributor extending it later. Examples are in Rust, Go, C, and Python.

Motivating failure mode: code written on the shortest path to "functional" — by an AI or
a rushed human — passes its tests and then fails all three readers at once. This guide is
the counter-pressure.

Scope note: where a project has its own committed conventions, those win. This document
is the general case.

---

## 0. The cost model

Every rule below is justified against one metric: **working-memory load per line
reviewed** — the number of facts a reader must hold in their head to verify the line in
front of them is correct.

- A reviewer approves when they can build *local* confidence: each unit checkable
  against a stated contract using only what is on screen.
- A debugger works backwards from a wrong output; every hidden state change, reused
  variable, or swallowed error is a place the trail goes cold.
- A contributor extends by pattern-matching; every inconsistency is a fork in the road
  with no signpost.

Two derived principles used throughout:

1. **Surprise is the unit of review cost.** Boring code reviews fast. Cleverness is
   spent only where it buys measured performance or removes a whole class of bugs, and
   it is paid for with a comment.
2. **Verification beats trust.** Prefer constructs a compiler, type system, or assertion
   checks over constructs a reviewer must take on faith. A comment asserting an
   invariant is worth less than code enforcing it.

---

## 1. Naming

### N1. Name length scales with the distance to the definition

The further from its declaration a name is used, the more the name must carry.

- Loop index visible for 3 lines: `i` is correct; `loop_counter_index` is noise.
- Function-local used across 40 lines: a real word — `retries`, `deadline`.
- Struct field, global, or public API: the full concept — `max_inflight_requests`.

```go
// Good: short name, short scope (Go stdlib idiom)
for i, r := range routes {
    if r.Match(path) { return i }
}

// Bad: long scope, information-free name
type Server struct {
    d  map[string]int   // what is d? reviewer must find every use to know
}
```

### N2. One concept, one word — everywhere

`fetch`, `get`, `retrieve`, `load` must not coexist for the same operation. Pick one per
concept, project-wide. The reviewer must be able to assume that a different word means a
different thing (`get` = cheap accessor, `fetch` = network, `load` = disk — fine, if the
distinction is real and consistent).

### N3. Names carry what the type system doesn't

Encode units, coordinate frames, and trust boundaries either in the type (preferred) or
in the name (fallback). Never in neither.

```rust
// Best: the type makes misuse uncompilable
fn connect(timeout: Duration) -> Result<Conn, ConnectError>

// Acceptable when a newtype is not worth it
uint32_t timeout_ms;        /* C: unit in the name */
html_unsafe = render(user_input)   # Python: trust level in the name
```

### N4. Predicates read as assertions, stated positively

`is_empty`, `has_capacity`, `contains(x)`. Never `not_ready`, `disabled`, or any name a
reader must negate twice (`if !not_ready`).

### N5. Banned generic nouns

`data`, `info`, `item`, `object`, `manager`, `handler`, `processor`, `util`, `helper`,
`temp`, `result2`, `do_stuff`. Each forces the reviewer to read the body to learn what
the name should have said. (Exception: `handler` where it is the domain term, e.g. an
HTTP handler.)

### N6. Functions are verbs; queries and commands are distinguishable by name

A name shaped like a question (`is_valid`, `len`, `peek`) must not mutate. A mutation is
named as an action (`push`, `invalidate`, `drain`). Violating this breaks the reader's
ability to skim call sites for side effects — the single most useful skim there is.

### N7. No two names one edit apart in the same scope

`addr_len` / `addrs_len`, `req` / `res` in a proxy hop, `s1` / `s2`. These survive review
and cause the bug. Rename one until they are visually distinct or restructure so only
one exists.

### N8. Renames ship as their own commit

A rename mixed into a logic change makes the logic diff unreadable. Mechanical change,
separate commit (see §10).

### N9. Name conventions can encode contracts — make them grep-able

A project-wide naming convention can carry a behavioral guarantee, checkable by grep:
Redis's `...OrReply` suffix means "on error, the client reply has already been sent";
ripgrep's `..._candidate` means "takes pre-normalized input", `..._into` means "writes
into a caller-supplied buffer"; Go's `MustParse` means "panics on failure". One such
convention replaces a doc sentence at hundreds of call sites — but only if it is
documented once, centrally, and never violated.

---

## 2. Functions and abstraction

### F1. One job, statable in one sentence

The doc comment's first line is that sentence. Needing "and" is the signal to split —
*unless* the sequence is itself the job (a parser, a protocol state machine, an
initialization order). "Does one thing" means one *level of concern*, not one line of
code.

### F2. Length is not the metric; branching depth is

Long **linear** code is fine and often correct: a 150-line function that performs steps
1–9 in order, with guide comments at each step, reviews faster than 9 five-line
functions that force the reviewer to hop and rebuild the sequence mentally. High-quality
C codebases (SQLite, Redis, the kernel) are full of long linear functions.

Long **branchy** code is not fine. The heuristic that survives contact with real code:

> A reader must be able to hold every *live* branch condition in their head at any point
> in the body. Two nested conditions: fine. Four: restructure.

### F3. Extract only when the piece has a name and a contract

Extraction is justified by (a) reuse, (b) a genuinely separable concern with a clean
interface, or (c) enabling isolated testing of tricky logic. Extraction is **not**
justified by line count. Over-factored code ("lasagna") replaces one long readable
function with a call-graph scavenger hunt, and each hop evicts the reader's context.
Corollary (Ousterhout): prefer few deep functions over many shallow ones — a function
whose body is barely longer than its call site is negative value.

### F4. Parameters: at most 3–4; no bare booleans; no same-type neighbors

- More than ~4 parameters: group into a struct/options object whose field names document
  each value at the call site.
- `f(true, false)` is unreviewable. Use enums or named/keyword arguments:

```python
# Bad
resize(img, 100, 200, True, False)

# Good
resize(img, width=100, height=200, keep_aspect=True, antialias=False)
```

```rust
// Rust/Go/C equivalent: a two-variant enum beats a bool
fn sort(items: &mut [T], order: Order)   // Order::Ascending, not `true`
```

- Two adjacent parameters of the same type (`copy(a, b)` — which is dest?) get either
  distinct types, a struct, or an argument order matching a project-wide convention
  stated in the docs (e.g. C's `memcpy(dst, src)`).

### F5. Guard clauses first; the happy path stays left-aligned

Validate and bail early. The main logic reads top-to-bottom at minimal indentation, and
every early exit is visible in the left margin ("line of sight" — Go idiom).

```go
// Good
func (s *Server) handle(r *Request) error {
    if r == nil            { return errNilRequest }
    if !s.ready.Load()     { return errNotReady }

    // happy path, indent level 1, to the end
    ...
}
```

### F6. Command–query separation

A function either computes an answer or performs an effect. Combined forms
(`get_or_create`, `pop`) exist but are named to reveal the effect, and are the
exception.

### F7. One level of abstraction per body

A body that mixes `send_summary_email(user)` with raw socket writes forces the reader
to gear-shift. Either everything at the policy level or everything at the mechanism
level.

---

## 3. Control flow

### C1. Nesting depth ≤ 2–3; beyond that, restructure

The Linux kernel enforces this mechanically with 8-space tabs: "if you need more than
3 levels of indentation, you're screwed anyway, and should fix your program."
Flatteners, in order of preference: guard clauses (F5), extracting the inner block as a
named function (only per F3), inverting the condition, splitting the loop.

### C2. No `else` after a terminating branch

```rust
// Bad                              // Good
if done {                           if done {
    return Ok(n);                       return Ok(n);
} else {                            }
    retry();                        retry();
}
```

The `else` adds an indentation level and implies a symmetry that does not exist.

### C3. `match`/`switch` over `if/else` chains when dispatching on one value

An `if/else if` ladder testing the same discriminant hides two things a `match` makes
checkable: that the arms are mutually exclusive, and that they are exhaustive.
Compiler-checked exhaustiveness is a gift to the reviewer — a new enum variant becomes
a compile error at every match site instead of a silently-taken default branch.

- On enums **you own**, avoid the `_` wildcard arm; enumerate. The wildcard converts
  future missing-case bugs from compile errors into runtime surprises.
- On open sets (error codes from a foreign API, user input), the default arm is
  mandatory and must *do* something (log, error), never silently continue.
- Match arms stay short; an arm growing past a few lines becomes a function call.

```rust
// Reviewer verifies coverage at a glance; compiler verifies it forever.
match outcome {
    Outcome::Delivered      => stats.sent += 1,
    Outcome::Retryable(err) => self.backoff(err),
    Outcome::Fatal(err)     => return Err(err),
}
```

### C4. Conditions are short, positive, and named when compound

Three-clause boolean expressions get a name — a local or a predicate function — so the
condition states intent and the definition states mechanics:

```c
/* Bad */
if (r->hdr_len > 0 && !r->chunked && r->clen >= 0 && !r->upgraded) { ... }

/* Good */
int has_plain_body = r->hdr_len > 0 && !r->chunked
                  && r->clen >= 0 && !r->upgraded;
if (has_plain_body) { ... }
```

### C5. "Can't happen" is an assertion, not an empty branch

An impossible state gets `assert` / `unreachable!()` / `panic("corrupt state: ...")` —
executable documentation that also catches the day it stops being impossible. Silent
empty branches and defensive `return`s for impossible inputs hide corruption and make
the reviewer wonder which states are actually reachable. (SQLite's assertion density is
a large part of why its long functions stay reviewable.)

### C6. Loops: one entry, few visible exits, stable loop variables

`break`/`continue` are fine and preferred over flag variables — but every exit must be
findable by scanning the left margin. Mutating the iteration variable inside the body,
or the collection being iterated, is banned outside of algorithms whose whole point is
that mutation (and those get a comment).

---

## 4. State and data flow

### S1. Smallest scope, latest declaration, immutable by default

Declare at first use, not at the top of the function. Prefer `const`/immutable bindings
everywhere the language allows: a reviewer who sees an immutable binding never has to
scan forward for reassignment. Every mutable variable is a standing obligation on the
reader.

### S2. One variable, one meaning

Reusing `buf` for the request and then the response, or `n` for a count and then an
index, saves one declaration and costs every future reader a re-derivation. Cheap fix:
more variables, tighter scopes.

### S3. Minimize assignment-to-use distance

A value computed on line 10 and used on line 90 forces an 80-line memory span. Compute
where used, or hoist the use.

### S4. No action at a distance

Functions read their parameters and produce their return value. Module-level mutable
state, out-parameters, and "sets a field that a later call reads" protocols each turn a
local review into a global one. Where unavoidable (C, embedded, FFI), the coupling is
documented at both ends.

---

## 5. Types and contracts

### T1. Make illegal states unrepresentable

A type that cannot express the invalid case is an invariant the reviewer checks **once**
(at the type definition) instead of at every use site.

```rust
// Bad: three fields, four invalid combinations to check everywhere
struct Conn { connected: bool, addr: Option<SocketAddr>, err: Option<Error> }

// Good: exactly the legal states
enum Conn { Idle, Connected(SocketAddr), Failed(Error) }
```

Same principle, dynamic languages: parse at the boundary, pass validated objects inward
("parse, don't validate"). Inner code never re-checks.

### T2. Newtypes for values that must not mix

`UserId(u64)` vs `OrderId(u64)`; `Meters(f64)` vs `Seconds(f64)`. Cheap in Rust/Go,
worth it whenever two same-representation values would be catastrophic to swap.

### T3. Public API contracts are written down at the definition

What it does (one sentence), preconditions, error conditions, panic conditions,
thread-safety, ownership of passed/returned resources. This is the text the reviewer
checks the body against and the caller reads instead of the body. Format per §7.

---

## 6. Error handling

### E1. No swallowed errors, ever

Every error is handled, propagated, or *explicitly and visibly* discarded with a reason:

```go
// The only acceptable ignore: named, justified, in place.
_ = f.Close() // read-only file; close cannot fail meaningfully here
```

```python
# Bad — the reviewer cannot tell design from laziness
try:
    cleanup()
except Exception:
    pass
```

Bare `except:`/`catch (...)`/`_ => {}` around broad code is a review-block: it converts
unknown bugs into silence.

### E2. Context is added once, at the boundary where it exists

Wrap errors with the operation and operand at the point that knows them
(`fmt.Errorf("open config %q: %w", path, err)`, `.with_context(...)`). Do **not**
log-and-rethrow at every layer — one error, one log line, at the top-level handler.
Repeated logging makes real incident logs unreadable.

### E3. Programmer error asserts; environmental error returns

A violated invariant (caller passed garbage, state machine in impossible state) is a
bug: `assert`/`panic` loudly, close to the cause. An expected-world failure (file
missing, connection reset, malformed input from outside) returns an error the caller
can act on. Confusing the two either crashes on user input or limps past corruption.

### E4. Error text is written for the 3am debugger

Operation + operand + expectation vs. reality:
`"parse listen directive: port 99999 out of range 1-65535"` — greppable, actionable,
no code-reading required. Not `"invalid value"`.

Errors that reach end users additionally carry a repair hint where one exists. ripgrep:
`"unopened alternate group; missing '{' (maybe escape '}' with '[}]'?)"` — the message
does the support ticket's job.

### E5. A degrade-and-continue policy is declared once, at the module boundary

Best-effort code — collectors that skip a failing source, parsers that drop malformed
rows, probes that treat absence as "not available" — is legitimate, but only as a
*stated policy*, never as an emergent property of scattered `ok()`s. The module (or
function) doc says what degrades, and every silent path must be covered by that
declaration; a swallowed error the declaration doesn't cover is an E1 violation, not
policy. Within a declared policy, prefer one log line at the failure site over total
silence — the debugger's question is "why is this datum missing," and a policy that
cannot answer it has degraded the debugger too.

---

## 7. Comments and documentation

The section AI-written code gets wrong most often, in both directions.

### D1. A comment is a claim the code cannot make

Test for every comment: *could a competent reader recover this from the code alone?*
If yes, delete it. If no, it stays — and is load-bearing.

**Always comment (this list is "not long enough" territory):**
- **Why this, not the obvious alternative** — the rejected approach and the reason.
- **Invariants and preconditions** the code relies on but cannot enforce.
- **Every `unsafe` block, lock-ordering rule, memory-ordering choice** — with the
  argument for soundness, not just a wave.
- **Magic constants** — where the number comes from (spec section, measurement, RFC).
- **External constraints** — protocol quirks, upstream bugs worked around (with link),
  compatibility requirements.
- **Non-obvious algorithm choices** and their complexity.
- **Regexes and bit manipulation** — a plain-language restatement.
- **Deliberately empty branches** — why nothing happens here.

**Never comment (the "too long" territory):**
- Narration of mechanism: `// increment the counter` above `n += 1`. Restating code
  doubles the text and creates a second copy that rots.
- Diff narration / reviewer-directed text: `// Fixed the bug by checking null first`,
  `// New improved implementation`. Comments address the next reader of the *file*,
  not the reviewer of the *change*. This class is a hallmark of AI-generated patches
  and is deleted on sight.
- Changelogs and attribution: version control holds these.
- Commented-out code: delete it; git remembers.
- Section banners around five lines.

### D2. Doc comments are contracts, mechanism-free

First line: one complete sentence, what the caller gets. Then, only as applicable:
preconditions, error/panic conditions, thread-safety, ownership, complexity, one example
when the signature alone misleads.

```rust
/// Drains up to `budget` records from the ring, blocking never.
///
/// Returns the number drained. Safe to call from the signal-free
/// exporter context only; worker-side callers must use `try_push`.
```

The body is then reviewed *against* this text. A doc comment that describes the
implementation ("loops over the ring and copies...") is useless the day the
implementation changes and dangerous the day after.

### D3. Guide comments make long linear functions reviewable

In a legitimately long function (F2), one-line signposts divide the phases:

```c
/* Phase 2: negotiate — offer both encodings, prefer OTAP. */
```

This is the one sanctioned form of "what" comment: it operates at a higher altitude
than the code, so it does not duplicate it. (Taxonomy per antirez: guide comments lower
cognitive load even when they add no information.)

### D4. Comment density is an output, not a target

FFI-heavy, lock-heavy, or spec-implementing code legitimately runs 30–45% comment
lines; a plain data transform runs near zero. Any rule of the form "aim for N%"
produces either narration-padding or deleted invariants.

### D5. A wrong comment is worse than no comment

Comments live adjacent to what they describe, are updated in the same diff that changes
the code, and reviewers treat a code/comment mismatch as a correctness bug — because one
of the two is wrong and the reviewer doesn't know which.

### D6. Contract comments trigger on coupling, not visibility

Any function that owns concurrent tasks, threads, or cross-task shared state gets a
D2 contract comment regardless of visibility: what it spawns, what outlives it, and
how everything terminates. Concurrency is where the reader most needs a stated
invariant, and it is exactly the code where "private, so no contract required" lets
the largest function in the file go unstated. Same rule for a protocol split across
components (A sends X, B must react with Y): the contract is written at both ends, or
the invariant is enforced locally instead (S4).

### D7. Comments describe the code that exists, not the code that doesn't

A comment explaining what the code does *not* do, no longer does, or was decided against
is history narration, not documentation — it is residue of the author's process (most
often a pivot) and means nothing to a reader who wasn't there. Deleted on sight:
"we don't X here", "this no longer Xs", "note that X was removed", and any multiline
block whose subject is absent behavior.

The one sanctioned form: the implementation is **atypical** — a competent reader with no
knowledge of the project's history would expect X and the code deliberately does
otherwise. Then one comment states the expected alternative and why it was rejected
(this is D1's "why this, not the obvious alternative"). The test: *does the surprise
survive for a reader who never saw the previous version or the abandoned plan?* If the
surprise exists only relative to project history, the comment goes; if a stranger would
stumble without it, it stays — and is kept as short as the surprise allows.

### D8. Process vocabulary never enters the artifact — review-block

Internal work-item and rule identifiers — plan items ("A1", "W2G"), workstreams
("WS-C"), decision points ("DP-B"), style-rule IDs ("D7", "E1"), roadmap phases,
internal doc names — mean nothing to a reader who has only the code. They are
orchestration vocabulary, and the artifact's readers were never in the orchestration.
Banned in comments, identifiers, filenames, log/error strings, and test names alike.
**Any occurrence fails review — no severity judgment involved.**

The fix is substitution, not deletion: replace the ID with the authority or rationale
it was standing in for (a spec/RFC citation, an in-repo symbol or test, or the reason
in plain prose). If it was standing in for nothing, delete it.

```rust
// ✗  // Per D7 / plan item C3: no fallback here.
// ✓  // Deliberately no fallback: a silent retry here would mask the
//    //   config-generation mismatch that tests/swap_survival.rs exists to catch.
```

---

## 8. Tests as review artifacts

Reviewers read tests first: they are the executable statement of what the change claims.
Style rules for tests differ from production code on purpose.

### X1. The test name states the property, not the method under test

`test_drain_stops_at_budget_when_ring_has_more` — readable as a spec line in the test
runner output. Not `test_drain_2`.

### X2. Repetition over abstraction (DAMP, not DRY)

A test should be verifiable by reading it alone, top to bottom: setup, action,
assertion. Helper pyramids and clever fixtures that hide the input force the reviewer
to review the test framework instead of the claim. Duplication across tests is
acceptable where it keeps each test self-contained.

### X3. Table-driven where cases share a shape

The Go-stdlib pattern: one loop, a slice of `{input, want}` rows. The reviewer audits a
data table (easy) instead of n copies of logic (error-prone), and adding a case is a
one-line diff.

### X4. A test that has never failed is unverified

New tests must be shown to fail when the code is broken (revert the fix, mutate the
constant). A test that mirrors the implementation's bug passes forever and certifies
nothing. "Verified by construction" is not evidence; an executed failing run is.

### X5. Assert on behavior, not implementation

Tests that assert private call sequences or internal state break on every refactor and
train the team to update tests without reading them. Assert on the contract (T3).

---

## 9. AI-authorship failure modes — a review checklist

The recurring defects of shortest-path-to-green code, whoever wrote it. Each maps to a
rule above; listed here as things a reviewer probes and an author self-checks.

1. **Handles the example, not the class.** The fix makes the failing test pass and no
   other member of the input class. Probe: "what nearby input still breaks?" (X4: the
   new test must fail on the old code — and one more test from the same class.)
2. **Idiom mismatch with the surrounding file.** New code uses a different error
   pattern, naming style, or abstraction level than the code 20 lines above. Rule:
   *match the file you are in*; consistency outranks the author's preferred style.
3. **Reviewer-directed comments.** `// now correctly handles the edge case` (D1).
4. **Reinvented helpers.** A fresh `clamp`/`retry`/`parse_duration` when the codebase
   has one. Authors search before writing; reviewers grep suspicious utilities.
5. **Hedge code.** Defensive null-checks and fallbacks for states the invariants
   exclude — written because the author didn't establish the invariant. Every hedge
   both hides real corruption (C5) and tells the reviewer the author is unsure which
   states are reachable. Understand, then assert.
6. **Over-broad catch to go green.** A widened `except`/`match _` that converts a
   failing case into silence (E1, C3).
7. **Weakened oracles.** Assertions loosened (`assert x == 5` → `assert x >= 0`),
   tolerances widened, tests deleted — the diff to the *tests* is reviewed with more
   suspicion than the diff to the code.
8. **Fabricated interfaces.** Calls to plausible-but-nonexistent APIs or flags.
   Unfamiliar identifiers in a diff get looked up, not assumed.
9. **Drive-by churn.** Reformatting, import reordering, and renames mixed into the
   logic change, burying the semantic diff (§10).
10. **Negative-space narration.** Multiline comments describing what the code does
    *not* do — typically residue of a pivot or an abandoned approach ("we no longer
    X", "unlike the previous design..."). Deleted unless the implementation is
    genuinely atypical to a history-blind reader (D7); then one short why-comment,
    not a narrative.
11. **Process-vocabulary leakage.** Plan/rule/workstream IDs ("A1", "D7", "WS-C",
    "Phase 2") or internal doc names in comments, identifiers, strings, or test
    names. Automatic review FAIL (D8); replace with the underlying authority or
    plain-prose rationale.

---

## 10. Change hygiene — the diff is the unit of review

1. **≤ ~400 changed lines per review unit.** Defect detection collapses beyond that
   (the Cisco/SmartBear review study puts the effective range at 200–400 LOC, under an
   hour per session). A 1,500-line PR does not get one review; it gets one skim and an
   approval.
2. **One intent per commit.** "Refactor X" and "fix Y" are two commits even when Y
   motivated X. Mechanical changes (rename, format, move) ship separately from
   behavioral ones — the reviewer skims one and scrutinizes the other.
3. **The commit message states why.** The diff shows what changed; only the message can
   say what was wrong before and why this is the fix. "Fix race" is not a message;
   "drain loop re-read head after releasing the lock, allowing a second drain to
   interleave — hold lock across read" is.
4. **No new dependency without its own justification.** A dependency is code you review
   once and trust forever after: license, maintenance, transitive surface. In C/Rust
   ecosystems targeting review-heavy upstreams, prefer the stdlib and small vendored
   pieces over framework deps.
5. **Zero manual formatting decisions.** `gofmt` / `rustfmt` / `black` / `clang-format`
   with repo-pinned config. Formatting is CI's job; a human review comment about
   whitespace is a process failure. Line length ~80–100 — not aesthetics: side-by-side
   diff panes.
6. **Mechanize every rule that can be mechanized.** Any style rule a machine can check
   is removed from the reviewer's checklist permanently: curl's `checksrc.pl` plus a
   ~90-entry banned-function list runs in every debug build; ripgrep compiles with
   `#![deny(missing_docs)]`; Go ships `gofmt` and `vet` as culture. Reviewer attention
   is the scarcest resource in the whole pipeline — spend it only on what tools cannot
   see.

---

## 11. File and module layout

- **Narrative order.** A file reads top-to-bottom: module doc comment (one paragraph:
  what lives here and why it exists), public API, then internals — or the language's
  idiomatic order, applied consistently.
- **The module doc answers "should I be reading this file?"** in its first line.
- **Interfaces small, modules deep.** Expose the minimum (`pub(crate)`, `static`,
  `_prefix`); a small surface over substantial functionality is what makes a module
  cheap to review from the outside (Ousterhout's "deep modules").
- **Related things adjacent.** A type, its impl/methods, and its tests live together or
  in the language's conventional split. A reviewer should never need more than two
  files open to verify one change.

---

## 12. Quick reference card

| Area | Rule of thumb |
|---|---|
| Names | Length ∝ scope distance; one concept one word; positive predicates |
| Functions | One sentence of purpose; long-linear OK, deep-branchy never; ≤4 params, no bare bools |
| Branching | Nesting ≤ 2–3; guards first; `match` for one-value dispatch; no `_` on owned enums |
| State | Immutable by default; smallest scope; one variable one meaning |
| Types | Illegal states unrepresentable; newtype unmixable values |
| Errors | Never swallowed; context once; assert bugs, return environment failures; degrade policies declared at the boundary |
| Comments | Why/invariant/constraint = mandatory; what/narration/diff-talk = deleted; absent-behavior narration deleted unless the code is atypical to a history-blind reader (D7); process IDs (plan items, rule IDs, workstreams) anywhere in the artifact = automatic FAIL (D8); concurrency owners get contracts regardless of visibility |
| Tests | Name = property; DAMP; table-driven; shown to fail |
| Diffs | ≤400 lines; one intent; mechanics separate from semantics; message says why |
| Formatting | Autoformatter, pinned config, zero human involvement |

---

## 13. Field validation against high-quality codebases

A source-level survey of seven projects with strong quality reputations: **SQLite**
(src/complete.c, src/random.c, src/btree.c head, testing docs), **Go standard library**
(sort/search.go, strings/builder.go, the Code Review Comments wiki), **Redis**
(src/t_string.c, antirez's comments essay), **curl** (docs/internals/CODE_STYLE.md,
lib/cookie.c), **Linux kernel** (Documentation/process/coding-style.rst), **ripgrep**
(crates/globset/src/lib.rs), **requests** (src/requests/sessions.py). All snippets below
are verbatim from fetched source.

### 13.1 Rules this survey confirms

| Rule | Field evidence |
|---|---|
| N1 scope-proportional names | Universal. Go wiki states it verbatim: "The further from its declaration that a name is used, the more descriptive the name must be" — "Prefer `c` to `lineCount`. Prefer `i` to `sliceIndex`." Kernel §4: loop counter "should probably be called `i`. Calling it `loop_counter` is non-productive"; globals "need to have descriptive names" (`count_active_users()`, not `cntusr()`). Practiced in SQLite, Redis, curl, ripgrep. |
| F2 long-linear OK, deep-branchy never | The strongest cross-project pattern: **bimodal function length**. SQLite pairs a 3-line `sqlite3_complete` wrapper with a ~195-line single-`while`+`switch` state machine; Redis pairs 3-line `incrCommand` with the ~142-line `setGenericCommand` engine; Go has 1-line `SearchInts`. The kernel codifies the exception: functions "should fit on one or two screenfuls," but "one long (but simple) case-statement... it's OK to have a longer function." Kernel adds a bound this guide adopts: local-variable count "shouldn't exceed 5-10, or you're doing something wrong." |
| F5/C1/C2 guards, shallow nesting | Kernel §1: ">3 levels of indentation, you're screwed anyway." Go wiki: "keep the normal code path at a minimal indentation." Redis exits early after every `addReply...`; SQLite and the kernel use single-exit `goto` with *named* labels ("Avoid GW-BASIC names like `err1:`" — `out_free_buffer:`). |
| D1/D2 comments = why + contract, on the boundary | Doctrine in all three C projects and Go. The dominant shape everywhere: **a full contract comment on the function boundary, near-bare bodies**, with body comments reserved for invariants and surprises. SQLite's `sqlite3_complete` carries a ~90-line header specifying all 8 states and the return bit-layout — the body is checkable against it line by line. Go's search.go is ~55% comment, almost all API doc with runnable examples, plus branch-level invariants: `// Invariant: f(i-1) == false, f(j) == true.` Why-comments cite the source of the constraint: Go `// see #25907, #47276`, curl `/* not using checkprefix() because matching should be case-sensitive */`, requests "kept to avoid breaking backwards compatibility". |
| D4 density is an output | SQLite runs 27–43% comment lines; Redis ~19%; ripgrep bodies near 0% (contract mass lives in `///` docs instead). All are top-tier. No project targets a ratio. |
| C5 assert the impossible | SQLite: 6,754 `assert()` statements in the core, plus `ALWAYS()`/`NEVER()` macros that compile to constants in coverage builds so defensive branches don't block 100% branch coverage — assertion as first-class design. curl uses `DEBUGASSERT`; Go panics on misuse with package-prefixed messages: `panic("strings: illegal use of non-zero Builder copied by value")`. |
| E2–E4 context at the failure site | SQLite `SQLITE_NOMEM_BKPT` variants capture `__LINE__`; `corruptPageError()` builds messages with page number and filename. ripgrep's `Error{glob, kind}` carries the offending pattern. requests: `InvalidSchema(f"No connection adapters were found for {url!r}")`. Redis names the config knob in the message: "string exceeds maximum allowed size (proto-max-bulk-len)". |
| E3 assert bugs / return environment | Go draws the line exactly here: `error` returns for the world, `panic` for API misuse. requests likewise: domain exceptions for the network, bare `ValueError` for programmer error. |
| X3 tables over logic | SQLite uses table-driven style in *production* code: the `trans[8][8]` state-transition table with row/column label comments. curl's style doc: "Being clear and unambiguous beats being clever and saving two lines of code." |
| §10.6 mechanize | curl: `checksrc.pl` + banned-function list (~90 entries: `malloc`, `sprintf`, `strcpy`, `atoi`, even `snprintf`) enforced in default debug builds. ripgrep: `#![deny(missing_docs)]`. Style survives by build failure, not reviewer vigilance. |
| N9 grep-able contract naming | Redis `...OrReply`; ripgrep `_candidate`/`_into`; SQLite's internal `sqlite3CamelCase` vs public `sqlite3_lowercase` split. |

### 13.2 Where the field disagrees — with itself and with this guide

Honest divergences between top-tier projects; a guide that pretends these don't exist
loses credibility with the engineers it's for.

1. **Semantic prefixes.** The kernel: Hungarian notation "is asinine." SQLite: pervasive
   `zSql` (NUL-terminated string), `pBuf` (pointer), `nParen` (count), `iRoot` (index),
   `eLockType` (enum). These positions are less opposed than they read: SQLite's
   prefixes encode *referent semantics C's type system won't check* (count vs index vs
   handle), not the compiler type — which is rule N3's fallback branch. In strongly
   typed languages the prefix belongs in the type; in C, SQLite's scheme is a rational
   N3 implementation and the kernel's scorn targets the type-restating variant
   (`dwSize`). Verdict: N3 stands; pick per language.
2. **Comments inside function bodies.** Kernel §8: "try to avoid putting comments inside
   a function body: if the function is so complex that you need to separately comment
   parts of it, you should probably go back to chapter 6 [and split it]." SQLite:
   trailing comment on nearly every local, invariant comments throughout long bodies.
   Both positions are coherent given their function-length choice — short functions
   externalize explanation to the boundary; long linear engines internalize it as guide
   comments (D3). The rule pair (F2 + D3) covers both; what no project does is narrate
   mechanism.
3. **Local name length.** requests and curl use fully descriptive locals even in tight
   scopes (`merged_setting`, `cookie_domain_len`) where Go/kernel/SQLite would write
   `ms`, `dlen`. N1's *proportionality* holds everywhere (nobody writes one-letter
   globals); the constant of proportionality is a house choice. Python's culture sits
   longer; systems C sits shorter. Match the house.
4. **Bare error codes.** curl's inner layer returns naked `CURLcode` with no message —
   context is attached layers up. This trades E2's "context where it exists" for a
   no-allocation constraint in library plumbing. In constrained/hot paths that trade is
   legitimate; the context then must exist at the boundary layer, and curl's does.

### 13.3 What the survey added to this guide

Three things the first-principles draft missed, now folded in above:

- **Mechanical enforcement as a pillar, not a footnote** (§10.6) — the surveyed projects
  don't rely on reviewers remembering rules; they make violations fail the build.
- **Contract-encoding name conventions** (N9) — `OrReply` / `_candidate` / `Must...`.
- **Repair hints in user-facing errors** (E4) — ripgrep's `(maybe escape '}' with
  '[}]'?)`.

And one calibration: SQLite's 590:1 test-to-code ratio, 100% branch coverage under
MC/DC, and a fuzzer running ~10⁹ mutations/day put a ceiling on what style alone buys.
Style makes correctness *checkable*; the checking still has to happen (§8, X4).

---

## 14. Sources and further reading

- Kernighan & Pike, *The Practice of Programming* — naming, simplicity, clarity as the
  primary virtues.
- John Ousterhout, *A Philosophy of Software Design* — deep modules, comments as design,
  the case against shallow abstraction.
- Steve McConnell, *Code Complete* (2nd ed.) — the evidence-based numbers on variable
  scope, routine length, and review effectiveness.
- SmartBear / Cisco code-review study — the 200–400 LOC per review-hour ceiling.
- antirez, "Writing system software: code comments" (antirez.com/news/124) — the comment
  taxonomy §7 builds on.
- John Carmack, "In-depth: Functional programming in C++ … on inlined code" (2007
  email, published 2014) — the case for long linear functions.
- Go: *Effective Go*, wiki *Code Review Comments*, Go Proverbs.
- Linux kernel `Documentation/process/coding-style.rst`.
- SQLite, "How SQLite Is Tested" (sqlite.org/testing.html).
- Rust API Guidelines (rust-lang.github.io/api-guidelines).
- PEP 8 / PEP 20 (Zen of Python).
- Kernighan's law: "Debugging is twice as hard as writing the code in the first place.
  Therefore, if you write the code as cleverly as possible, you are, by definition, not
  smart enough to debug it."
