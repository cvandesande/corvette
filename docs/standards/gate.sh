#!/usr/bin/env bash
# gate.sh — dispatcher for the CI and pre-commit gate legs.
set -u
SELF="$(cd "$(dirname "$0")" && pwd)/$(basename "$0")"
REPO="$(git rev-parse --show-toplevel)" || exit 2
cd "$REPO" || exit 2

usage() {
  cat <<'USAGE'
usage: gate.sh <leg> [args...]
  comment         pct <file>|baseline|check <base>..<tip>|staged|selftest
  obs             baseline|deps|selftest
  d8              staged|range <a>..<b>|selftest
  selftest        dispatcher reachability check
USAGE
}

comment_measurable() {
  case "$1" in
    docs/*|evidence/*|tests/*|conformance/*|vendor/*) return 1 ;;
  esac
  case "$1" in
    *.rs|*.sh|*.yml|*.yaml|*.conf|*.toml|Dockerfile*|*/Dockerfile*) return 0 ;;
    *) return 1 ;;
  esac
}

comment_pct_of() {
  local f="$1" src="${2:-$1}"
  [ -f "$src" ] || { echo "0.0 0 0 $f"; return; }
  case "$f" in
    *.rs)
      awk 'BEGIN{c=0;n=0;inb=0;saferun=0}
        {s=$0; sub(/^[ \t]+/,"",s); t=s; sub(/[ \t]+$/,"",t)
         if(t=="///" || t=="//!" || t=="//" || t=="*") next
         if(saferun==0 && s ~ /^\/\/+[ \t]*SAFETY:/){saferun=1; next}
         if(saferun==1 && s ~ /^\/\//){saferun=2; next}
         n++
         if(inb){c++; if(index(s,"*/"))inb=0; next}
         if(s ~ /^\/\//){c++; next}
         if(s ~ /^\/\*/){c++; if(!index(substr(s,3),"*/"))inb=1; next}
         saferun=0}
        END{printf "%.1f %d %d\n", (n?100*c/n:0), c, n}' "$src" ;;
    *)
      awk 'BEGIN{c=0;n=0}
        {s=$0; sub(/^[ \t]+/,"",s); t=s; sub(/[ \t]+$/,"",t)
         if(t=="#" && !(NR==1 && s ~ /^#!/)) next
         n++
         if(s ~ /^#/ && !(NR==1 && s ~ /^#!/)) c++}
        END{printf "%.1f %d %d\n", (n?100*c/n:0), c, n}' "$src" ;;
  esac | { read -r p c n; echo "$p $c $n $f"; }
}

comment_gen() {
  git ls-files | while read -r f; do
    comment_measurable "$f" || continue
    line="$(comment_pct_of "$f")"
    read -r pct cm _ <<<"$line"
    if awk -v p="$pct" -v c="$cm" "BEGIN{exit !(p>$TARGET && c>$GRACE)}"; then
      echo "$line"
    fi
  done | sort -k4
}

comment_safety_count() {
  [ -f "$1" ] || { echo 0; return; }
  awk 'BEGIN{n=0}
    {s=$0; sub(/^[ \t]+/,"",s); if(s ~ /^\/\/+[ \t]*SAFETY:/) n++}
    END{print n+0}' "$1"
}

comment_gen_safety() {
  git ls-files | while read -r f; do
    case "$f" in *.rs) ;; *) continue ;; esac
    comment_measurable "$f" || continue
    cnt="$(comment_safety_count "$f")"
    [ "$cnt" -gt 0 ] && printf '%d\t%s\n' "$cnt" "$f"
  done | sort -k1,1nr -k2,2
}

comment_full_gen() {
  comment_gen
  echo "# SAFETY counts"
  comment_gen_safety
}

# comment_cap_of <file>: looks up <file> in docs/standards/comment-exceptions.txt.
# Prints the cap percentage (no "%") and returns 0 on a well-formed match,
# prints nothing and returns 2 when no row matches, prints "MALFORMED" and
# returns 1 when a row for <file> exists but fails to parse (fail-closed).
comment_cap_of() {
  local target="$1" exc="docs/standards/comment-exceptions.txt" line path rest capfield why capnum pipes
  [ -f "$exc" ] || { echo ""; return 2; }
  while IFS= read -r line; do
    case "$line" in ''|'#'*) continue ;; esac
    path="${line%%|*}"; path="$(printf '%s' "$path" | sed 's/^[ \t]*//;s/[ \t]*$//')"
    [ "$path" = "$target" ] || continue
    pipes="$(printf '%s' "$line" | tr -cd '|' | wc -c)"
    if [ "$pipes" -lt 2 ]; then echo "MALFORMED"; return 1; fi
    rest="${line#*|}"
    capfield="${rest%%|*}"; capfield="$(printf '%s' "$capfield" | sed 's/^[ \t]*//;s/[ \t]*$//')"
    why="${rest#*|}"; why="$(printf '%s' "$why" | sed 's/^[ \t]*//;s/[ \t]*$//')"
    case "$capfield" in *%) capnum="${capfield%\%}" ;; *) echo "MALFORMED"; return 1 ;; esac
    case "$capnum" in ''|*[!0-9.]*) echo "MALFORMED"; return 1 ;; esac
    [ -n "$why" ] || { echo "MALFORMED"; return 1; }
    echo "$capnum"; return 0
  done < "$exc"
  echo ""; return 2
}

# comment_exceptions_regen: down-only ratchet, pure function of the tree —
# when a row's file now measures below its committed cap, rewrite the cap to
# the measured value. Never raises a cap. Aborts (leaving the file
# untouched) on a malformed row, same fail-closed rule as comment_cap_of.
comment_exceptions_regen() {
  local exc="docs/standards/comment-exceptions.txt" tmp line path rest capfield why capnum pipes cur
  [ -f "$exc" ] || return 0
  tmp="$(mktemp)"
  while IFS= read -r line; do
    case "$line" in
      ''|'#'*) printf '%s\n' "$line" >> "$tmp"; continue ;;
    esac
    path="${line%%|*}"; path="$(printf '%s' "$path" | sed 's/^[ \t]*//;s/[ \t]*$//')"
    pipes="$(printf '%s' "$line" | tr -cd '|' | wc -c)"
    if [ "$pipes" -lt 2 ]; then
      echo "FAIL malformed exception row: $line" >&2; rm -f "$tmp"; return 1
    fi
    rest="${line#*|}"
    capfield="${rest%%|*}"; capfield="$(printf '%s' "$capfield" | sed 's/^[ \t]*//;s/[ \t]*$//')"
    why="${rest#*|}"; why="$(printf '%s' "$why" | sed 's/^[ \t]*//;s/[ \t]*$//')"
    case "$capfield" in *%) capnum="${capfield%\%}" ;; *) echo "FAIL malformed exception row: $line" >&2; rm -f "$tmp"; return 1 ;; esac
    case "$capnum" in ''|*[!0-9.]*) echo "FAIL malformed exception row: $line" >&2; rm -f "$tmp"; return 1 ;; esac
    { [ -n "$path" ] && [ -n "$why" ]; } || { echo "FAIL malformed exception row: $line" >&2; rm -f "$tmp"; return 1; }
    if [ -f "$path" ]; then
      cur="$(comment_pct_of "$path" | cut -d' ' -f1)"
      if awk -v c="$cur" -v cap="$capnum" 'BEGIN{exit !(c<cap)}'; then
        capfield="${cur}%"
      fi
    fi
    printf '%s | %s | %s\n' "$path" "$capfield" "$why" >> "$tmp"
  done < "$exc"
  mv "$tmp" "$exc"
}

