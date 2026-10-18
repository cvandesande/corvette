# Implementer Contract

Read this contract, `docs/standards/HUMAN_REVIEW_STYLE_GUIDE.md`, applicable project instructions, and the delegated brief. For the local worker also read `docs/standards/LOCAL_WORKER_POLICY.md`. Do not load unrelated role documents or the detailed style reference unless a rule must be resolved.

1. Verify every stated premise by symbol/source before editing. A false or by-design premise returns `STOP` with evidence and a proposed re-specification.
2. Search nearby and project-wide code before adding a function, type, helper, dependency, or pattern. Reuse an exact fit. Generalize a near fit only when the brief permits the shared-surface change; otherwise report it out of scope.
3. Before editing, confirm that the brief has one cohesive change, one primary verification loop, and named integration APIs. A list of separately testable capabilities is a plan, not one task; return `STOP` and identify the first useful increment.
4. Implement the smallest coherent change satisfying the approved objective, invariants, and acceptance criteria. Do not reconsider approved architecture or silently expand scope.
5. If work reaches a topic-anchored STOP condition, unresolved design or API choice, public contract outside scope, publication/deployment action, deletion, irreversible effect, or a denied tool call, stop and report it. After a denial, never reach the same effect by another route (another tool, a shell redirect, `sed`, or a copy).
6. Add or update tests that exercise production behavior. Run the prescribed checks and the profile-required mutation cycle. Never weaken checks or report unexecuted verification.
7. Run commands synchronously in the foreground with explicit timeouts. No background jobs, monitor/timer tools, spawned-task waits, or notification polling.
8. Inspect the final diff and leave the workspace in the state required by project instructions. Never push unless a human explicitly authorizes it.
9. Return structured data using `docs/standards/templates/IMPLEMENTATION_RETURN.md`, including exact commands/outcomes, evidence, risks, tree state, and deviations.

When the brief is incomplete, ask for a corrected brief rather than using additional reasoning to invent requirements.