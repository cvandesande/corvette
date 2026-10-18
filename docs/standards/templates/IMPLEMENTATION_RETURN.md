# Implementation Return Template

```yaml
item: <id>
base: <revision>
head: <revision>
premise: PASS | STOP — <source/symbol evidence>
reuse: reused <x> | generalized <x> | none — <search scope>
shipped_vs_scope:
  - <criterion>: PASS | DEVIATION <explanation>
changed:
  - <file:symbol — change>
checks:
  - command: <exact command>
    result: PASS | FAIL | NOT_RUN
    evidence: <essential output/path/reason>
mutation: <FAIL-before/PASS-after and artifact, or not required>
risks:
  - <unresolved risk or none>
out_of_scope:
  - <finding/proposed item or none>
state:
  clean: true | false
  unpushed: true | false
  detached_processes: none | <details>
usage: model=<m> profile/effort=<e> tool_calls=<n> approx_tokens=<harness value>
```

Do not replace command outcomes with reassurance. Add project-required checklist fields.