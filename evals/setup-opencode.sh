#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUN_DIR="${RUN_DIR:-${SCRIPT_DIR}/.opencode-run}"
OPENCODE_SOURCE_DIR="${OPENCODE_SOURCE_DIR:-${SCRIPT_DIR}/../config/.opencode}"
OPENCODE_TARGET_DIR="${OPENCODE_TARGET_DIR:-${RUN_DIR}/.opencode}"
OPENCODE_BIN="${OPENCODE_BIN:-${SCRIPT_DIR}/node_modules/.bin/opencode}"
SERVER_PID_FILE="${SERVER_PID_FILE:-${SCRIPT_DIR}/.opencode-server.pid}"
SERVER_LOG_FILE="${SERVER_LOG_FILE:-${SCRIPT_DIR}/.opencode-server.log}"
SERVER_HOSTNAME="${SERVER_HOSTNAME:-127.0.0.1}"
SERVER_PORT="${SERVER_PORT:-0}"

usage() {
  cat <<EOF
Usage:
  $(basename "$0") setup
  $(basename "$0") start-server
  $(basename "$0") stop-server
  $(basename "$0") status
  $(basename "$0") up
  $(basename "$0") down

Environment overrides:
  RUN_DIR          Directory used as opencode server workspace (default: evals/.opencode-run)
  OPENCODE_SOURCE_DIR Source .opencode dir (default: config/.opencode)
  OPENCODE_TARGET_DIR Workspace target .opencode dir (default: <RUN_DIR>/.opencode)
  OPENCODE_BIN     opencode binary path (default: evals/node_modules/.bin/opencode)
  SERVER_PID_FILE  opencode server pid file path (default: evals/.opencode-server.pid)
  SERVER_LOG_FILE  opencode server log file path (default: evals/.opencode-server.log)
  SERVER_HOSTNAME  opencode server hostname (default: 127.0.0.1)
  SERVER_PORT      opencode server port (default: 0)
EOF
}

resolve_opencode_bin() {
  if [[ -x "${OPENCODE_BIN}" ]]; then
    printf "%s\n" "${OPENCODE_BIN}"
    return
  fi

  if command -v opencode >/dev/null 2>&1; then
    command -v opencode
    return
  fi

  echo "Could not find opencode binary. Set OPENCODE_BIN or install opencode." >&2
  exit 1
}

setup_workspace() {
  mkdir -p "${RUN_DIR}"

  if [[ -d "${OPENCODE_SOURCE_DIR}" ]]; then
    rm -rf "${OPENCODE_TARGET_DIR}"
    mkdir -p "${OPENCODE_TARGET_DIR}"
    cp -a "${OPENCODE_SOURCE_DIR}/." "${OPENCODE_TARGET_DIR}/"
    echo "Workspace ready at ${RUN_DIR}; synced ${OPENCODE_SOURCE_DIR} -> ${OPENCODE_TARGET_DIR}"
  else
    echo "Workspace ready at ${RUN_DIR}; skipped config sync (missing ${OPENCODE_SOURCE_DIR})"
  fi
}

is_server_running() {
  if [[ ! -f "${SERVER_PID_FILE}" ]]; then
    return 1
  fi

  local server_pid
  server_pid="$(cat "${SERVER_PID_FILE}")"
  [[ -n "${server_pid}" ]] && kill -0 "${server_pid}" >/dev/null 2>&1
}

find_server_pids() {
  if [[ "${SERVER_PORT}" == "0" ]]; then
    return
  fi

  if ! command -v pgrep >/dev/null 2>&1; then
    return
  fi

  pgrep -f "opencode serve --hostname ${SERVER_HOSTNAME} --port ${SERVER_PORT}" 2>/dev/null || true
}

kill_server_pids() {
  local pids
  pids="$(find_server_pids)"

  if [[ -z "${pids}" ]]; then
    return
  fi

  echo "Stopping existing opencode server on ${SERVER_HOSTNAME}:${SERVER_PORT}"
  for pid in ${pids}; do
    kill "${pid}" >/dev/null 2>&1 || true
  done

  sleep 0.3

  pids="$(find_server_pids)"
  if [[ -z "${pids}" ]]; then
    return
  fi

  for pid in ${pids}; do
    kill -9 "${pid}" >/dev/null 2>&1 || true
  done
}

