# Independent Review Report Template

```markdown
# Review <item>

- range: `<base>..<head>`
- profile: Routine | Significant | Critical
- workspace/isolation: <details>
- reviewer model/effort: <harness values>

## Scope and invariants
<diff and sweep evidence>

## Premise and contracts
<source/symbol evidence>

## Tests and mutation
<exact commands, output, production-path assessment, reviewer mutation if required>

## Integration, documentation, and tree state
<relevant checks and provenance>

## Findings
- `file:line|symbol` — blocking | optional — <failure, impact, reproduction, required correction>

## Evidence inventory
<artifact paths and tested revision>

PASS | FAIL
```

For Critical review, append each completed check to the file immediately and keep the verdict as the final line. PASS requires all prescribed execution.