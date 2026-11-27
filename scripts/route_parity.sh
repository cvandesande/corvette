#!/usr/bin/env bash
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

# Unique per run: curl appends -w text straight after the body; this literal splits them apart in one call.
boundary="___route_parity_${BASHPID}_${RANDOM}___"

declare -a row_paths=() row_statuses=() row_types=() row_predicates=()

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
      contains:* | absent:* | starts_with:*)
        pred_kind="${predicate%%:*}" pred_value="${predicate#*:}"
        if [[ -z "$pred_value" ]]; then
          echo "$SCRIPT_NAME: $table:$line_no ($f_path): $pred_kind predicate has an empty value -- refusing a predicate that would trivially pass against any body" >&2
          exit 1
        fi
        ;;
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

# The boundary appears twice; the FIRST match ends the body -- the last is the trailer's own closing copy.
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
  # TYPE never contains ':', so splitting on the first two colons after the boundary is exact.
  trailer="${trailer#"$boundary":}"
  trailer="${trailer%"$boundary"}"
  local status="${trailer%%:*}"
  local content_type="${trailer#*:}"
  printf '%s\t%s\n' "$status" "$content_type"
}

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