start_server() {
  kill_server_pids

  if is_server_running; then
    echo "Server already running (pid $(cat "${SERVER_PID_FILE}"))"
    return
  fi

  setup_workspace

  local opencode_cmd
  opencode_cmd="$(resolve_opencode_bin)"

  echo "Starting opencode server in ${RUN_DIR}"
  local original_pwd
  original_pwd="$(pwd)"
  cd "${RUN_DIR}"
  "${opencode_cmd}" serve --hostname "${SERVER_HOSTNAME}" --port "${SERVER_PORT}" > "${SERVER_LOG_FILE}" 2>&1 &
  local server_pid=$!
  cd "${original_pwd}"
  printf "%s\n" "${server_pid}" > "${SERVER_PID_FILE}"

  sleep 0.3

  if command -v pgrep >/dev/null 2>&1; then
    local child_pid
    child_pid=""
    for pid in $(pgrep -P "${server_pid}" 2>/dev/null || true); do
      child_pid="${pid}"
      break
    done

    if [[ -n "${child_pid}" ]]; then
      server_pid="${child_pid}"
      printf "%s\n" "${server_pid}" > "${SERVER_PID_FILE}"
    fi
  fi

  if ! kill -0 "${server_pid}" >/dev/null 2>&1; then
    rm -f "${SERVER_PID_FILE}"
    echo "Failed to start opencode server. See ${SERVER_LOG_FILE}" >&2
    exit 1
  fi

  echo "Server started (pid ${server_pid}); logs: ${SERVER_LOG_FILE}"
}

stop_server() {
  kill_server_pids

  if [[ ! -f "${SERVER_PID_FILE}" ]]; then
    echo "Server is not running"
    return
  fi

  local server_pid
  server_pid="$(cat "${SERVER_PID_FILE}")"

  if [[ -z "${server_pid}" ]]; then
    rm -f "${SERVER_PID_FILE}"
    echo "Server PID file was empty; cleaned up"
    return
  fi

  if ! kill -0 "${server_pid}" >/dev/null 2>&1; then
    rm -f "${SERVER_PID_FILE}"
    echo "Server process not running; cleaned stale pid file"
    return
  fi

  echo "Stopping opencode server (pid ${server_pid})"
  kill "${server_pid}" || true

  local timeout_seconds=8
  local started_at
  started_at="$(date +%s)"

  while kill -0 "${server_pid}" >/dev/null 2>&1; do
    if (( $(date +%s) - started_at >= timeout_seconds )); then
      break
    fi
    sleep 0.1
  done

  if kill -0 "${server_pid}" >/dev/null 2>&1; then
    kill -9 "${server_pid}" || true
  fi

  rm -f "${SERVER_PID_FILE}"
  echo "Server stopped"
}

status_server() {
  if is_server_running; then
    echo "server running pid $(cat "${SERVER_PID_FILE}")"
  else
    echo "server stopped"
  fi

  if [[ -d "${RUN_DIR}" ]]; then
    echo "workspace ready ${RUN_DIR}"
  else
    echo "workspace missing ${RUN_DIR}"
  fi
}

up() {
  start_server
}

down() {
  stop_server

  if [[ -z "${RUN_DIR}" || "${RUN_DIR}" == "/" ]]; then
    echo "Refusing to delete unsafe RUN_DIR: '${RUN_DIR}'" >&2
    exit 1
  fi

  if [[ -d "${RUN_DIR}" ]]; then
    rm -rf "${RUN_DIR}"
    echo "Workspace deleted ${RUN_DIR}"
  else
    echo "Workspace already missing ${RUN_DIR}"
  fi
}

main() {
  local action="${1:-}"

  case "${action}" in
    setup)
      setup_workspace
      ;;
    start-server)
      start_server
      ;;
    stop-server)
      stop_server
      ;;
    status)
      status_server
      ;;
    up)
      up
      ;;
    down)
      down
      ;;
    -h|--help|help|"")
      usage
      ;;
    *)
      echo "Unknown command: ${action}" >&2
      usage
      exit 1
      ;;
  esac
}

main "$@"
