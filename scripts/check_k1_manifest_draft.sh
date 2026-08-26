#!/usr/bin/env bash
# Verifies the K1 drafted manifest diff (issue #12, item K1): a reviewable
# document that is never applied to any cluster or written back to
# `~/dockers/kubernetes`. This script performs four independent,
# mechanically-checked assertions, named directly by the item's own Do
# step 3:
#
#   1. INV-2: every image reference this diff ADDS carries either the exact
#      placeholder `PENDING-DIGEST` (while no real digest has yet been
#      recorded from an actual push) or, once one has, the exact digest
#      P1's own publish script recorded -- never a bare tag, and never a
#      digest-shaped value this repo never actually produced.
#   2. The chosen UDP NodePort collides with no other port number declared
#      anywhere in the diff's resulting manifest, and sits inside
#      Kubernetes' own default NodePort range.
#   3. The cert-manager Secret name this diff mounts into the relay
#      container matches the live manifest's OWN Ingress `tls[].secretName`
#      -- read fresh from the live file every run, not hardcoded, so drift
#      on either side is caught rather than silently trusted.
#   4. The `rtsp`-named containerPort appears on EXACTLY ONE container in
#      the diff's resulting pod template -- never zero (silently dropped),
#      never two (duplicated across containers).
#
# Fails closed: any condition that prevents establishing a fact (a missing
# file, an unparseable structure, an ambiguous count) is a failure, not a
# skipped check.
#
# Usage: check_k1_manifest_draft.sh [draft_file] [live_manifest_file]
#   draft_file:        defaults to .agents/issue-12/evidence/K1-manifest-draft.yaml
#   live_manifest_file: defaults to ~/dockers/kubernetes/tirnanog/frigate.yaml
#   Both may be overridden -- for mutation testing against a scratch copy of
#   either file. Never override live_manifest_file for a real run: it exists
#   so this check keeps reading the actual deployed source of truth, not a
#   frozen copy of it.
set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
DRAFT_FILE="${1:-$REPO/.agents/issue-12/evidence/K1-manifest-draft.yaml}"
LIVE_MANIFEST="${2:-$HOME/dockers/kubernetes/tirnanog/frigate.yaml}"
DIGEST_RECORD="$REPO/.agents/issue-12/evidence/corvette-media-bridge-digest.txt"

if [[ ! -f "$DRAFT_FILE" ]]; then
  echo "check_k1_manifest_draft: no such file: $DRAFT_FILE" >&2
  exit 1
fi
if [[ ! -f "$LIVE_MANIFEST" ]]; then
  echo "check_k1_manifest_draft: no such file: $LIVE_MANIFEST" >&2
  exit 1
fi

fail=0

# --- Reconstruct the diff's resulting ("final") state -----------------------
#
# Only a line this diff ADDS, or leaves as unchanged CONTEXT, is part of the
# result; a removed line is gone. Diff file/hunk headers are structural, not
# content. This file's own free-form header commentary above the diff (every
# line starts with '#', never a bare diff marker) naturally falls out of
# every pattern below and is excluded the same way, with no separate
# "find where the diff starts" step needed.
scratch_dir="$(mktemp -d)"
trap 'rm -rf "$scratch_dir"' EXIT
final_state="$scratch_dir/final_state"
added_lines="$scratch_dir/added_lines"
: >"$final_state"
: >"$added_lines"

awk -v final="$final_state" -v added="$added_lines" '
  /^\+\+\+ / { next }
  /^--- /    { next }
  /^@@/      { next }
  /^\+/      { line = substr($0, 2); print line > final; print line > added; next }
  /^ /       { print substr($0, 2) > final; next }
  { next }
' "$DRAFT_FILE"

if [[ ! -s "$final_state" ]]; then
  echo "check_k1_manifest_draft: reconstructed final state is empty -- $DRAFT_FILE does not look like a unified diff (no '--- a/' / '+++ b/' header found, or no context/added lines under it)" >&2
  exit 1
fi
if [[ ! -s "$added_lines" ]]; then
  echo "check_k1_manifest_draft: no added ('+') lines found in $DRAFT_FILE -- a diff that adds nothing cannot be this item's draft" >&2
  exit 1
fi

# --- Check 1: INV-2, digest-or-placeholder form on every added image line --
image_lines="$(grep -E '^[[:space:]]*image:' "$added_lines" || true)"
if [[ -z "$image_lines" ]]; then
  echo "check_k1_manifest_draft: no 'image:' line found among this diff's added lines -- expected at least the moq-relay and corvette-media-bridge containers to declare one" >&2
  fail=1
else
  expected_digest=""
  if [[ -f "$DIGEST_RECORD" ]]; then
    expected_digest="$(grep -E '^digest: ' "$DIGEST_RECORD" | sed -E 's/^digest: //')"
    if [[ ! "$expected_digest" =~ ^sha256:[0-9a-f]{64}$ ]]; then
      echo "check_k1_manifest_draft: $DIGEST_RECORD exists but its 'digest: ' line is not a well-formed sha256:<64 hex> value ('$expected_digest') -- cannot establish the expected real digest" >&2
      fail=1
      expected_digest=""
    fi
  fi

  while IFS= read -r img_line; do
    [[ -z "$img_line" ]] && continue
    value="$(sed -E 's/^[[:space:]]*image:[[:space:]]*//' <<<"$img_line")"
    if [[ -n "$expected_digest" ]]; then
      if [[ "$value" != *"@$expected_digest" ]]; then
        echo "check_k1_manifest_draft: INV-2 violation -- image reference '$value' does not carry the real digest recorded at $DIGEST_RECORD (@$expected_digest)" >&2
        fail=1
      fi
    else
      if [[ "$value" != *"@PENDING-DIGEST" ]]; then
        echo "check_k1_manifest_draft: INV-2 violation -- image reference '$value' must end in the exact placeholder '@PENDING-DIGEST' while $DIGEST_RECORD does not yet exist (no real digest has been recorded from an actual push); found something else instead, which is never legitimate whether or not it happens to look like a real digest" >&2
        fail=1
      fi
    fi
  done <<<"$image_lines"
