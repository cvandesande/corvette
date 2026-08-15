#!/usr/bin/env bash
# Checks a declarative table of routes against a running nginx -- deployed
# Frigate in --mode pod, or a local fixture in --mode local -- and reports any
# route whose status, content type, or body shape does not match what the
# table says it should be.
#
# Two known routes in this system return HTTP 200 with the wrong body when
# something is broken: a client route shadowed by a filesystem autoindex, and
# an iframe that loads this site's own shell into itself. A check that only
# reads the status code cannot see either failure -- both look like success.
# So every row states a content type AND a body-shape predicate, and this
# script refuses to run any table where a row is missing one: that refusal is
# the actual enforcement, not the checks that run once it passes.
#
# Usage:
#   route_parity.sh --table FILE --mode local --base-url URL [--json-out FILE]
#   route_parity.sh --table FILE --mode pod --namespace NS --pod POD \
#     --base-url URL [--json-out FILE]
#
#   --table     the expectations file (format below).
#   --mode      local: fetch URL directly with curl, for a fixture on this
#               machine. pod: fetch by running curl *inside* --pod via
#               `kubectl exec`, so nothing here needs a port-forward and
#               nothing on this machine ends up serving the checked routes.
#   --base-url  origin to fetch from -- http://127.0.0.1:PORT in both modes;
#               in --mode pod this is resolved inside the pod's own network
#               namespace, not this machine's.
#   --json-out  write one JSON object per row (path, expected/actual status,
#               expected/actual content type, whether it passed) to FILE.
#
# Table format: one route per line, four '|'-separated fields:
#   PATH | EXPECTED-STATUS | EXPECTED-CONTENT-TYPE | BODY-PREDICATE
# Blank lines and lines starting with '#' are skipped. BODY-PREDICATE is one
# or more ';'-separated predicates, each PREFIX:VALUE:
#   contains:TEXT     the body contains TEXT as a literal substring
#   absent:TEXT       the body does not contain TEXT
#   starts_with:TEXT  the body's first bytes are exactly TEXT
#   magic:HEX         the body's first len(HEX)/2 bytes equal HEX
# EXPECTED-CONTENT-TYPE matches by prefix, so a row can state "text/html"
# without also pinning a charset the server happens to append.
set -euo pipefail

SCRIPT_NAME="route_parity"

table=""
mode=""
base_url=""
namespace=""
pod=""
json_out=""

while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --table) table="$2"; shift 2 ;;
    --mode) mode="$2"; shift 2 ;;
    --base-url) base_url="$2"; shift 2 ;;
    --namespace) namespace="$2"; shift 2 ;;
    --pod) pod="$2"; shift 2 ;;
    --json-out) json_out="$2"; shift 2 ;;
    *)
      echo "$SCRIPT_NAME: unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

if [[ -z "$table" ]]; then
  echo "$SCRIPT_NAME: --table is required" >&2
  exit 2
fi
if [[ ! -f "$table" ]]; then
  echo "$SCRIPT_NAME: no such table file: $table" >&2
  exit 1
fi
if [[ -z "$base_url" ]]; then
  echo "$SCRIPT_NAME: --base-url is required" >&2
  exit 2
fi
case "$mode" in
  local) ;;
  pod)
    if [[ -z "$namespace" || -z "$pod" ]]; then
      echo "$SCRIPT_NAME: --mode pod requires --namespace and --pod" >&2
      exit 2
    fi
    ;;
  "")
    echo "$SCRIPT_NAME: --mode is required (local or pod)" >&2
    exit 2
    ;;
  *)
    echo "$SCRIPT_NAME: --mode must be local or pod, got '$mode'" >&2
    exit 2
    ;;
esac

work_dir="$(mktemp -d)"
trap 'rm -rf "$work_dir"' EXIT

# A boundary that cannot appear by accident in a route's body: unique to this
# process and run, so two invocations racing in the same /tmp cannot collide,
# and long enough that a coincidental match inside binary media (JPEG, MP4,
# WebP bytes) is not a realistic risk. It marks where curl's own -w output
# starts, appended immediately after the body with no separator of curl's
# choosing -- searching for this exact literal is what lets one curl call
# return body, status, and content type without a second round trip to parse
# headers separately.
boundary="___route_parity_${BASHPID}_${RANDOM}___"

# Phase 1: validate the table before making a single request. A row missing a
# content type or a body predicate would otherwise pass silently -- reduce it
# to a status check the moment the table is read, so nothing downstream ever
# sees a row like that.
declare -a row_paths=() row_statuses=() row_types=() row_predicates=()