comment_fail_help() {
  echo "rule    a touched file stays at or under ${TARGET}% comment lines (doc comments included) or ${GRACE} comment lines: prose the code can say rots"
  echo "fix     climb the comment ladder from rung 1, delete first; not a fix: --no-verify, a new or raised exception cap, trimming formatting"
  echo "        a stale table alone: bash docs/standards/gate.sh comment baseline, then commit the table; regenerating never clears a density FAIL"
  echo "see     docs/standards/STYLE.md D1a (the comment ladder) and AGENTS.md \"Comment gate\""
}

leg_comment() {
  local BASELINE="docs/standards/comment-baseline.txt" TARGET="3.0" GRACE=3
  case "${1:-}" in
    pct) comment_pct_of "${2:?file}" ;;
    baseline)
      comment_full_gen > "$BASELINE"
      comment_exceptions_regen || exit 1
      wc -l < "$BASELINE" | xargs echo "entries:" ;;
    check)
      local RANGE="${2:?range base..tip}" BASE FAIL=0 OLD BOOTSTRAP f status a b p c cap rc_cap
      BASE="${RANGE%%..*}"
      d8_resolve_range "$RANGE" || { echo "FAIL cannot resolve range '$RANGE' (base '$BASE' missing from this clone? fetch full history)"; exit 1; }
      if ! diff -u "$BASELINE" <(comment_full_gen) > /dev/null 2>&1; then
        echo "FAIL stale $BASELINE — run: bash docs/standards/gate.sh comment baseline, and commit it"
        FAIL=1
      fi
      if ! OLD="$(git show "$BASE:$BASELINE" 2>/dev/null)"; then
        OLD=""; BOOTSTRAP=1
      else
        BOOTSTRAP=0
      fi
      while IFS="$(printf '\t')" read -r status a b; do
        case "$status" in
          R100) continue ;;
          R*) f="$b" ;;
          *) f="$a" ;;
        esac
        comment_measurable "$f" || continue
        [ -f "$f" ] || continue
        read -r p c _ <<<"$(comment_pct_of "$f")"
        cap="$(comment_cap_of "$f")"; rc_cap=$?
        if [ "$rc_cap" -eq 1 ]; then
          echo "FAIL    malformed exception row for $f in docs/standards/comment-exceptions.txt (format: path | NN.N% | reason)"
          FAIL=1
          continue
        fi
        if [ "$rc_cap" -eq 0 ]; then
          if awk -v p="$p" -v cap="$cap" "BEGIN{exit !(p<=cap)}"; then
            echo "ok      $p% ($c lines) $f (cap ${cap}%)"
          elif [ "$BOOTSTRAP" -eq 1 ]; then
            echo "note    $p% ($c lines) $f — no table at range base; over-cap not evaluable; bring it under its cap before merging"
          else
            echo "FAIL    $p% ($c lines) $f — over its exception cap ${cap}% (docs/standards/comment-exceptions.txt)"
            FAIL=1
          fi
          continue
        fi
        if awk -v p="$p" -v c="$c" "BEGIN{exit !(p<=$TARGET || c<=$GRACE)}"; then
          echo "ok      $p% ($c lines) $f"
          continue
        fi
        if [ "$BOOTSTRAP" -eq 1 ]; then
          echo "note    $p% ($c lines) $f — no table at range base; over-target not evaluable; bring it to ${TARGET}% or ${GRACE} lines before merging"
        else
          echo "FAIL    $p% ($c lines) $f — touched file over ${TARGET}% target (must reach ${TARGET}% or ${GRACE} lines)"
          FAIL=1
        fi
      done < <(git diff -M --diff-filter=ACMR --name-status "$RANGE")
      if [ "$FAIL" -eq 0 ]; then echo "comment-gate: PASS"; else comment_fail_help; echo "comment-gate: FAIL"; fi
      exit $FAIL ;;
    staged)
      local FAIL=0 OLD TMP f status a b p c cap rc_cap
      if git rev-parse -q --verify MERGE_HEAD >/dev/null 2>&1; then
        echo "comment-gate: staged skipped on merge commit — deferring to the range check: run bash docs/standards/gate.sh comment check <base>..HEAD on the merged tree"
        exit 0
      fi
      OLD="$(git show "HEAD:$BASELINE" 2>/dev/null || true)"
      TMP="$(mktemp)"; trap 'rm -f "$TMP"' EXIT
      while IFS="$(printf '\t')" read -r status a b; do
        case "$status" in
          R100) continue ;;
          R*) f="$b" ;;
          *) f="$a" ;;
        esac
        comment_measurable "$f" || continue
        git show ":$f" > "$TMP" 2>/dev/null || continue
        read -r p c _ <<<"$(comment_pct_of "$f" "$TMP")"
        cap="$(comment_cap_of "$f")"; rc_cap=$?
        if [ "$rc_cap" -eq 1 ]; then
          echo "FAIL    malformed exception row for $f in docs/standards/comment-exceptions.txt (format: path | NN.N% | reason)"
          FAIL=1
          continue
        fi
        if [ "$rc_cap" -eq 0 ]; then
          if awk -v p="$p" -v cap="$cap" "BEGIN{exit !(p<=cap)}"; then
            continue
          fi
          echo "FAIL    $p% ($c lines) $f — over its exception cap ${cap}% (docs/standards/comment-exceptions.txt)"
          FAIL=1
          continue
        fi
        if awk -v p="$p" -v c="$c" "BEGIN{exit !(p<=$TARGET || c<=$GRACE)}"; then
          continue
        fi
        if [ -z "$OLD" ]; then
          echo "note    $p% ($c lines) $f — no table at HEAD; density gate not evaluable locally"
        else
          echo "FAIL    $p% ($c lines) $f — file over ${TARGET}% comment density (must reach ${TARGET}% or ${GRACE} lines)"
          FAIL=1
        fi
      done < <(git diff --cached -M --diff-filter=ACMR --name-status)
      [ "$FAIL" -eq 0 ] || { comment_fail_help; echo "comment-gate: FAIL (staged)"; }
      exit $FAIL ;;
    selftest)
      local TMP p1 c1 n1 p2 c2 n2 REPO2 OUT RC p3 c3 n3 p3b c3b n3b p4 c4 n4 p4b c4b n4b c5 n5 SEC FIRST_LINE OUT2 RC2 OUT3 RC3 OUT4 RC4 OUT5 RC5 OUT6 RC6
      TMP="$(mktemp)"; trap 'rm -f "$TMP"' EXIT
      printf '%s\n' \
        'fn f() {}' '// prose one' '// prose two' '//' '///' '   ///   ' 'fn g() {}' \
        > "$TMP"
      read -r p1 c1 n1 < <(comment_pct_of "x.rs" "$TMP" | cut -d' ' -f1-3)
      [ "$c1" = "2" ] || { echo "selftest FAIL: expected 2 comment lines, got $c1"; exit 1; }
      [ "$n1" = "4" ] || { echo "selftest FAIL: expected 4 total lines (separators excluded), got $n1"; exit 1; }
      printf '%s\n' 'fn f() {}' '// prose one' '// prose two' 'fn g() {}' > "$TMP"
      read -r p2 c2 n2 < <(comment_pct_of "x.rs" "$TMP" | cut -d' ' -f1-3)
      [ "$p1" = "$p2" ] || { echo "selftest FAIL: stripping separators changed density ($p1% -> $p2%)"; exit 1; }
      echo "selftest PASS: $c1/$n1 -> $p1% ; separators stripped -> $c2/$n2 -> $p2% (no change)"

      REPO2="$(mktemp -d)"; trap 'rm -f "$TMP"; rm -rf "$REPO2"' EXIT
      ( cd "$REPO2" && mkdir -p docs/standards && git init -q && git config user.email t@t && git config user.name t
        printf 'fn f() {}\n// a\n// b\n// c\n// d\n// e\n// f\n// g\n// h\nfn g() {}\n' > over.rs
        "$SELF" comment baseline > /dev/null
        git add over.rs docs/standards/comment-baseline.txt && git commit -q -m base
        printf 'fn f() {}\n// a\n// b\n// c\n// d\nfn g() {}\n' > over.rs
        "$SELF" comment baseline > /dev/null
        git add over.rs docs/standards/comment-baseline.txt && git commit -q -m reduced )
      OUT="$(cd "$REPO2" && "$SELF" comment check HEAD~1..HEAD 2>&1)"; RC=$?
      printf '%s\n' "$OUT" | grep -q "^FAIL.*over.rs" || { echo "selftest FAIL: expected FAIL for reduced-but-still-over-target file; got: $OUT"; exit 1; }
      [ "$RC" -ne 0 ] || { echo "selftest FAIL: expected nonzero exit for reduced-but-still-over-target file"; exit 1; }
      echo "selftest PASS: over-target file that merely decreased still FAILs"

      printf '%s\n' 'fn f() {}' '// SAFETY: reason' 'unsafe { g() }' '// prose' 'fn h() {}' > "$TMP"
      read -r p3 c3 n3 < <(comment_pct_of "x.rs" "$TMP" | cut -d' ' -f1-3)
      printf '%s\n' 'fn f() {}' 'unsafe { g() }' '// prose' 'fn h() {}' > "$TMP"
      read -r p3b c3b n3b < <(comment_pct_of "x.rs" "$TMP" | cut -d' ' -f1-3)
      if [ "$p3" != "$p3b" ] || [ "$c3" != "$c3b" ] || [ "$n3" != "$n3b" ]; then
        echo "selftest FAIL: 1-line SAFETY changed density ($c3/$n3 vs $c3b/$n3b)"; exit 1
      fi
      printf '%s\n' 'fn f() {}' '// SAFETY: reason' '// continuation' 'unsafe { g() }' '// prose' 'fn h() {}' > "$TMP"
      read -r p4 c4 n4 < <(comment_pct_of "x.rs" "$TMP" | cut -d' ' -f1-3)
      printf '%s\n' 'fn f() {}' 'unsafe { g() }' '// prose' 'fn h() {}' > "$TMP"
      read -r p4b c4b n4b < <(comment_pct_of "x.rs" "$TMP" | cut -d' ' -f1-3)
      if [ "$p4" != "$p4b" ] || [ "$c4" != "$c4b" ] || [ "$n4" != "$n4b" ]; then
        echo "selftest FAIL: 2-line SAFETY changed density ($c4/$n4 vs $c4b/$n4b)"; exit 1
      fi
      echo "selftest PASS: 1-line SAFETY -> $c3/$n3 (same as deleted); 2-line SAFETY -> $c4/$n4 (same as deleted)"

      printf '%s\n' 'fn f() {}' '// SAFETY: reason' '// continuation' '// line3' '// line4' 'unsafe { g() }' > "$TMP"
      read -r _ c5 n5 < <(comment_pct_of "x.rs" "$TMP" | cut -d' ' -f1-3)
      [ "$c5" = "2" ] || { echo "selftest FAIL: expected 2 prose comment lines (line3+line4), got $c5"; exit 1; }
      [ "$n5" = "4" ] || { echo "selftest FAIL: expected 4 total lines (opener+continuation excluded), got $n5"; exit 1; }
      echo "selftest PASS: 4-line SAFETY block -> lines 3-4 count as prose ($c5/$n5)"

      rm -rf "$REPO2"; REPO2="$(mktemp -d)"; trap 'rm -f "$TMP"; rm -rf "$REPO2"' EXIT
      ( cd "$REPO2" && mkdir -p docs/standards && git init -q && git config user.email t@t && git config user.name t
        printf 'fn f() {}\n// SAFETY: one\nunsafe { a() }\n// SAFETY: two\n// cont\nunsafe { b() }\n' > many.rs
        printf 'fn f() {}\n// SAFETY: solo\nunsafe { c() }\n' > few.rs
        git add many.rs few.rs
        "$SELF" comment baseline > /dev/null
        git add docs/standards/comment-baseline.txt && git commit -q -m base )
      SEC="$(cd "$REPO2" && awk '/^# SAFETY counts$/{f=1;next} f' docs/standards/comment-baseline.txt)"
      printf '%s\n' "$SEC" | grep -q "^2	many.rs$" || { printf '%s\n' "selftest FAIL: expected '2 many.rs' in SAFETY counts, got: $SEC"; exit 1; }
      printf '%s\n' "$SEC" | grep -q "^1	few.rs$" || { printf '%s\n' "selftest FAIL: expected '1 few.rs' in SAFETY counts, got: $SEC"; exit 1; }
      FIRST_LINE="$(printf '%s\n' "$SEC" | head -1)"
      [ "$FIRST_LINE" = "2	many.rs" ] || { echo "selftest FAIL: expected many.rs (count 2) first, got: $FIRST_LINE"; exit 1; }
      echo "selftest PASS: baseline SAFETY counts section present and descending-ordered"
      ( cd "$REPO2" && printf 'fn f() {}\n// SAFETY: solo\nunsafe { c() }\n// SAFETY: new one\nunsafe { d() }\n' > few.rs
        git add few.rs && git commit -q -m "add safety line, no rebaseline" )
      OUT2="$(cd "$REPO2" && "$SELF" comment check HEAD~1..HEAD 2>&1)"; RC2=$?
      printf '%s\n' "$OUT2" | grep -q "^FAIL stale" || { echo "selftest FAIL: expected stale-baseline FAIL after unregenerated SAFETY addition; got: $OUT2"; exit 1; }
      [ "$RC2" -ne 0 ] || { echo "selftest FAIL: expected nonzero exit for stale baseline"; exit 1; }
      echo "selftest PASS: added SAFETY line without rebaseline -> FAIL stale"

      rm -rf "$REPO2"; REPO2="$(mktemp -d)"; trap 'rm -f "$TMP"; rm -rf "$REPO2"' EXIT
      ( cd "$REPO2" && mkdir -p docs/standards && git init -q && git config user.email t@t && git config user.name t
        printf 'fn f() {}\n// a\n// b\n// c\n// d\n// e\n// f\n// g\n// h\n' > over.rs
        "$SELF" comment baseline > /dev/null
        git add over.rs docs/standards/comment-baseline.txt && git commit -q -m base
        git mv over.rs renamed.rs )
      OUT3="$(cd "$REPO2" && "$SELF" comment staged 2>&1)"; RC3=$?
      [ "$RC3" -eq 0 ] || { echo "selftest FAIL: expected exit 0 for a staged pure rename; got rc=$RC3: $OUT3"; exit 1; }
      printf '%s\n' "$OUT3" | grep -q "^FAIL" && { echo "selftest FAIL: staged pure rename must not FAIL; got: $OUT3"; exit 1; }
      echo "selftest PASS: staged pure rename of an over-target file does not FAIL"
      ( cd "$REPO2" && git mv renamed.rs renamed2.rs && printf '// i\n' >> renamed2.rs && git add renamed2.rs )
      OUT4="$(cd "$REPO2" && "$SELF" comment staged 2>&1)"; RC4=$?
      printf '%s\n' "$OUT4" | grep -q "^FAIL.*renamed2.rs" || { echo "selftest FAIL: expected FAIL for staged rename+edit of an over-target file; got rc=$RC4: $OUT4"; exit 1; }
      [ "$RC4" -ne 0 ] || { echo "selftest FAIL: expected nonzero exit for staged rename+edit"; exit 1; }
      echo "selftest PASS: staged rename+edit of an over-target file still FAILs"

      rm -rf "$REPO2"; REPO2="$(mktemp -d)"; trap 'rm -f "$TMP"; rm -rf "$REPO2"' EXIT
      ( cd "$REPO2" && mkdir -p docs/standards && git init -q && git config user.email t@t && git config user.name t
        printf 'fn f() {}\n// a\n// b\n// c\n// d\n// e\n// f\n// g\n// h\n' > over.rs
        "$SELF" comment baseline > /dev/null
        git add over.rs docs/standards/comment-baseline.txt && git commit -q -m base
        git mv over.rs renamed.rs
        "$SELF" comment baseline > /dev/null
        git add -A && git commit -q -m renamed )
      OUT5="$(cd "$REPO2" && "$SELF" comment check HEAD~1..HEAD 2>&1)"; RC5=$?
      [ "$RC5" -eq 0 ] || { echo "selftest FAIL: expected exit 0 for a committed pure rename; got rc=$RC5: $OUT5"; exit 1; }
      printf '%s\n' "$OUT5" | grep -q "^FAIL.*renamed.rs" && { echo "selftest FAIL: check-mode pure rename must not FAIL; got: $OUT5"; exit 1; }
      echo "selftest PASS: check-mode pure rename of an over-target file does not FAIL"
      ( cd "$REPO2" && git mv renamed.rs renamed2.rs && printf '// i\n' >> renamed2.rs
        "$SELF" comment baseline > /dev/null
        git add -A && git commit -q -m "renamed and edited" )
      OUT6="$(cd "$REPO2" && "$SELF" comment check HEAD~1..HEAD 2>&1)"; RC6=$?
      printf '%s\n' "$OUT6" | grep -q "^FAIL.*renamed2.rs" || { echo "selftest FAIL: expected FAIL for check-mode rename+edit of an over-target file; got rc=$RC6: $OUT6"; exit 1; }
      [ "$RC6" -ne 0 ] || { echo "selftest FAIL: expected nonzero exit for check-mode rename+edit"; exit 1; }
      echo "selftest PASS: check-mode rename+edit of an over-target file still FAILs"

      rm -rf "$REPO2"; REPO2="$(mktemp -d)"; trap 'rm -f "$TMP"; rm -rf "$REPO2"' EXIT
      ( cd "$REPO2" && mkdir -p docs/standards && git init -q && git config user.email t@t && git config user.name t
        printf 'fn f() {}\n// a\n// b\n// c\n// d\nfn g() {}\n' > capped.rs
        "$SELF" comment baseline > /dev/null
        printf '%s\n' "capped.rs | 90% | test seed row" > docs/standards/comment-exceptions.txt
        git add capped.rs docs/standards/comment-exceptions.txt docs/standards/comment-baseline.txt && git commit -q -m base
        printf 'fn f() {}\n// a\n// b\n// c\n// d\n// e\nfn g() {}\n' > capped.rs
        mv docs/standards/comment-exceptions.txt "$TMP.exc"
        "$SELF" comment baseline > /dev/null
        mv "$TMP.exc" docs/standards/comment-exceptions.txt
        git add -A && git commit -q -m edit )
      OUT7="$(cd "$REPO2" && "$SELF" comment check HEAD~1..HEAD 2>&1)"; RC7=$?
      printf '%s\n' "$OUT7" | grep -q "^ok.*capped.rs" || { echo "selftest FAIL: expected ok for under-cap exception file; got rc=$RC7: $OUT7"; exit 1; }
      [ "$RC7" -eq 0 ] || { echo "selftest FAIL: expected exit 0 for under-cap exception file; got rc=$RC7"; exit 1; }
      echo "selftest PASS: exception-file over 3% but under its cap = PASS"

      rm -rf "$REPO2"; REPO2="$(mktemp -d)"; trap 'rm -f "$TMP"; rm -rf "$REPO2"' EXIT
      ( cd "$REPO2" && mkdir -p docs/standards && git init -q && git config user.email t@t && git config user.name t
        printf 'fn f() {}\n// a\n// b\nfn g() {}\n' > capped2.rs
        printf '%s\n' "capped2.rs | 10% | test seed row" > docs/standards/comment-exceptions.txt
        "$SELF" comment baseline > /dev/null
        git add capped2.rs docs/standards/comment-exceptions.txt docs/standards/comment-baseline.txt && git commit -q -m base
        printf 'fn f() {}\n// a\n// b\n// c\n// d\n// e\n// f\nfn g() {}\n' > capped2.rs
        "$SELF" comment baseline > /dev/null
        git add -A && git commit -q -m edit )
      OUT8="$(cd "$REPO2" && "$SELF" comment check HEAD~1..HEAD 2>&1)"; RC8=$?
      printf '%s\n' "$OUT8" | grep -q "^FAIL.*capped2.rs" || { echo "selftest FAIL: expected FAIL for over-cap exception file; got rc=$RC8: $OUT8"; exit 1; }
      [ "$RC8" -ne 0 ] || { echo "selftest FAIL: expected nonzero exit for over-cap exception file"; exit 1; }
      echo "selftest PASS: exception-file over its cap = FAIL"

      rm -rf "$REPO2"; REPO2="$(mktemp -d)"; trap 'rm -f "$TMP"; rm -rf "$REPO2"' EXIT
      ( cd "$REPO2" && mkdir -p docs/standards && git init -q && git config user.email t@t && git config user.name t
        printf 'fn f() {}\n// a\n// b\n// c\n// d\n// e\n// f\n// g\n// h\nfn g() {}\n' > ratchet.rs
        printf '%s\n' "ratchet.rs | 90% | test seed row" > docs/standards/comment-exceptions.txt
        "$SELF" comment baseline > /dev/null
        git add ratchet.rs docs/standards/comment-exceptions.txt docs/standards/comment-baseline.txt && git commit -q -m base )
      CAP_BEFORE="$(cd "$REPO2" && awk -F'|' '{print $2}' docs/standards/comment-exceptions.txt | tr -d ' ')"
      [ "$CAP_BEFORE" != "90%" ] || { echo "selftest FAIL: seed cap 90% must already ratchet down to the file's measured density at first baseline"; exit 1; }
      ( cd "$REPO2" && printf 'fn f() {}\n// a\nfn g() {}\n' > ratchet.rs && "$SELF" comment baseline > /dev/null )
      CUR="$(cd "$REPO2" && "$SELF" comment pct ratchet.rs | cut -d' ' -f1)"
      CAP_AFTER="$(cd "$REPO2" && awk -F'|' '{print $2}' docs/standards/comment-exceptions.txt | tr -d ' ')"
      [ "$CAP_AFTER" = "${CUR}%" ] || { echo "selftest FAIL: expected cap ratcheted down to ${CUR}%, got $CAP_AFTER"; exit 1; }
      echo "selftest PASS: density drop re-baselines the cap down to ${CAP_AFTER} (was $CAP_BEFORE)"
      ( cd "$REPO2" && printf 'fn f() {}\n// a\n// b\n// c\n// d\n// e\n// f\n// g\n// h\nfn g() {}\n' > ratchet.rs && "$SELF" comment baseline > /dev/null )
      CAP_AFTER2="$(cd "$REPO2" && awk -F'|' '{print $2}' docs/standards/comment-exceptions.txt | tr -d ' ')"
      [ "$CAP_AFTER2" = "$CAP_AFTER" ] || { echo "selftest FAIL: cap rose mechanically ($CAP_AFTER -> $CAP_AFTER2)"; exit 1; }
      echo "selftest PASS: caps never rise mechanically ($CAP_AFTER2 held after density rose back up)"

      rm -rf "$REPO2"; REPO2="$(mktemp -d)"; trap 'rm -f "$TMP"; rm -rf "$REPO2"' EXIT
      ( cd "$REPO2" && mkdir -p docs/standards && git init -q && git config user.email t@t && git config user.name t
        printf 'fn f() {}\n// a\n// b\n// c\n// d\nfn g() {}\n' > other.rs
        printf '%s\n' "unrelated.rs | 90% | test seed row" > docs/standards/comment-exceptions.txt
        "$SELF" comment baseline > /dev/null
        git add other.rs docs/standards/comment-exceptions.txt docs/standards/comment-baseline.txt && git commit -q -m base
        printf 'fn f() {}\n// a\n// b\n// c\n// d\n// e\nfn g() {}\n' > other.rs
        "$SELF" comment baseline > /dev/null
        git add -A && git commit -q -m edit )
      OUT9="$(cd "$REPO2" && "$SELF" comment check HEAD~1..HEAD 2>&1)"; RC9=$?
      printf '%s\n' "$OUT9" | grep -q "^FAIL.*other.rs" || { echo "selftest FAIL: expected FAIL for non-listed over-target file; got rc=$RC9: $OUT9"; exit 1; }
      [ "$RC9" -ne 0 ] || { echo "selftest FAIL: expected nonzero exit for non-listed over-target file"; exit 1; }
      echo "selftest PASS: non-listed file over 3% still FAILs even when an exceptions table exists"

      rm -rf "$REPO2"; REPO2="$(mktemp -d)"; trap 'rm -f "$TMP"; rm -rf "$REPO2"' EXIT
      ( cd "$REPO2" && mkdir -p docs/standards && git init -q && git config user.email t@t && git config user.name t
        printf 'fn f() {}\n// a\nfn g() {}\n' > bad.rs
        printf '%s\n' "bad.rs | not-a-percent | missing cap field" > docs/standards/comment-exceptions.txt
        touch docs/standards/comment-baseline.txt
        git add bad.rs docs/standards/comment-exceptions.txt docs/standards/comment-baseline.txt && git commit -q -m base
        printf 'fn f() {}\n// a\n// b\nfn g() {}\n' > bad.rs
        git add bad.rs && git commit -q -m edit )
      OUT10="$(cd "$REPO2" && "$SELF" comment check HEAD~1..HEAD 2>&1)"; RC10=$?
      printf '%s\n' "$OUT10" | grep -q "^FAIL.*malformed exception row for bad.rs" || { echo "selftest FAIL: expected malformed-row FAIL for bad.rs; got rc=$RC10: $OUT10"; exit 1; }
      [ "$RC10" -ne 0 ] || { echo "selftest FAIL: expected nonzero exit for malformed exception row"; exit 1; }
      echo "selftest PASS: malformed exception row = FAIL"
      ( cd "$REPO2" && "$SELF" comment baseline > /dev/null 2>&1 ) && { echo "selftest FAIL: expected baseline regen to abort on a malformed exception row"; exit 1; }
      echo "selftest PASS: baseline regen fail-closes on a malformed exception row (table left untouched)"

      rm -rf "$REPO2"; REPO2="$(mktemp -d)"; trap 'rm -f "$TMP"; rm -rf "$REPO2"' EXIT
      ( cd "$REPO2" && mkdir -p docs/standards && git init -q && git config user.email t@t && git config user.name t
        printf 'fn f() {}\n// a\n// b\n// c\n// d\n// e\n// f\n// g\n// h\nfn g() {}\n' > over.rs
        "$SELF" comment baseline > /dev/null
        git add over.rs docs/standards/comment-baseline.txt && git commit -q -m base
        git checkout -q -b side && printf 'conflict-side\n' > conflict.rs && git add conflict.rs && git commit -q -m side
        git checkout -q main 2>/dev/null || git checkout -q master
        printf 'conflict-main\n' > conflict.rs && git add conflict.rs && git commit -q -m main-conflict
        git merge -q side -m merge 2>/dev/null
        printf 'fn f() {}\n// a\n// b\n// c\n// d\n// e\n// f\n// g\n// h\n// i\nfn g() {}\n' > over.rs
        git add over.rs conflict.rs )
      OUT11="$(cd "$REPO2" && "$SELF" comment staged 2>&1)"; RC11=$?
      [ "$RC11" -eq 0 ] || { echo "selftest FAIL: expected exit 0 for staged over-target file during a merge commit; got rc=$RC11: $OUT11"; exit 1; }
      printf '%s\n' "$OUT11" | grep -q "deferring to the range check" || { echo "selftest FAIL: expected deferral notice during a merge commit; got: $OUT11"; exit 1; }
      echo "selftest PASS: staged over-target file during a merge commit (MERGE_HEAD present) exits 0 with deferral notice"
      ( cd "$REPO2" && rm -f .git/MERGE_HEAD )
      OUT12="$(cd "$REPO2" && "$SELF" comment staged 2>&1)"; RC12=$?
      printf '%s\n' "$OUT12" | grep -q "^FAIL.*over.rs" || { echo "selftest FAIL: expected FAIL for the same staged over-target file once MERGE_HEAD is gone; got rc=$RC12: $OUT12"; exit 1; }
      [ "$RC12" -ne 0 ] || { echo "selftest FAIL: expected nonzero exit once MERGE_HEAD is gone"; exit 1; }
      echo "selftest PASS: same staged over-target file FAILs once MERGE_HEAD is absent (no behavior change on non-merge commits)"
      exit 0 ;;
    *) echo "usage: gate.sh comment pct <file> [src] | baseline | check <base>..<tip> | staged | selftest"; exit 2 ;;
  esac
}


