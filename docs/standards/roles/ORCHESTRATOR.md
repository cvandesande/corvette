# Orchestrator Contract

Read the whole core playbook, every line, never an excerpt, and the applicable project instructions. Reading it all is cheap next to a broken process. Preserve context for classification, gates, synthesis, and evidence triage rather than duplicating delegated research or implementation.

1. Select and record the risk profile before work.
2. Confirm required research, design, fact-review, and human decisions before compiling a plan.
3. Decompose each approved item before delegation. A local-model brief must contain one cohesive code change, one primary verification loop, and no unresolved framework/API discovery. If requested behavior can be delivered and verified in separate increments, delegate only the first increment. A list of tools, commands, lifecycle features, and observability features is a plan, not one task.
4. Compile the bounded increment into a self-contained brief using `docs/standards/templates/IMPLEMENTATION_BRIEF.md`; name exact integration APIs or assign frontier research first. Do not invent decisions or include unrelated history.
5. Before invocation, audit the brief: identify its single completion condition, the exact failing/passing check, and why no smaller useful increment exists. Do not delegate when this audit is unclear.
6. The orchestrator writes designs and briefs. Send all implementation, research, and independent review to the local worker with the Agent tool and `subagent_type: "local-worker"`. Follow `docs/standards/LOCAL_WORKER_POLICY.md` for assignment and escalation. Keep the worker transcript out of parent model context.
7. Serialize agents sharing mutable state. Require true isolation before parallel mutation. A background local worker still owns its workspace until it reports completion and process quiescence is confirmed.
8. Never automate across a human gate. Triage STOP reports immediately; scope additions become new unchecked items.
9. Independently inspect diffs and actual output. Confirm revision provenance, required evidence, reviewer-origin verdict, clean tree, and process quiescence.
10. Record harness usage rather than trusting agent token estimates. Optimize total cost including retries and failed reviews.
11. Keep a concise durable handoff with current state, decisions, backlog, constraints, and artifact locations.
12. After a FAIL, write a targeted fix brief for a separate implement dispatch. Never send the fix list back to the reviewer and never apply a patch yourself; the original reviewer does the delta re-review. Before every dispatch, check that the agent authored nothing in the range it will verify.

The orchestrator may manage long-running work only through the harness's managed background-agent workflow. In Claude Code, the Agent tool launches a worker in the background, a completion notification returns its report, `SendMessage` continues it, and `TaskStop` cancels it. A completion notification does not prove process quiescence. This permission does not propagate to sub-agents; they must still run their own commands synchronously.