# Trims leading/trailing whitespace off one field without a subprocess.
trim() {
  local s="$1"
  s="${s#"${s%%[![:space:]]*}"}"
  s="${s%"${s##*[![:space:]]}"}"
  printf '%s' "$s"
}

line_no=0
while IFS= read -r line || [[ -n "$line" ]]; do
  line_no=$((line_no + 1))
  trimmed="$(trim "$line")"
  [[ -z "$trimmed" || "$trimmed" == \#* ]] && continue

  # `read` with fewer IFS-delimited fields than variables leaves the missing
  # ones empty rather than unset, so a row with too few '|' separators is not
  # a distinct case to detect here -- it surfaces below as an empty content
  # type or predicate, exactly like a row that spelled out the field as blank.
  IFS='|' read -r f_path f_status f_type f_predicate <<<"$line"
  f_path="$(trim "$f_path")"
  f_status="$(trim "${f_status:-}")"
  f_type="$(trim "${f_type:-}")"
  f_predicate="$(trim "${f_predicate:-}")"

  if [[ -z "$f_path" || -z "$f_status" ]]; then
    echo "$SCRIPT_NAME: $table:$line_no: missing path or expected status" >&2
    exit 1
  fi
  if [[ -z "$f_type" ]]; then
    echo "$SCRIPT_NAME: $table:$line_no ($f_path): no expected content type -- refusing to run a table that would let this row pass on status alone" >&2
    exit 1
  fi
  if [[ -z "$f_predicate" ]]; then
    echo "$SCRIPT_NAME: $table:$line_no ($f_path): no body predicate -- refusing to run a table that would let this row pass on a 200 carrying the wrong body" >&2
    exit 1
  fi

  IFS=';' read -ra predicates <<<"$f_predicate"
  for predicate in "${predicates[@]}"; do
    case "$predicate" in
      contains:* | absent:* | starts_with:*) ;;
      magic:*)
        hex="${predicate#magic:}"
        if [[ -z "$hex" || ! "$hex" =~ ^[0-9a-fA-F]+$ || $((${#hex} % 2)) -ne 0 ]]; then
          echo "$SCRIPT_NAME: $table:$line_no ($f_path): magic predicate is not an even-length hex string: '$hex'" >&2
          exit 1
        fi
        ;;
      *)
        echo "$SCRIPT_NAME: $table:$line_no ($f_path): unreadable predicate '$predicate' -- want contains:, absent:, starts_with:, or magic:" >&2
        exit 1
        ;;
    esac
  done

  row_paths+=("$f_path")
  row_statuses+=("$f_status")
  row_types+=("$f_type")
  row_predicates+=("$f_predicate")
done <"$table"

rows_in="${#row_paths[@]}"
if [[ "$rows_in" -eq 0 ]]; then
  echo "$SCRIPT_NAME: $table has no rows -- nothing to check" >&2
  exit 1
fi

# Phase 2: fetch one row. Body, status, and content type all come back from a
# single request -- curl's -w text is appended straight onto the response
# body, so the boundary above is what separates them again once the bytes are
# on disk. Redirecting straight to a file (never through a shell variable)
# is what keeps a NUL byte in a JPEG or a wasm binary from being silently
# dropped, which a command substitution would do.
fetch_row() {
  local path=$1 raw_file=$2
  local url="$base_url$path"
  local -a curl_args=(--silent --show-error
    --write-out "${boundary}:%{http_code}:%{content_type}${boundary}"
    "$url")
  case "$mode" in
    local) curl "${curl_args[@]}" >"$raw_file" 2>"$raw_file.stderr" ;;
    pod) kubectl -n "$namespace" exec "$pod" -- curl "${curl_args[@]}" >"$raw_file" 2>"$raw_file.stderr" ;;
  esac
}

# Splits a fetch_row raw file into a body file and prints "STATUS<TAB>TYPE".
# Returns non-zero if the boundary never arrived -- a request that never
# completed (a dropped connection, a pod curl invocation that itself failed)
# leaves no trailer to find, and that is reported as this row's own distinct
# failure rather than as a body/type mismatch it never actually measured.
#
# The boundary appears twice in a well-formed response -- once opening the
# trailer, once closing it -- so the FIRST match is what marks the end of the
# body; taking the last would count the trailer's own closing copy as part of
# the body.
split_response() {
  local raw_file=$1 body_file=$2
  local offset
  offset="$(grep -aob -F -- "$boundary" "$raw_file" | head -1 | cut -d: -f1)"
  if [[ -z "$offset" ]]; then
    return 1
  fi
  head -c "$offset" "$raw_file" >"$body_file"
  local trailer
  trailer="$(tail -c +"$((offset + 1))" "$raw_file")"
  # trailer is "<boundary>:STATUS:TYPE<boundary>"; TYPE itself never contains
  # ':', so splitting on the first two colons after the opening boundary is
  # exact, not merely the common case.
  trailer="${trailer#"$boundary":}"
  trailer="${trailer%"$boundary"}"
  local status="${trailer%%:*}"
  local content_type="${trailer#*:}"
  printf '%s\t%s\n' "$status" "$content_type"
}

# Evaluates one predicate against a body file. magic compares hex-encoded
# bytes rather than a literal string match, so a predicate can pin bytes a
# shell string cannot hold (a wasm module's leading NUL).
check_predicate() {
  local predicate=$1 body_file=$2
  local kind="${predicate%%:*}" value="${predicate#*:}"
  case "$kind" in
    contains) grep -aq -F -- "$value" "$body_file" ;;
    absent) ! grep -aq -F -- "$value" "$body_file" ;;
    starts_with)
      local head_bytes
      head_bytes="$(head -c "${#value}" "$body_file")"
      [[ "$head_bytes" == "$value" ]]
      ;;
    magic)
      local want_hex="${value,,}" got_hex
      got_hex="$(head -c "$((${#want_hex} / 2))" "$body_file" | od -An -tx1 | tr -d ' \n')"
      [[ "$got_hex" == "$want_hex" ]]
      ;;
    *)
      return 1
      ;;
  esac
}

# Phase 3: run every row. checks_run only ever grows by exactly one per row,
# so it is also the row-count-out this script's own completeness bar
# requires: a row this loop does not reach is a row that was silently
# dropped, and that is a bug in this script, not an acceptable outcome.
checks_run=0
checks_failed=0
json_rows=()

for i in "${!row_paths[@]}"; do
  path="${row_paths[$i]}"
  want_status="${row_statuses[$i]}"
  want_type="${row_types[$i]}"
  predicate_field="${row_predicates[$i]}"
  checks_run=$((checks_run + 1))

  raw_file="$work_dir/row-$i.raw"
  body_file="$work_dir/row-$i.body"

  if ! fetch_row "$path" "$raw_file"; then
    echo "  FAIL $path: request did not complete ($(cat "$raw_file.stderr" 2>/dev/null))" >&2
    checks_failed=$((checks_failed + 1))
    json_rows+=("{\"path\":\"${path//\"/\\\"}\",\"result\":\"unreachable\"}")
    continue
  fi

  parsed="$(split_response "$raw_file" "$body_file")" || {
    echo "  FAIL $path: no result boundary in the response -- request did not complete" >&2
    checks_failed=$((checks_failed + 1))
    json_rows+=("{\"path\":\"${path//\"/\\\"}\",\"result\":\"unreachable\"}")
    continue
  }
  got_status="${parsed%%$'\t'*}"
  got_type="${parsed#*$'\t'}"

  row_ok=1
  reason=""
  if [[ "$got_status" != "$want_status" ]]; then
    row_ok=0
    reason="status $got_status, expected $want_status"
  elif [[ "$got_type" != "$want_type"* ]]; then
    row_ok=0
    reason="content-type '$got_type', expected '$want_type'"
  else
    IFS=';' read -ra predicates <<<"$predicate_field"
    for predicate in "${predicates[@]}"; do
      if ! check_predicate "$predicate" "$body_file"; then
        row_ok=0
        reason="body predicate failed: $predicate"
        break
      fi
    done
  fi

  if [[ "$row_ok" -eq 1 ]]; then
    echo "  ok   $path: $got_status $got_type"
  else
    echo "  FAIL $path: $reason" >&2
    checks_failed=$((checks_failed + 1))
  fi

  json_rows+=("$(printf '{"path":"%s","expected_status":"%s","actual_status":"%s","expected_content_type":"%s","actual_content_type":"%s","predicate":"%s","pass":%s}' \
    "${path//\"/\\\"}" "$want_status" "$got_status" "$want_type" "${got_type//\"/\\\"}" "${predicate_field//\"/\\\"}" \
    "$([[ "$row_ok" -eq 1 ]] && echo true || echo false)")")
done

if [[ -n "$json_out" ]]; then
  {
    printf '[\n'
    for i in "${!json_rows[@]}"; do
      printf '  %s%s\n' "${json_rows[$i]}" "$([[ "$i" -lt $((${#json_rows[@]} - 1)) ]] && echo ,)"
    done
    printf ']\n'
  } >"$json_out"
fi

echo "$SCRIPT_NAME: $checks_run/$rows_in rows checked, $checks_failed failed"
if [[ "$checks_run" -ne "$rows_in" ]]; then
  echo "$SCRIPT_NAME: row count mismatch -- $rows_in in the table, $checks_run actually checked" >&2
  exit 1
fi
if [[ "$checks_failed" -ne 0 ]]; then
  exit 1
fi
