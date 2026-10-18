# Code Style for Human Review

Optimize for the reviewer, debugger, and future contributor who did not write the code. Minimize the facts they must hold at once. Project conventions and automated formatters win where they differ from this guide.

Two principles govern the rest:

1. **Reduce surprise.** Prefer boring, local, consistent code. Spend cleverness only for measured value, and explain the non-obvious reason.
2. **Make claims executable.** Types, assertions, lints, and tests are stronger than comments that ask a reviewer to trust an invariant.

Detailed rationale, examples, field validation, and sources are in `HUMAN_REVIEW_STYLE_REFERENCE.md`; they are lookup material, not routine prompt context.

## 1. Discovery and local consistency

Match the file and project before applying a personal preference. Existing naming, error, ownership, abstraction, and test patterns are part of the local review contract unless they violate a safety requirement.

Search before adding or changing a symbol. When project-code-intelligence or another discovery system injects tool-selection instructions, follow that doctrine rather than duplicating it here. This guide requires the outcome, not a specific tool:

- inspect nearby code and project-wide equivalents before adding a helper, type, dependency, or pattern;
- inspect callers and tests before removing, renaming, or changing a signature;
- treat index/search results as evidence, not a verdict, and verify relevant live source before editing.

Do not repeat injected discovery instructions in task briefs or role prompts. Record only the task-specific search result.

## 2. Naming

- Name length grows with scope and distance from the definition. Short loop-local names are fine; fields, globals, and public APIs carry the full concept.
- Use one word for one concept project-wide. Different words should signal real differences.
- Put units, coordinate frames, trust levels, and other non-typechecked distinctions in a type or, as a fallback, the name.
- Predicates read as positive assertions: `is_empty`, `has_capacity`, `contains`.
- Functions are verbs. Query-shaped names do not hide mutation; commands reveal effects.
- Avoid information-free nouns such as `data`, `info`, `item`, `manager`, `processor`, `util`, `helper`, `temp`, and numbered results unless they are established domain terms.
- Avoid visually confusable names in one scope.
- Contract-encoding conventions are useful only when documented centrally, consistent, and mechanically searchable.
- Keep mechanical renames separate from behavioral changes.

## 3. Functions and control flow

A function has one purpose that can be stated in one sentence at one level of abstraction. Length alone is not a defect: long linear code can review better than a maze of shallow wrappers. Split or restructure when live branch conditions, state, or concern changes exceed what a reader can hold locally.

- Extract code for reuse, a separable contract, or isolated testing of tricky logic—not to meet a line count.
- Prefer a few deep functions over call-through wrappers whose bodies add no abstraction.
- Keep parameter lists small. Group related options; avoid bare booleans and adjacent same-type values that can be swapped.
- Put guard clauses first and keep the normal path left-aligned.
- Do not use `else` after a terminating branch.
- Use exhaustive `match`/`switch` for closed one-value dispatch. Enumerate variants you own; handle open external sets explicitly.
- Keep conditions short and positive. Name a compound condition when the name adds intent.
- Keep nesting near two or three levels. Flatten with guards, condition inversion, loop splitting, or a genuinely named extraction.
- Queries compute; commands mutate. Combined operations reveal both in their names.
- Impossible states assert or become unrepresentable. Do not silently return from corruption.
- Loop exits remain visible; iteration variables and collections do not change unexpectedly.

## 4. State, types, and contracts

- Use the smallest scope, declare near first use, and prefer immutable bindings.
- One variable has one meaning. Do not reuse a buffer, count, index, or result for a different concept.
- Keep assignment close to use.
- Avoid action at a distance. Prefer explicit parameters and returns over mutable globals, out-parameters, and hidden “call A before B” protocols. Document unavoidable coupling at both ends.
- Make illegal states unrepresentable with enums, validated boundary types, and newtypes for values that must not mix.
- Public APIs state purpose, preconditions, errors, panics, thread safety, ownership, and relevant complexity at the definition.
- Functions that own tasks, threads, locks, or cross-component protocol state need the same contract even when private: what starts, what outlives what, ordering rules, and termination.

## 5. Errors