d8_added_lines() { sed -n '/^+++/d; s/^+//p'; }
# file:line of each line d8_added_lines emits, in the same order.
d8_added_locs() { awk '/^\+\+\+ /{f=substr($0,7);next} /^\+\+\+/{next} /^@@/{split($3,a,/[+,]/);n=a[2];next} /^\+/{print f":"n;n++;next} /^ /{n++}'; }
# Rust format specs ({b:#04x}) and RFC section cites legitimately look like
# process IDs; strip both before the ID patterns run.
# "gate.sh d8" (this leg's own invocation form) reads as a D-NNN
# divergence-register ID (D_PAT) to its own pattern; strip it, same as
# the pre-fold script stripped its own name "d8-gate".
d8_strip_fmt() { sed -e 's/{[^{} ;]*}//g; s/d8-gate//g; s/gate\.sh d8\b//g; s/RFC [0-9][0-9]* §[0-9][0-9.]*//g' -e 's/[A-Za-z0-9._-]\{1,\}\/[A-Za-z0-9._-]\{1,\}#[0-9]\{1,\}//g'; }
d8_code_part()    { sed 's|//.*||'; }
d8_comment_part() { sed -e 's|^[^/]*//|//|' -e t -e 's/.*//'; }

d8_check_lines() {
  local hits
  hits=$(d8_strip_fmt | grep -nE "$2")
  [ -n "$hits" ] && [ -n "${D8_LOCS:-}" ] \
    && hits=$(printf '%s\n' "$hits" | awk 'NR==FNR{l[NR]=$0;next}{n=$0;sub(/:.*/,"",n);sub(/^[0-9]+:/,"");print l[n]": "$0}' "$D8_LOCS" -)
  if [ -n "$hits" ]; then
    echo "D8 gate FAIL ($1): process/plan identifiers found:"
    echo "$hits"
    return 1
  fi
  return 0
}

