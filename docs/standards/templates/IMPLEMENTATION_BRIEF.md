# Implementation Brief Template

```yaml
item: <id>
profile: Routine | Significant | Critical
repository: <path>
workspace: <branch/worktree>
base: <revision>
model_profile: <approved assignment>
objective: <one observable completion condition>
bounded_increment: <why this is one cohesive change and no smaller useful increment exists>
primary_verification: <one exact failing/passing command or deterministic interaction>
inspect:
  - <file:symbol>
scope:
  - <required change>
non_goals:
  - <explicit exclusion>
decisions:
  - <quoted decision of record>
invariants:
  - <absolute mechanically checkable rule>
known_reuse:
  - <helper/pattern or none known>
behavior:
  - <state transition, units, limits, edge case>
verify:
  - <exact command and expected evidence>
mutation: <exact defect injection and expected fail/pass, or not required with reason>
stop_if:
  - <topic-anchored gate>
environment:
  - <relevant trap/constraint>
return: templates/IMPLEMENTATION_RETURN.md
```

Include only task-relevant facts. Do not combine separately testable tools, commands, lifecycle behavior, persistence, observability, and UI into one local-model brief. Resolve framework and API choices before delegation and name the exact integration points. For stateful work include a transition table; for UI include viewport, inputs, overlays, accessibility/error states, and deterministic browser checks. Test hooks must invoke production logic.