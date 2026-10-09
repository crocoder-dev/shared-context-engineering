#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'USAGE'
Usage: scripts/bench-hook-telemetry.sh <sce-binary-built-with-telemetry-test-receiver> <output-dir>

Benchmarks `sce hooks pre-commit` under four telemetry modes (disabled, reachable
loopback receiver, accept-but-never-respond blackhole, connection refused) with
hyperfine, then measures post-command shutdown from the D6 lifecycle markers.

Build the binary first:
  nix develop -c ./scripts/run-cli-cargo.sh build --release \
    --features telemetry-test-receiver --manifest-path cli/Cargo.toml

Run:
  nix shell nixpkgs#hyperfine nixpkgs#python3 -c \
    scripts/bench-hook-telemetry.sh cli/target/release/sce <output-dir>

Environment: BENCH_WARMUP (default 5), BENCH_RUNS (default 40),
BENCH_MARKER_RUNS (default 40), SCE_TELEMETRY_FLUSH_TIMEOUT_MS (default 1000).
USAGE
}

if [ "$#" -ne 2 ]; then
  usage
  exit 2
fi

for tool in hyperfine python3; do
  if ! command -v "${tool}" >/dev/null 2>&1; then
    printf 'bench-hook-telemetry: %s not found; run via nix shell nixpkgs#hyperfine nixpkgs#python3 -c\n' "${tool}" >&2
    exit 2
  fi
done

sce_binary="$(realpath "$1")"
output_dir="$(mkdir -p "$2" && realpath "$2")"
warmup="${BENCH_WARMUP:-5}"
runs="${BENCH_RUNS:-40}"
marker_runs="${BENCH_MARKER_RUNS:-40}"
flush_ms="${SCE_TELEMETRY_FLUSH_TIMEOUT_MS:-1000}"

if [ ! -x "${sce_binary}" ]; then
  printf 'bench-hook-telemetry: %s is not an executable file\n' "${sce_binary}" >&2
  exit 2
fi

sandbox="$(mktemp -d "${TMPDIR:-/tmp}/bench-hook-telemetry.XXXXXX")"
listener_pids=()
cleanup() {
  for pid in "${listener_pids[@]:-}"; do
    [ -n "${pid}" ] && kill "${pid}" 2>/dev/null || true
  done
  rm -rf "${sandbox}"
}
trap cleanup EXIT

listener="${sandbox}/listener.py"
cat >"${listener}" <<'PY'
import socket
import sys
import threading

mode = sys.argv[1]
port_file = sys.argv[2]
server = socket.socket()
server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
server.bind(("127.0.0.1", 0))
server.listen(256)
with open(port_file, "w") as handle:
    handle.write(str(server.getsockname()[1]))
held = []


def serve(conn):
    data = b""
    while b"\r\n\r\n" not in data:
        chunk = conn.recv(65536)
        if not chunk:
            conn.close()
            return
        data += chunk
    head, _, body = data.partition(b"\r\n\r\n")
    length = 0
    for line in head.split(b"\r\n"):
        if line.lower().startswith(b"content-length:"):
            length = int(line.split(b":")[1])
    while len(body) < length:
        chunk = conn.recv(65536)
        if not chunk:
            break
        body += chunk
    conn.sendall(
        b"HTTP/1.1 200 OK\r\nContent-Type: application/x-protobuf\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
    conn.close()


while True:
    conn, _ = server.accept()
    if mode == "blackhole":
        held.append(conn)
    else:
        threading.Thread(target=serve, args=(conn,), daemon=True).start()
PY

start_listener() {
  local mode="$1" port_file="${sandbox}/${1}.port"
  python3 "${listener}" "${mode}" "${port_file}" >/dev/null 2>&1 &
  listener_pids+=("$!")
  for _ in $(seq 1 100); do
    [ -s "${port_file}" ] && break
    sleep 0.05
  done
}

closed_port() {
  python3 - <<'PY'
import socket
s = socket.socket()
s.bind(("127.0.0.1", 0))
print(s.getsockname()[1])
s.close()
PY
}

start_listener reachable
start_listener blackhole
reachable_port="$(cat "${sandbox}/reachable.port")"
blackhole_port="$(cat "${sandbox}/blackhole.port")"
refused_port="$(closed_port)"

base_env=(
  "PATH=${PATH}"
  "HOME=${sandbox}"
  "XDG_CONFIG_HOME=${sandbox}/config"
  "XDG_STATE_HOME=${sandbox}/state"
  "XDG_CACHE_HOME=${sandbox}/cache"
  "NO_COLOR=1"
)

receiver_env() {
  printf 'SCE_TELEMETRY=test-receiver SCE_TELEMETRY_TEST_RECEIVER_ENDPOINT=http://127.0.0.1:%s SCE_TELEMETRY_FLUSH_TIMEOUT_MS=%s' "$1" "${flush_ms}"
}

mode_command() {
  local mode="$1" extra=""
  case "${mode}" in
    disabled) extra="" ;;
    reachable) extra="$(receiver_env "${reachable_port}")" ;;
    blackhole) extra="$(receiver_env "${blackhole_port}")" ;;
    refused) extra="$(receiver_env "${refused_port}")" ;;
  esac
  printf 'env -i %s %s %s hooks pre-commit' "${base_env[*]}" "${extra}" "${sce_binary}"
}