fi

# --- Check 2: the chosen UDP NodePort is unique and in-range ----------------
nodeport_lines="$(grep -E '^[[:space:]]*nodePort:[[:space:]]*[0-9]+' "$added_lines" || true)"
nodeport_count="$(printf '%s\n' "$nodeport_lines" | grep -c . || true)"
if [[ "$nodeport_count" -ne 1 ]]; then
  echo "check_k1_manifest_draft: expected exactly one added 'nodePort:' line, found $nodeport_count" >&2
  fail=1
else
  chosen_nodeport="$(sed -E 's/^[[:space:]]*nodePort:[[:space:]]*//' <<<"$nodeport_lines")"
  if ! [[ "$chosen_nodeport" =~ ^[0-9]+$ ]]; then
    echo "check_k1_manifest_draft: nodePort value '$chosen_nodeport' is not a plain integer" >&2
    fail=1
  else
    # Range and collision are independent facts -- checked and reported
    # separately, neither gated behind the other, so a mutation that only
    # breaks one of them is still named for the one it broke.
    if (( chosen_nodeport < 30000 || chosen_nodeport > 32767 )); then
      echo "check_k1_manifest_draft: chosen nodePort $chosen_nodeport is outside Kubernetes' default --service-node-port-range (30000-32767)" >&2
      fail=1
    fi
    # Every numeric port/containerPort/nodePort field anywhere in the
    # drafted result. If the chosen NodePort collides with some OTHER port
    # this manifest declares, its value appears more than once in this list
    # (its own nodePort line, plus whatever it collided with).
    all_port_values="$(grep -E '^[[:space:]]*(port|containerPort|nodePort):[[:space:]]*[0-9]+' "$final_state" | sed -E 's/^[[:space:]]*[a-zA-Z]+:[[:space:]]*//' || true)"
    collision_count="$(printf '%s\n' "$all_port_values" | grep -xc "$chosen_nodeport" || true)"
    if [[ "$collision_count" -gt 1 ]]; then
      echo "check_k1_manifest_draft: chosen nodePort $chosen_nodeport collides with another port/containerPort/nodePort declared elsewhere in the drafted manifest (that exact number appears $collision_count times total, expected exactly 1: its own nodePort declaration)" >&2
      fail=1
    fi
  fi
fi

# --- Check 3: the drafted cert Secret name matches the live manifest --------
live_secret_name="$(grep -E '^[[:space:]]*secretName:[[:space:]]*' "$LIVE_MANIFEST" | sed -E 's/^[[:space:]]*secretName:[[:space:]]*//' | sort -u || true)"
live_secret_count="$(printf '%s\n' "$live_secret_name" | grep -c . || true)"
if [[ "$live_secret_count" -ne 1 ]]; then
  echo "check_k1_manifest_draft: expected exactly one distinct 'secretName:' value in $LIVE_MANIFEST, found $live_secret_count -- cannot establish the live cert Secret's name unambiguously" >&2
  fail=1
else
  drafted_secret_name="$(grep -E '^[[:space:]]*secretName:[[:space:]]*' "$added_lines" | sed -E 's/^[[:space:]]*secretName:[[:space:]]*//' || true)"
  drafted_secret_count="$(printf '%s\n' "$drafted_secret_name" | grep -c . || true)"
  if [[ "$drafted_secret_count" -ne 1 ]]; then
    echo "check_k1_manifest_draft: expected exactly one added 'secretName:' line (the relay's TLS Secret volume), found $drafted_secret_count" >&2
    fail=1
  elif [[ "$drafted_secret_name" != "$live_secret_name" ]]; then
    echo "check_k1_manifest_draft: drafted cert Secret name '$drafted_secret_name' does not match the live manifest's own Secret name '$live_secret_name' ($LIVE_MANIFEST)" >&2
    fail=1
  fi
fi

# --- Check 4: the 'rtsp'-named containerPort appears on exactly one container
rtsp_containerport_count="$(awk '
  /^[[:space:]]*#/ { next }
  /^[[:space:]]*- name: rtsp[[:space:]]*$/ { pending = 1; next }
  pending && /containerPort:/ { count++; pending = 0; next }
  { pending = 0 }
  END { print count + 0 }
' "$final_state")"
if [[ "$rtsp_containerport_count" -ne 1 ]]; then
  echo "check_k1_manifest_draft: expected exactly one 'rtsp'-named containerPort in the drafted pod template, found $rtsp_containerport_count" >&2
  fail=1
fi

if [[ "$fail" -ne 0 ]]; then
  exit 1
fi

echo "check_k1_manifest_draft: $DRAFT_FILE matches the expected shape (INV-2 image form, collision-free in-range NodePort, cert Secret name matches $LIVE_MANIFEST, exactly one rtsp-named containerPort)"