- Every error is handled, propagated, or explicitly discarded with a local reason. Broad empty catches and wildcard error arms block review.
- Add operation and operand context once at the boundary that knows them. Log once at the top-level handler; do not log and rethrow at every layer.
- Programmer errors assert; expected environmental failures return actionable errors.
- Error text states operation, operand, expected versus actual, and a repair hint when one exists.
- Best-effort behavior is a declared module/function policy, not scattered swallowing. The policy states what may degrade and preserves enough signal to diagnose missing output.
- Do not widen catches, add silent fallbacks, or convert impossible states into defensive hedges merely to make tests pass.

## 6. Comments and documentation

A comment states information the code cannot express. It explains why, invariant, authority, external constraint, soundness, lock/memory ordering, magic constant origin, non-obvious algorithm, regex/bit operation, or deliberately atypical behavior.

Delete:

- mechanism narration and obvious restatement;
- diff/reviewer narration such as “now correctly handles”;
- changelog history, attribution, commented-out code, and small section banners;
- descriptions of absent or removed behavior unless a history-blind reader would reasonably expect the alternative;
- unverifiable links or internal planning vocabulary.

Doc comments are caller contracts, not implementation tours. Their first sentence states what the caller gets; add only applicable preconditions, failures, ownership, concurrency, complexity, and examples.

Guide comments may divide a long linear function at a higher level than the code. Comment density is an outcome: safety-, protocol-, and FFI-heavy code needs more rationale than a plain transform.

Comments must remain self-contained for the artifact’s reader. Do not put plan items, workstreams, decision IDs, style-rule IDs, roadmap phases, or private document names in comments, identifiers, filenames, tests, logs, or errors. Replace them with the underlying public specification, in-repository symbol/test, or plain-language rationale. A code/comment mismatch is a correctness defect.

## 7. Tests as review artifacts

Reviewers read tests as executable claims.

- Name a test for the property and condition, not the method or sequence number.
- Keep setup, action, and assertion visible. Prefer local repetition over helper layers that hide inputs or oracles.
- Use table-driven tests when cases share one shape.
- Show that a new regression test fails with the target defect and passes after restoration when the pipeline profile requires mutation evidence.
- Assert observable behavior and public contracts, not private call order or incidental state.
- Cover the input class, not only the reported example. Probe a nearby boundary or alternate member when practical.
- Never weaken an oracle, broaden tolerance, delete a test, or encode the production algorithm again merely to obtain green output.

## 8. Change hygiene

The diff is the unit of review.

- Keep one review unit near 200–400 changed lines when practical. Split larger work by coherent intent, not arbitrary file slices.
- One commit has one intent. Separate formatting, moves, generated updates, and renames from behavioral changes when that separation makes the semantic diff clearer.
- Commit messages state why the old behavior was wrong and why the change is appropriate.
- New dependencies require explicit license, maintenance, transitive-surface, and alternatives review.
- Use the project-pinned formatter. Do not spend reviewer attention on manual whitespace.
- Mechanize every objective rule that can become a formatter, lint, type check, build check, grep, or test.
- Keep related types, implementations, contracts, and tests close, and expose the smallest useful interface.
- A file follows the language’s idiomatic narrative order and makes its purpose apparent before internals.

## 9. Author and reviewer sweep

Before handoff, challenge the change for:

1. a fix that handles only the example rather than the input class;
2. mismatch with nearby naming, errors, ownership, or abstraction level;
3. reinvented helpers or missed callers;
4. reviewer-directed or negative-space comments;
5. defensive hedge code hiding an unverified invariant;
6. broadened catches, wildcard dispatch, silent fallback, or weakened checks;
7. fabricated APIs, flags, links, or external-contract assumptions;
8. drive-by formatting, imports, moves, or renames hiding logic;
9. process vocabulary leaked into the artifact;
10. tests that duplicate implementation or never demonstrated failure;
11. stale generated, built, indexed, or deployed artifacts;
12. behavior tested only in a convenient mode rather than the modes shipped.

Report concrete findings by file and line or symbol, failure mode, impact, and reproduction. Style is valuable because it makes correctness checkable; it never replaces the checking.