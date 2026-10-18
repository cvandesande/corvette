# Local Worker Policy

The local worker does the implementation, research, and independent review for a bounded, approved brief. Frontier models write the designs and briefs and orchestrate.

## Where the worker is defined

The agent definition and the model profile belong to the machine, not to the project: `~/.claude/agents/local-worker.md` and `~/.claude/local-worker/PROFILE.md`. A project without a local worker deletes this file and the worker lines in `AGENTS.md`.

## Invocation

- Invoke the worker through the Agent tool with `subagent_type: "local-worker"`.
- The worker has no memory of the parent conversation and cannot ask questions. The brief must contain every fact, path, decision, check, and STOP condition the task needs.
- The Agent tool starts the worker in the background. Completion arrives as a notification. Do not poll.
- Do not read the worker transcript into parent context. Use the worker's final report.
- Cancel a running worker with `TaskStop`. Continue a finished worker with `SendMessage`, only when its context is still useful.
- Callers cannot select a reasoning level or a server option. There is one fixed configuration.
- A denied tool call is a STOP. The worker reports the denial and never reaches the same effect by another route. A worker that worked around a denial is a finding, whatever the outcome.

## Concurrency

- Run no more than 2 workers at once.
- Workers that share a working tree or a build directory run one at a time.
- A finished worker's completion message does not prove its processes have stopped. Check before sharing the tree.

## Assignment

The split is by kind of work, not by difficulty or risk profile.

| Work | Assigned to |
|---|---|
| Design, brief writing, orchestration | Frontier model |
| Implementation, at any risk profile | Local worker |
| Research | Local worker |
| Independent review | Local worker, as a separate invocation |
| Item escalated after repeated failure | Frontier model |

Each brief states one completion condition. For implementation it is one exact pass/fail check; if no such check can be written, the design or the brief is not finished.

For research, every load-bearing claim has a source and anything not established is marked `UNVERIFIED`.

For review, use a fresh invocation that does not see the implementer's report and that executes every required check. A review never produces the fix: after a FAIL the fix is a separate implementer dispatch, and the original reviewer does the delta re-review.

## Escalation

- **Attempt:** one worker run against one brief and one check.
- **Failure signature:** the failing check plus its assertion or error text. Two failures are the same only when the failure signature is the same.

Escalate an item to a frontier model when the same failure signature occurs on two consecutive attempts, or after three attempts on the item, whichever comes first. A different failure resets the repeat count but still counts toward the three attempts.

Do not count as an attempt: a failure caused by a flaw in the brief (return to planning and fix the brief); a worker `STOP` report, which is a gate and not a failure; an environmental failure, such as the server being down or a suite timing out because the server is busy.

Record attempts, failure signatures, and the outcome for each item.