modes=(disabled reachable blackhole refused)
mkdir -p "${sandbox}/work"
cd "${sandbox}/work"

for mode in "${modes[@]}"; do
  hyperfine --warmup "${warmup}" --runs "${runs}" --input null --style basic \
    --export-json "${output_dir}/hyperfine-${mode}.json" \
    -n "${mode}" "$(mode_command "${mode}")" \
    | tee "${output_dir}/hyperfine-${mode}.txt"
done

for mode in reachable blackhole refused; do
  marker_file="${output_dir}/markers-${mode}.txt"
  : >"${marker_file}"
  for i in $(seq 1 "${marker_runs}"); do
    run_file="${sandbox}/run.markers"
    rm -f "${run_file}"
    marker_command="$(mode_command "${mode}")"
    marker_command="${marker_command/env -i /env -i SCE_TELEMETRY_TEST_LIFECYCLE_FILE=${run_file} }"
    bash -c "${marker_command}" </dev/null >/dev/null 2>&1 || true
    awk -v run="${i}" '{ t[$1] = $2 } END { printf "%d %d %d %d\n", run, t["command_complete"], t["shutdown_end"] - t["shutdown_begin"], t["process_exit_requested"] - t["command_complete"] }' "${run_file}" >>"${marker_file}"
  done
done

python3 - "${output_dir}" "${flush_ms}" <<'PY'
import json
import statistics
import sys

out, flush_ms = sys.argv[1], int(sys.argv[2])


def pct(values, q):
    values = sorted(values)
    index = (len(values) - 1) * q
    low = int(index)
    high = min(low + 1, len(values) - 1)
    return values[low] + (values[high] - values[low]) * (index - low)


def load(mode):
    with open(f"{out}/hyperfine-{mode}.json") as handle:
        return [t * 1000 for t in json.load(handle)["results"][0]["times"]]


disabled = load("disabled")
summary = {"flush_budget_ms": flush_ms, "disabled_ms": {"p50": pct(disabled, 0.5), "p95": pct(disabled, 0.95), "max": max(disabled)}, "modes": {}}
d50, d95 = pct(disabled, 0.5), pct(disabled, 0.95)
for mode in ("reachable", "blackhole", "refused"):
    times = load(mode)
    overhead = sorted(t - d50 for t in times)
    paired = [t - statistics.median(disabled) for t in times]
    rows = [list(map(int, line.split())) for line in open(f"{out}/markers-{mode}.txt") if line.strip()]
    post = [r[3] / 1e6 for r in rows]
    shutdown = [r[2] / 1e6 for r in rows]
    summary["modes"][mode] = {
        "runs": len(times),
        "end_to_end_ms": {"p50": pct(times, 0.5), "p95": pct(times, 0.95), "max": max(times)},
        "overhead_ms_vs_disabled_p50": {"p50": pct(paired, 0.5), "p95": pct(paired, 0.95), "max": max(paired)},
        "post_command_ms": {"p50": pct(post, 0.5), "p95": pct(post, 0.95), "max": max(post)} if post else None,
        "shutdown_ms": {"p50": pct(shutdown, 0.5), "p95": pct(shutdown, 0.95), "max": max(shutdown)} if shutdown else None,
        "marker_runs": len(rows),
    }
with open(f"{out}/summary.json", "w") as handle:
    json.dump(summary, handle, indent=2)
print(json.dumps(summary, indent=2))
PY
