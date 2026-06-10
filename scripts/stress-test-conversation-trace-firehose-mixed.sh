#!/usr/bin/env bash
set -euo pipefail

total_requests=100
min_delay_ms=10
max_delay_ms=300
sce_bin="${SCE_BIN:-sce}"

usage() {
  cat <<'EOF'
Usage:
  scripts/stress-test-conversation-trace-firehose-mixed.sh [flags]

Firehose-style stress test for `sce hooks conversation-trace`.
Each request is launched in the background, with a random launch-to-launch
delay, and sends a random valid typed batch payload:
`message.updated` or `message.part.updated`.
The script prints a complete summary and exits non-zero if any request process
fails.

Flags:
  -n, --requests <count>        Total requests to launch. Default: 100
  -m, --min-delay-ms <ms>       Minimum delay between launches. Default: 10
  -M, --max-delay-ms <ms>       Maximum delay between launches. Default: 300
      --sce-bin <path>          Binary to invoke. Default: $SCE_BIN or sce
  -h, --help                    Show this help text

Examples:
  scripts/stress-test-conversation-trace-firehose-mixed.sh -n 250 -m 0 -M 25
  SCE_BIN=./target/debug/sce scripts/stress-test-conversation-trace-firehose-mixed.sh --requests 1000
EOF
}

fail() {
  printf 'error: %s\n' "$1" >&2
  exit 1
}

require_value() {
  local flag="$1"
  local value="${2:-}"

  if [[ -z "$value" || "$value" == -* ]]; then
    fail "${flag} requires a value"
  fi
}

is_non_negative_integer() {
  [[ "$1" =~ ^[0-9]+$ ]]
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    -n|--requests)
      require_value "$1" "${2:-}"
      total_requests="$2"
      shift 2
      ;;
    -m|--min-delay-ms)
      require_value "$1" "${2:-}"
      min_delay_ms="$2"
      shift 2
      ;;
    -M|--max-delay-ms)
      require_value "$1" "${2:-}"
      max_delay_ms="$2"
      shift 2
      ;;
    --sce-bin)
      require_value "$1" "${2:-}"
      sce_bin="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      fail "unknown flag '$1'"
      ;;
  esac
done

is_non_negative_integer "$total_requests" || fail "requests must be a non-negative integer"
is_non_negative_integer "$min_delay_ms" || fail "min-delay-ms must be a non-negative integer"
is_non_negative_integer "$max_delay_ms" || fail "max-delay-ms must be a non-negative integer"

if (( total_requests < 1 )); then
  fail "requests must be at least 1"
fi

if (( min_delay_ms > max_delay_ms )); then
  fail "min-delay-ms must be less than or equal to max-delay-ms"
fi

if ! command -v "$sce_bin" >/dev/null 2>&1; then
  fail "sce binary '$sce_bin' was not found; set SCE_BIN or pass --sce-bin"
fi

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
cd "$repo_root"

run_id="$(date -u +%Y%m%dT%H%M%SZ)-$$"
tmp_dir="$(mktemp -d)"
results_dir="$tmp_dir/results"
mkdir -p "$results_dir"

cleanup() {
  rm -rf "$tmp_dir"
}
trap cleanup EXIT

now_ms() {
  date +%s%3N
}

random_between() {
  local min="$1"
  local max="$2"
  local span=$((max - min + 1))
  local random_value=$((((RANDOM << 15) ^ RANDOM) & 0x3fffffff))

  printf '%s\n' $((min + (random_value % span)))
}

sleep_ms() {
  local delay_ms="$1"
  local seconds=$((delay_ms / 1000))
  local millis=$((delay_ms % 1000))

  sleep "${seconds}.$(printf '%03d' "$millis")"
}

random_role() {
  if (( RANDOM % 2 == 0 )); then
    printf 'user\n'
  else
    printf 'assistant\n'
  fi
}

random_part_type() {
  case $((RANDOM % 3)) in
    0) printf 'text\n' ;;
    1) printf 'reasoning\n' ;;
    *) printf 'patch\n' ;;
  esac
}

build_message_updated_payload() {
  local request_index="$1"
  local batch_size="$2"
  local payload='{"type":"message.updated","payloads":['
  local item_index role session_id message_id generated_at_unix_ms separator

  for ((item_index = 1; item_index <= batch_size; item_index++)); do
    role="$(random_role)"
    session_id="stress-session-${run_id}-$((RANDOM % 25))"
    message_id="stress-message-${run_id}-${request_index}-${item_index}"
    generated_at_unix_ms="$(now_ms)"
    separator=','
    if (( item_index == batch_size )); then
      separator=''
    fi

    payload+="{\"session_id\":\"${session_id}\",\"message_id\":\"${message_id}\",\"role\":\"${role}\",\"generated_at_unix_ms\":${generated_at_unix_ms}}${separator}"
  done

  payload+=']}'
  printf '%s\n' "$payload"
}

build_message_part_updated_payload() {
  local request_index="$1"
  local batch_size="$2"
  local payload='{"type":"message.part.updated","payloads":['
  local item_index part_type session_id message_id generated_at_unix_ms text separator

  for ((item_index = 1; item_index <= batch_size; item_index++)); do
    part_type="$(random_part_type)"
    session_id="stress-session-${run_id}-$((RANDOM % 25))"
    message_id="stress-message-${run_id}-${request_index}-${item_index}"
    generated_at_unix_ms="$(now_ms)"
    text="stress ${part_type} payload request ${request_index} item ${item_index} random $RANDOM"
    separator=','
    if (( item_index == batch_size )); then
      separator=''
    fi

    payload+="{\"session_id\":\"${session_id}\",\"message_id\":\"${message_id}\",\"part_type\":\"${part_type}\",\"text\":\"${text}\",\"generated_at_unix_ms\":${generated_at_unix_ms}}${separator}"
  done

  payload+=']}'
  printf '%s\n' "$payload"
}

