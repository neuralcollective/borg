#!/usr/bin/env bash
set -euo pipefail

BASE_URL="${1:-http://127.0.0.1:3131}"

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing required command: $1" >&2
    exit 1
  }
}

need_cmd curl
need_cmd jq

warn_count=0
fail_count=0

warn() {
  warn_count=$((warn_count + 1))
  echo "WARN: $*"
}

fail() {
  fail_count=$((fail_count + 1))
  echo "FAIL: $*"
}

ok() {
  echo "OK: $*"
}

echo "==> Preflight against ${BASE_URL}"

if ! curl -fsS "${BASE_URL}/api/health" >/dev/null; then
  fail "health endpoint failed (${BASE_URL}/api/health)"
else
  ok "health endpoint reachable"
fi

token_json="$(curl -fsS "${BASE_URL}/api/auth/token" || true)"
token="$(printf '%s' "${token_json}" | jq -r '.token // empty' 2>/dev/null || true)"
if [[ -z "${token}" ]]; then
  fail "could not fetch auth token from /api/auth/token"
fi

settings="$(curl -fsS -H "Authorization: Bearer ${token}" "${BASE_URL}/api/settings" || true)"
if [[ -z "${settings}" ]]; then
  fail "could not fetch /api/settings"
fi

backend="$(printf '%s' "${settings}" | jq -r '.backend // ""')"
storage_backend="$(printf '%s' "${settings}" | jq -r '.storage_backend // ""')"
backup_backend="$(printf '%s' "${settings}" | jq -r '.backup_backend // ""')"
search_backend="$(printf '%s' "${settings}" | jq -r '.search_backend // ""')"
public_url="$(printf '%s' "${settings}" | jq -r '.public_url // ""')"
project_max_bytes="$(printf '%s' "${settings}" | jq -r '.project_max_bytes // 0')"
pipeline_max_agents="$(printf '%s' "${settings}" | jq -r '.pipeline_max_agents // 0')"

if [[ "${backend}" == "codex" ]]; then
  ok "agent backend is codex"
elif [[ -n "${backend}" ]]; then
  warn "agent backend is '${backend}' (expected codex)"
else
  warn "agent backend is empty"
fi

if [[ "${storage_backend}" != "local" && "${storage_backend}" != "s3" ]]; then
  fail "storage_backend must be local or s3 (got '${storage_backend}')"
else
  ok "storage backend set to ${storage_backend}"
fi

if [[ "${storage_backend}" == "s3" ]]; then
  s3_bucket="$(printf '%s' "${settings}" | jq -r '.s3_bucket // ""')"
  s3_region="$(printf '%s' "${settings}" | jq -r '.s3_region // ""')"
  if [[ -z "${s3_bucket}" ]]; then fail "s3_bucket is required when storage_backend=s3"; else ok "s3_bucket configured"; fi
  if [[ -z "${s3_region}" ]]; then fail "s3_region is required when storage_backend=s3"; else ok "s3_region configured"; fi
fi

if [[ "${backup_backend}" != "disabled" && "${backup_backend}" != "s3" ]]; then
  fail "backup_backend must be disabled or s3 (got '${backup_backend}')"
elif [[ "${backup_backend}" == "s3" ]]; then
  backup_bucket="$(printf '%s' "${settings}" | jq -r '.backup_bucket // ""')"
  backup_region="$(printf '%s' "${settings}" | jq -r '.backup_region // ""')"
  if [[ -z "${backup_bucket}" ]]; then fail "backup_bucket is required when backup_backend=s3"; else ok "backup_bucket configured"; fi
  if [[ -z "${backup_region}" ]]; then fail "backup_region is required when backup_backend=s3"; else ok "backup_region configured"; fi
else
  warn "offsite backups are disabled"
fi

if [[ "${search_backend}" != "vespa" ]]; then
  fail "search_backend must be vespa (got '${search_backend}')"
else
  vespa_url="$(printf '%s' "${settings}" | jq -r '.vespa_url // ""')"
  if [[ -z "${vespa_url}" ]]; then
    fail "vespa_url is required when search_backend=vespa"
  else
    ok "vespa_url configured"
    chunk_check=$(curl -sf "${vespa_url}/search/?yql=select+*+from+project_chunk+where+true+limit+0" 2>/dev/null || true)
    if echo "$chunk_check" | grep -q '"totalCount"'; then
      ok "project_chunk schema active"
    else
      warn "project_chunk schema NOT deployed — run: just stack-up"
    fi
  fi
fi

if [[ "${public_url}" =~ ^https?:// ]]; then
  ok "public_url looks valid"
else
  warn "public_url is not set to http(s); OAuth cloud callbacks will fail"
fi

if [[ "${project_max_bytes}" -lt 104857600 ]]; then
  warn "project_max_bytes is below 100MB; may be too low for discovery matters"
else
  ok "project_max_bytes is ${project_max_bytes}"
fi

if [[ "${pipeline_max_agents}" -lt 1 ]]; then
  fail "pipeline_max_agents must be >= 1"
else
  ok "pipeline_max_agents is ${pipeline_max_agents}"
fi

if [[ "${fail_count}" -gt 0 ]]; then
  echo
  echo "Preflight result: ${fail_count} fail(s), ${warn_count} warning(s)"
  exit 1
fi

echo
echo "Preflight result: PASS with ${warn_count} warning(s)"