d8_fail_help() {
  echo "rule    shipped code and root docs carry no tracker, plan or rule identifiers (issue numbers, item labels, IDs like D7 or E1): they mean nothing without the plan"
  echo "fix     write what the identifier stood for: the reason, the invariant, the spec clause; not a fix: --no-verify, splitting or respelling the token, moving the text"
  echo "see     docs/standards/STYLE.md D8 and AGENTS.md \"d8-gate\""
}

d8_check_code_lines() {
  local buf rc=0
  buf=$(cat)
  printf '%s\n' "$buf" | d8_code_part | d8_check_lines "$1 (code position)" "$PAT" || rc=1
  printf '%s\n' "$buf" | d8_comment_part | d8_check_lines "$1 (comment position)" "$ID_PAT_NO_D|$ART_PAT|$PLAN_PAT" || rc=1
  return $rc
}

d8_resolve_range() {
  git rev-parse --verify --quiet "${1%%..*}^{commit}" >/dev/null \
    && git rev-parse --verify --quiet "${1##*..}^{commit}" >/dev/null
}

d8_selftest() {
  local n
  n=$(printf 'see artifacts/reviews/x.md FINDING #123\nper D-65 ruling\nthe S3b collapse per P2\nfn test_c6_rejects_empty_realm() {\n// per plan E3, [E6] and E6/E7 defer this; I-5 holds\n' \
      | d8_strip_fmt | grep -cE "$PAT")
  [ "$n" = "5" ] || { echo "d8-gate: SELF-TEST FAILED (controls not matched)"; exit 2; }
  printf '// D-105: see docs/nginx-divergences.md\n/// severity matches OSS (D-114)\n' \
    | d8_check_code_lines "selftest" >/dev/null \
    || { echo "d8-gate: SELF-TEST FAILED (comment-position register ID was flagged)"; exit 2; }
  printf 'assert!(ok, "must log at EMERG (D-114), not WARN");\n' \
    | d8_check_code_lines "selftest" >/dev/null \
    && { echo "d8-gate: SELF-TEST FAILED (string-position register ID not caught)"; exit 2; }
  printf '// per plan E3 this holds\n' \
    | d8_check_code_lines "selftest" >/dev/null \
    && { echo "d8-gate: SELF-TEST FAILED (comment-position plan ID not caught)"; exit 2; }
  printf '// re-arm after rename (DQ1)\nlet x = 1; // DQ-4 dual enforcement\n' \
    | d8_check_code_lines "selftest" >/dev/null \
    && { echo "d8-gate: SELF-TEST FAILED (design-question DQ ID not caught)"; exit 2; }
  n=$(printf 'required by the 838-3 exclude union\n' | d8_strip_fmt | grep -cE "$PAT" || true)
  [ "$n" = "1" ] || { echo "d8-gate: SELF-TEST FAILED (bare NNN-N item ID not caught)"; exit 2; }
  n=$(printf 'ports 21470-21489 reserved\nsee lines 159-236 for the band\n' | d8_strip_fmt | grep -cE "$PAT" || true)
  [ "$n" = "0" ] || { echo "d8-gate: SELF-TEST FAILED (false positive on numeric range)"; exit 2; }
  printf 'fn response_retry_condition_d78(req: &Req) -> bool {\n' \
    | d8_check_code_lines "selftest" >/dev/null \
    && { echo "d8-gate: SELF-TEST FAILED (lowercase dNN suffix after underscore not caught)"; exit 2; }
  printf 'const M3_BOOL_ROWS: &[Spec] = &[];\nfn m3_bool_row(name: &str) {\n' \
    | d8_check_code_lines "selftest" >/dev/null \
    && { echo "d8-gate: SELF-TEST FAILED (milestone identifier not caught)"; exit 2; }
  n=$(printf 'let comm3_buf = 1;\nlet mm31_x = 2;\n' | d8_strip_fmt | grep -cE "$PAT" || true)
  [ "$n" = "0" ] || { echo "d8-gate: SELF-TEST FAILED (false positive on fused-alpha mNN)"; exit 2; }
  printf 'fn d79_unconditional_return_bypasses_rate_limits() {\n' \
    | d8_check_code_lines "selftest" >/dev/null \
    && { echo "d8-gate: SELF-TEST FAILED (lowercase dNN fused prefix not caught)"; exit 2; }
  n=$(printf 'fn f() { println!("{b:#04x}"); }\n// Apache MD5 crypt, per RFC 2617\n// error E0463: cannot find crate for core\n' | d8_strip_fmt | grep -cE "$PAT" || true)
  [ "$n" = "0" ] || { echo "d8-gate: SELF-TEST FAILED (false positive on format spec or acronym)"; exit 2; }
  # shellcheck disable=SC2016 # $backend is literal nginx-variable test input
  n=$(printf 'server { listen unix:/tmp/n650-1b-test.sock; proxy_pass $backend; }\n' | d8_strip_fmt | grep -cE "$PAT" || true)
  [ "$n" = "1" ] || { echo "d8-gate: SELF-TEST FAILED (braced inline-config literal plan ID not caught)"; exit 2; }
  n=$(printf '/// treat that identically to the pre-J1 behaviour\n' | d8_strip_fmt | grep -cE "$PAT" || true)
  [ "$n" = "1" ] || { echo "d8-gate: SELF-TEST FAILED (hyphen-prefixed letter+digit ID not caught)"; exit 2; }
  n=$(printf 'negotiated h2 for this listener; T1 remains reserved\n' | d8_strip_fmt | grep -cE "$PAT" || true)
  [ "$n" = "0" ] || { echo "d8-gate: SELF-TEST FAILED (false positive on bare letter+digit vocabulary)"; exit 2; }
  n=$(printf 'here just picks the ordinary non-H2 fallthrough\n' | d8_strip_fmt | grep -cE "$PAT" || true)
  [ "$n" = "0" ] || { echo "d8-gate: SELF-TEST FAILED (false positive on non-H2 protocol vocabulary)"; exit 2; }
  n=$(printf 'fn test_o2_record_request_2xx() {\n' | d8_strip_fmt | grep -cE "$PAT" || true)
  [ "$n" = "1" ] || { echo "d8-gate: SELF-TEST FAILED (test_[abefot]N_ letter extension not caught)"; exit 2; }
  n=$(printf 'fn test_h2_preface_detection() {\n' | d8_strip_fmt | grep -cE "$PAT" || true)
  [ "$n" = "0" ] || { echo "d8-gate: SELF-TEST FAILED (false positive on kept test_h1_/test_h2_ protocol names)"; exit 2; }
  n=$(printf 'the mutation-3 catching leg\nplan mutation 4: an otherwise valid leaf\n' | d8_strip_fmt | grep -cE "$PAT" || true)
  [ "$n" = "2" ] || { echo "d8-gate: SELF-TEST FAILED (mutation-N / mutation N plan vocabulary not caught)"; exit 2; }
  n=$(printf 'Catches mutant (1) — a dropped guard\ncatches mutant 3 below\nsame standing as M1'\''s own\n' | d8_strip_fmt | grep -cE "$PAT" || true)
  [ "$n" = "3" ] || { echo "d8-gate: SELF-TEST FAILED (mutant (N) / M<N>'s vocabulary not caught)"; exit 2; }
  n=$(printf 'runs fine on an M1 Max laptop\nnegotiated over HTTP/2 with an M2 backend\n' | d8_strip_fmt | grep -cE "$PAT" || true)
  [ "$n" = "0" ] || { echo "d8-gate: SELF-TEST FAILED (false positive on ordinary M<N> product vocabulary)"; exit 2; }
  printf '// see RFC 9113 §8.3.1 for the definition\n' \
    | d8_check_code_lines "selftest" >/dev/null \
    || { echo "d8-gate: SELF-TEST FAILED (RFC section citation false-flagged)"; exit 2; }
  printf '// the review queue lists rows that need a decision\n' \
    | d8_check_code_lines "selftest" >/dev/null \
    || { echo "d8-gate: SELF-TEST FAILED (review as a plain word was flagged)"; exit 2; }
  printf 'const URL: &str = "/api/review/activity/motion";\n' \
    | d8_check_code_lines "selftest" >/dev/null \
    || { echo "d8-gate: SELF-TEST FAILED (review URL path in a string was flagged)"; exit 2; }
  printf 'assert_eq!(src, "/clips/previews/front/hour.mp4");\n' \
    | d8_check_code_lines "selftest" >/dev/null \
    || { echo "d8-gate: SELF-TEST FAILED (previews/ path was flagged)"; exit 2; }
  printf '// see rust-lang/rust#127909 for the miscompile\n' \
    | d8_check_code_lines "selftest" >/dev/null \
    || { echo "d8-gate: SELF-TEST FAILED (upstream owner/repo#N reference was flagged)"; exit 2; }
  printf 'const MASK: u32 = 0xF000;\n' \
    | d8_check_code_lines "selftest" >/dev/null \
    || { echo "d8-gate: SELF-TEST FAILED (F followed by a digit inside a hex literal was flagged)"; exit 2; }
  printf '// recorded on 2026-10-17\n' \
    | d8_check_code_lines "selftest" >/dev/null \
    || { echo "d8-gate: SELF-TEST FAILED (ISO date was flagged)"; exit 2; }
  printf '// see §4.2 of the spec\n' \
    | d8_check_code_lines "selftest" >/dev/null \
    || { echo "d8-gate: SELF-TEST FAILED (bare § spec cite was flagged)"; exit 2; }
  printf 'let key = "aF3xQ9";\n' \
    | d8_check_code_lines "selftest" >/dev/null \
    || { echo "d8-gate: SELF-TEST FAILED (F followed by a digit inside base64 text was flagged)"; exit 2; }
  printf '// fixes issue #18\n' \
    | d8_check_code_lines "selftest" >/dev/null \
    && { echo "d8-gate: SELF-TEST FAILED (issue number not caught)"; exit 2; }
  printf '// per item A1 of the plan\n' \
    | d8_check_code_lines "selftest" >/dev/null \
    && { echo "d8-gate: SELF-TEST FAILED (lettered item label not caught)"; exit 2; }
  printf '// DP-2 decides this\n' \
    | d8_check_code_lines "selftest" >/dev/null \
    && { echo "d8-gate: SELF-TEST FAILED (decision code in a comment not caught)"; exit 2; }
  printf 'assert!(ok, "see D-5");\n' \
    | d8_check_code_lines "selftest" >/dev/null \
    && { echo "d8-gate: SELF-TEST FAILED (register ID in string position not caught)"; exit 2; }
  printf '// the F1 finding\n' \
    | d8_check_code_lines "selftest" >/dev/null \
    && { echo "d8-gate: SELF-TEST FAILED (F label as a word not caught)"; exit 2; }
  n=$(printf 'per plan §E3 this holds\nsee §9.12 of the spec\n' | d8_strip_fmt | grep -cE "$PAT" || true)
  [ "$n" = "1" ] || { echo "d8-gate: SELF-TEST FAILED (plan §E cite not caught or bare § cite flagged)"; exit 2; }
  d8_resolve_range "not-a-real-rev..HEAD" >/dev/null 2>&1
  [ "$?" = "1" ] || { echo "d8-gate: SELF-TEST FAILED (resolve_range did not fail closed on an unresolvable revision)"; exit 2; }
  d8_resolve_range "HEAD..HEAD" >/dev/null 2>&1 || { echo "d8-gate: SELF-TEST FAILED (resolve_range rejected a resolvable revision)"; exit 2; }
}

d8_scope_selftest() {
  local repo f rc
  repo="$(mktemp -d)"
  ( cd "$repo" && git init -q && git config user.email t@t && git config user.name t \
    && mkdir -p tests conformance docs/standards && git commit -q --allow-empty -m base \
    && printf '// plan item 3 per #1234\n' | tee tests/x.rs tests/x.md tests/x.conf conformance/x.sh conformance/x.patch docs/x.md docs/standards/gate.sh >/dev/null \
    && git add -A && git commit -q -m add ) || { rm -rf "$repo"; echo "d8-gate: SELF-TEST FAILED (scope fixture)"; exit 2; }
  for f in tests/x.rs tests/x.md tests/x.conf conformance/x.sh conformance/x.patch; do
    ( cd "$repo" && git diff HEAD~1 -- "${D8_SPEC[@]}" | grep -q "^+++ b/$f" ) \
      || { rm -rf "$repo"; echo "d8-gate: SELF-TEST FAILED ($f not in the scanned pathspec)"; exit 2; }
  done
  for f in docs/x.md docs/standards/gate.sh; do
    ( cd "$repo" && git diff HEAD~1 -- "${D8_SPEC[@]}" | grep -q "^+++ b/$f" ) \
      && { rm -rf "$repo"; echo "d8-gate: SELF-TEST FAILED ($f scanned despite its exemption)"; exit 2; }
  done
  ( cd "$repo" && "$SELF" d8 range HEAD~1..HEAD >/dev/null 2>&1 ); rc=$?
  rm -rf "$repo"
  [ "$rc" = "1" ] || { echo "d8-gate: SELF-TEST FAILED (tests/*.rs hit not flagged by range mode, rc=$rc)"; exit 2; }
}

obs_deps_gen() {
  [ -f Cargo.toml ] && awk '/^\[dependencies\]|^\[dev-dependencies\]/{sect=1;next} /^\[/{sect=0} sect && /^[A-Za-z0-9_-]+[ \t]*=/{sub(/[ \t]*=.*/,"");print}' Cargo.toml | sort -u
  [ -f Cargo.lock ] && awk '/^name = /{gsub(/"/,"",$3); print $3}' Cargo.lock | sort -u
}

leg_obs() {
  local DEPS_BASELINE="docs/standards/obs-deps-baseline.txt"
  case "${1:-}" in
    baseline)
      obs_deps_gen > "$DEPS_BASELINE"
      wc -l < "$DEPS_BASELINE" | xargs echo "entries:" ;;
    deps)
      local FRESH FAIL=0
      FRESH="$(obs_deps_gen)"
      [ -f "$DEPS_BASELINE" ] || { echo "FAIL missing $DEPS_BASELINE — run: gate.sh obs baseline"; exit 1; }
      while IFS= read -r key; do
        [ -z "$key" ] && continue
        grep -qxF "$key" "$DEPS_BASELINE" || { echo "FAIL new dependency key '$key' absent from $DEPS_BASELINE"; FAIL=1; }
      done <<< "$FRESH"
      [ "$FAIL" -eq 0 ] && echo "obs-deps: PASS" || echo "obs-deps: FAIL"
      exit $FAIL ;;
    selftest)
      local REPO HAVE OUT RC
      REPO="$(mktemp -d)"
      # the legs run from a git toplevel, so the fixture is a git repo even though deps never reads git
      ( cd "$REPO" && git init -q && git config user.email t@t && git config user.name t && mkdir -p docs/standards
        printf '[package]\nname = "t"\nversion = "0.1.0"\n' > Cargo.toml
        printf '[[package]]\nname = "alpha"\nversion = "1.0.0"\n\n[[package]]\nname = "beta"\nversion = "1.0.0"\n' > Cargo.lock )
      OUT="$(cd "$REPO" && "$SELF" obs baseline 2>&1)"; RC=$?
      [ "$RC" -eq 0 ] || { rm -rf "$REPO"; echo "selftest FAIL: case (a) baseline exit; rc=$RC: $OUT"; exit 1; }
      HAVE="$(sort "$REPO/docs/standards/obs-deps-baseline.txt" 2>&1)"
      printf 'alpha\nbeta\n' | diff -u - <(printf '%s\n' "$HAVE") > /dev/null \
        || { rm -rf "$REPO"; echo "selftest FAIL: case (a) baseline content; expected alpha and beta, got: $HAVE"; exit 1; }
      OUT="$(cd "$REPO" && "$SELF" obs deps 2>&1)"; RC=$?
      { [ "$RC" -eq 0 ] && printf '%s\n' "$OUT" | grep -q "obs-deps: PASS"; } \
        || { rm -rf "$REPO"; echo "selftest FAIL: case (b) clean deps; rc=$RC: $OUT"; exit 1; }
      printf '[[package]]\nname = "gamma"\nversion = "1.0.0"\n' >> "$REPO/Cargo.lock"
      OUT="$(cd "$REPO" && "$SELF" obs deps 2>&1)"; RC=$?
      { [ "$RC" -ne 0 ] && printf '%s\n' "$OUT" | grep -q "new dependency key 'gamma'" && printf '%s\n' "$OUT" | grep -q "obs-deps: FAIL"; } \
        || { rm -rf "$REPO"; echo "selftest FAIL: case (c) new dependency gamma; rc=$RC: $OUT"; exit 1; }
      rm "$REPO/docs/standards/obs-deps-baseline.txt"
      OUT="$(cd "$REPO" && "$SELF" obs deps 2>&1)"; RC=$?
      { [ "$RC" -ne 0 ] && printf '%s\n' "$OUT" | grep -q "missing"; } \
        || { rm -rf "$REPO"; echo "selftest FAIL: case (d) missing baseline; rc=$RC: $OUT"; exit 1; }
      OUT="$(cd "$REPO" && "$SELF" obs baseline 2>&1)"; RC=$?
      [ "$RC" -eq 0 ] || { rm -rf "$REPO"; echo "selftest FAIL: case (e) baseline rerun; rc=$RC: $OUT"; exit 1; }
      OUT="$(cd "$REPO" && "$SELF" obs deps 2>&1)"; RC=$?
      [ "$RC" -eq 0 ] || { rm -rf "$REPO"; echo "selftest FAIL: case (e) regenerated baseline must clear the new name; rc=$RC: $OUT"; exit 1; }
      rm -rf "$REPO"
      echo "selftest PASS: obs deps — a fresh baseline records the lock names, a clean tree passes, a new dependency name fails by name, a missing baseline fails, and regenerating the baseline clears the new name"
      exit 0 ;;
    *) usage; exit 2 ;;
  esac
}