launch_request() {
  local request_index="$1"
  local event_type batch_size payload

  batch_size="$(random_between 1 4)"
  if (( RANDOM % 2 == 0 )); then
    event_type='message.updated'
    payload="$(build_message_updated_payload "$request_index" "$batch_size")"
  else
    event_type='message.part.updated'
    payload="$(build_message_part_updated_payload "$request_index" "$batch_size")"
  fi

  {
    local started_at_ms ended_at_ms output exit_code
    started_at_ms="$(now_ms)"
    if output="$(printf '%s\n' "$payload" | "$sce_bin" hooks conversation-trace 2>&1)"; then
      exit_code=0
    else
      exit_code=$?
    fi
    ended_at_ms="$(now_ms)"

    printf '%s\n' "$output" >"$results_dir/output-${request_index}.txt"
    printf '%s\t%s\t%s\t%s\t%s\t%s\n' \
      "$request_index" \
      "$exit_code" \
      "$event_type" \
      "$batch_size" \
      "$started_at_ms" \
      "$ended_at_ms" \
      >"$results_dir/result-${request_index}.tsv"
  } &
}

printf 'Conversation-trace firehose stress test\n'
printf 'Repository:          %s\n' "$repo_root"
printf 'Command:             %s hooks conversation-trace\n' "$sce_bin"
printf 'Requests:            %s\n' "$total_requests"
printf 'Launch delay:        %sms to %sms\n' "$min_delay_ms" "$max_delay_ms"
printf 'Batch size:          random 1 to 4 payload items per request\n'
printf 'Run ID:              %s\n' "$run_id"
printf '\n'

test_started_at_ms="$(now_ms)"
pids=()

for ((request_index = 1; request_index <= total_requests; request_index++)); do
  launch_request "$request_index"
  pids+=("$!")

  if (( request_index < total_requests )); then
    sleep_ms "$(random_between "$min_delay_ms" "$max_delay_ms")"
  fi
done

printf 'Launched %s requests. Waiting for background hook processes...\n' "$total_requests"

for pid in "${pids[@]}"; do
  wait "$pid"
done

test_ended_at_ms="$(now_ms)"

completed=0
succeeded=0
failed=0
message_updated_requests=0
message_part_updated_requests=0
message_updated_items=0
message_part_updated_items=0
hook_attempted=0
hook_persisted=0
hook_skipped=0
duration_total_ms=0
failed_request_ids=()

for ((request_index = 1; request_index <= total_requests; request_index++)); do
  result_file="$results_dir/result-${request_index}.tsv"
  output_file="$results_dir/output-${request_index}.txt"

  if [[ ! -f "$result_file" ]]; then
    failed=$((failed + 1))
    failed_request_ids+=("${request_index}:missing-result")
    continue
  fi

  IFS=$'\t' read -r recorded_index exit_code event_type batch_size started_at_ms ended_at_ms <"$result_file"
  completed=$((completed + 1))
  duration_total_ms=$((duration_total_ms + ended_at_ms - started_at_ms))

  if [[ "$event_type" == 'message.updated' ]]; then
    message_updated_requests=$((message_updated_requests + 1))
    message_updated_items=$((message_updated_items + batch_size))
  else
    message_part_updated_requests=$((message_part_updated_requests + 1))
    message_part_updated_items=$((message_part_updated_items + batch_size))
  fi

  if [[ -f "$output_file" ]]; then
    output="$(<"$output_file")"
    if [[ "$output" =~ attempted=([0-9]+),[[:space:]]persisted=([0-9]+),[[:space:]]skipped=([0-9]+) ]]; then
      hook_attempted=$((hook_attempted + BASH_REMATCH[1]))
      hook_persisted=$((hook_persisted + BASH_REMATCH[2]))
      hook_skipped=$((hook_skipped + BASH_REMATCH[3]))
    fi
  fi

  if (( exit_code == 0 )); then
    succeeded=$((succeeded + 1))
  else
    failed=$((failed + 1))
    failed_request_ids+=("${recorded_index}:exit-${exit_code}")
  fi
done

elapsed_ms=$((test_ended_at_ms - test_started_at_ms))
average_duration_ms=0
if (( completed > 0 )); then
  average_duration_ms=$((duration_total_ms / completed))
fi

printf '\nResults\n'
printf '  Requests launched:             %s\n' "$total_requests"
printf '  Requests completed:            %s\n' "$completed"
printf '  Requests succeeded:            %s\n' "$succeeded"
printf '  Requests failed:               %s\n' "$failed"
printf '  message.updated requests:      %s\n' "$message_updated_requests"
printf '  message.part.updated requests: %s\n' "$message_part_updated_requests"
printf '  message.updated payload items: %s\n' "$message_updated_items"
printf '  message.part payload items:    %s\n' "$message_part_updated_items"
printf '  Hook attempted rows reported:  %s\n' "$hook_attempted"
printf '  Hook persisted rows reported:  %s\n' "$hook_persisted"
printf '  Hook skipped rows reported:    %s\n' "$hook_skipped"
printf '  Total elapsed ms:              %s\n' "$elapsed_ms"
printf '  Average hook duration ms:      %s\n' "$average_duration_ms"

if (( failed > 0 )); then
  printf '\nFailed request samples\n'
  sample_count=0
  for failure in "${failed_request_ids[@]}"; do
    printf '  %s\n' "$failure"
    sample_count=$((sample_count + 1))
    if (( sample_count >= 10 )); then
      break
    fi
  done
fi

if (( failed > 0 )); then
  exit 1
fi

exit 0