leg_d8() {
  local ID_PAT_NO_D D_PAT ID_PAT ART_PAT PLAN_PAT PAT D8_SPEC diff locs
  ID_PAT_NO_D='#\[?[0-9]{2,4}|issue [0-9]|[Nn][0-9]{3}|HG-[0-9]|\bpin [0-9]|\bF[0-9]{1,2}\b|plan item|fix.round|\b[Ss]ub-?pass\b|\bS[0-9][a-d]?(-[0-9])?\b|\bP[0-9]\b|\btest_[abcdefot][0-9]+_|\bDP-?[0-9]\b|\bDQ-?[0-9]\b|\binvariant [0-9]|\bnormative requirement [0-9]|\bplan §?E[0-9]|\brung [0-9]|\[E[0-9]+\]|\bE[0-9]+/E[0-9]+\b|\bE[0-9]+-[0-9]+[a-z]?\b|\(E[0-9]+\)|\bI-[0-9]+\b|\bDP-[A-Z]\b|\bE[0-9]{1,2}\b|\b[Vv]erify item [0-9]|\b[Cc]ase [0-9] of the (brief|plan|review)|\b(pre|post)-[A-Z][0-9]{1,3}\b|\b[0-9]{3}-[0-9]\b|\b[Mm]utation[- ][0-9]|\b[Mm]utant[- ]?\(?[0-9]|\bplan M[0-9]{1,2}\b|\bM[0-9]{1,2}'\''s\b|\bM[0-9]{1,2} milestone\b|(\b|_)[Mm][0-9]{1,2}_[a-zA-Z]'
  D_PAT='(\b|_)[Dd]-?[0-9]{1,3}[^0-9a-zA-Z]'
  ID_PAT="$ID_PAT_NO_D|$D_PAT"
  ART_PAT='FINDING|artifacts/|briefs?/|HANDOFF|MAJOR|[Aa]dvisory'
  PLAN_PAT="(this|the) item'?s?\\b|Test \\([a-z]\\)[ :—-]|(design |plan )?item [A-Za-z]?[0-9]{1,2}('s)?[^0-9a-zA-Z]|FIX-NOW"
  PAT="$ID_PAT|$ART_PAT|$PLAN_PAT"
  # Exempt: evidence/ (append-only logs); gate.sh (its selftest literals and patterns are the identifiers the gate detects);
  # docs/ and docs/standards/ markdown (the divergence register, rule and classification tables cite register and tracker keys as their content).
  D8_SPEC=('*.rs' '*.toml' '*.sh' '*.yaml' '*.yml' '*.md' '*Dockerfile*' 'Makefile' 'tests/' 'conformance/'
    ':(exclude)evidence/' ':(exclude)docs/standards/gate.sh' ':(exclude)docs/*.md' ':(exclude)docs/standards/*.md')

  d8_selftest
  case "${1:-}" in
    selftest)
      d8_scope_selftest
      echo "d8-gate: self-test OK" ;;
    staged)
      local rc=0
      diff=$(git diff --cached -- "${D8_SPEC[@]}"); locs=$(mktemp)
      printf '%s\n' "$diff" | d8_added_locs >"$locs"
      printf '%s\n' "$diff" | d8_added_lines \
        | D8_LOCS="$locs" d8_check_code_lines "staged .rs/.toml/.md additions" || { d8_fail_help; rc=1; }
      rm -f "$locs"; exit $rc ;;
    range)
      [ -n "${2:-}" ] || { echo "usage: gate.sh d8 range <a>..<b>"; exit 2; }
      case "$2" in *..*) ;; *) echo "usage: gate.sh d8 range <a>..<b> (two-ended range required)"; exit 2;; esac
      d8_resolve_range "$2" || { echo "d8-gate: cannot resolve '$2' in $(pwd)"; exit 2; }
      local rc=0
      diff=$(git diff "$2" -- "${D8_SPEC[@]}"); locs=$(mktemp)
      printf '%s\n' "$diff" | d8_added_locs >"$locs"
      printf '%s\n' "$diff" | d8_added_lines \
        | D8_LOCS="$locs" d8_check_code_lines "range $2 .rs/.toml/.md additions" || { d8_fail_help; rc=1; }
      rm -f "$locs"
      if git ls-files -- evidence/ | grep -q .; then
        echo "d8-gate: committed files under evidence/ — run logs are CI job artifacts, never committed"; rc=1
      fi
      exit $rc ;;
    *)
      echo "usage: gate.sh d8 staged|range <a>..<b>|selftest"; exit 2 ;;
  esac
}

leg_selftest() {
  local leg fn missing=0
  for leg in comment obs d8; do
    fn="leg_${leg//-/_}"
    type "$fn" >/dev/null 2>&1 || { echo "gate.sh: MISSING leg function for '$leg'"; missing=1; }
  done
  [ "$missing" -eq 0 ] && echo "gate.sh: self-test OK (all leg functions reachable)"
  exit "$missing"
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
  case "${1:-}" in
    comment) shift; leg_comment "$@" ;;
    obs) shift; leg_obs "$@" ;;
    d8) shift; leg_d8 "$@" ;;
    selftest) leg_selftest ;;
    *) usage; exit 2 ;;
  esac
fi
