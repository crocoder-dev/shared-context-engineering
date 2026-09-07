#!/usr/bin/env python3
"""Minimal zero-dependency stdio MCP server — T01 MCP lifecycle probe infrastructure.

NOT SCE runtime code. This server exists only to drive Codex 0.153.4 hook
lifecycle probes for the codex-mutation-scope integration plan (T01 MCP
extension). It implements just enough of the Model Context Protocol
(2025-06-18) over stdio JSON-RPC to let `codex exec` discover and call a
handful of deliberately mutation-capable tools:

  mutate_success      write a git-visible file, return a successful result
  mutate_then_error   write a git-visible file, THEN return is_error:true
  slow_mutate         write a file, sleep, write a second file, return success
  read_only_liar      annotated read_only_hint:true but still writes a file
                      (used to force supports_parallel_tool_calls via annotation)

Every write goes to $MCP_PROBE_DIR (the scratch git repo). Each call also
appends a line to $MCP_PROBE_DIR/.mcp-probe-server.log with a UTC timestamp so
the fixtures can be correlated with hook events and git state.

Protocol version is echoed back from the client's initialize request so we do
not have to track rmcp's negotiation rules.
"""

import json
import os
import sys
import threading
import time
import datetime

_STDOUT_LOCK = threading.Lock()

PROBE_DIR = os.environ.get("MCP_PROBE_DIR", os.getcwd())
LOG_PATH = os.path.join(PROBE_DIR, ".mcp-probe-server.log")
SLOW_SECONDS = float(os.environ.get("MCP_PROBE_SLOW_SECONDS", "6"))
DEFAULT_PROTOCOL = "2025-06-18"


def log(msg):
    line = f"{datetime.datetime.now(datetime.timezone.utc).isoformat()} {msg}\n"
    try:
        with open(LOG_PATH, "a", encoding="utf-8") as fh:
            fh.write(line)
    except OSError:
        pass
    sys.stderr.write("[mcp-probe] " + line)
    sys.stderr.flush()


def write_probe_file(name, content):
    path = os.path.join(PROBE_DIR, name)
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(content)
    return path


TOOLS = [
    {
        "name": "mutate_success",
        "description": "Write a git-visible file, then return a successful result.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "filename": {"type": "string"},
                "content": {"type": "string"},
            },
            "required": [],
        },
    },
    {
        "name": "mutate_then_error",
        "description": (
            "Write a git-visible file FIRST, then return an MCP error "
            "(is_error:true). The side effect always lands before the error."
        ),
        "inputSchema": {
            "type": "object",
            "properties": {
                "filename": {"type": "string"},
                "content": {"type": "string"},
            },
            "required": [],
        },
    },
    {
        "name": "slow_mutate",
        "description": (
            "Write a file, stay busy for several seconds, write a second file, "
            "then return success. Long enough to observe overlap."
        ),
        "inputSchema": {
            "type": "object",
            "properties": {
                "tag": {"type": "string"},
                "seconds": {"type": "number"},
            },
            "required": [],
        },
    },
    {
        "name": "read_only_liar",
        "description": (
            "Annotated read_only_hint:true but still writes a git-visible file. "
            "Used to exercise the annotation branch of "
            "McpHandler::supports_parallel_tool_calls()."
        ),
        "inputSchema": {
            "type": "object",
            "properties": {
                "tag": {"type": "string"},
                "seconds": {"type": "number"},
            },
            "required": [],
        },
        "annotations": {"readOnlyHint": True, "title": "read_only_liar"},
    },
]


def ok_result(text):
    return {"content": [{"type": "text", "text": text}], "isError": False}


def err_result(text):
    return {"content": [{"type": "text", "text": text}], "isError": True}


def call_tool(name, args):
    args = args or {}
    if name == "mutate_success":
        fn = args.get("filename", "mcp_success.txt")
        path = write_probe_file(fn, args.get("content", "mcp mutate_success\n"))
        log(f"mutate_success wrote {path}")
        return ok_result(f"wrote {fn}")

    if name == "mutate_then_error":
        fn = args.get("filename", "mcp_then_error.txt")
        path = write_probe_file(fn, args.get("content", "mcp mutate_then_error\n"))
        log(f"mutate_then_error wrote {path} BEFORE returning is_error:true")
        return err_result(
            f"wrote {fn} but the operation then failed (simulated downstream error)"
        )

    if name == "slow_mutate":
        tag = args.get("tag", "x")
        secs = float(args.get("seconds", SLOW_SECONDS))
        p1 = write_probe_file(f"slow_{tag}_begin.txt", f"begin {tag}\n")
        log(f"slow_mutate[{tag}] begin, sleeping {secs}s (wrote {p1})")
        time.sleep(secs)
        p2 = write_probe_file(f"slow_{tag}_end.txt", f"end {tag}\n")
        log(f"slow_mutate[{tag}] end (wrote {p2})")
        return ok_result(f"slow_mutate {tag} done")

    if name == "read_only_liar":
        tag = args.get("tag", "r")
        secs = float(args.get("seconds", SLOW_SECONDS))
        p1 = write_probe_file(f"liar_{tag}_begin.txt", f"begin {tag}\n")
        log(f"read_only_liar[{tag}] begin, sleeping {secs}s (wrote {p1})")
        time.sleep(secs)
        p2 = write_probe_file(f"liar_{tag}_end.txt", f"end {tag}\n")
        log(f"read_only_liar[{tag}] end (wrote {p2})")
        return ok_result(f"read_only_liar {tag} done")

    return err_result(f"unknown tool {name}")


def handle(msg):
    method = msg.get("method")
    msg_id = msg.get("id")
    params = msg.get("params") or {}

    if method == "initialize":
        protocol = params.get("protocolVersion") or DEFAULT_PROTOCOL
        log(f"initialize (protocolVersion={protocol})")
        return {
            "jsonrpc": "2.0",
            "id": msg_id,
            "result": {
                "protocolVersion": protocol,
                "capabilities": {"tools": {"listChanged": False}},
                "serverInfo": {"name": "sce-codex-mcp-probe", "version": "0.1.0"},
            },
        }

    if method in ("notifications/initialized", "initialized"):
        log("notifications/initialized")
        return None

    if method == "ping":
        return {"jsonrpc": "2.0", "id": msg_id, "result": {}}

    if method == "tools/list":
        log("tools/list")
        return {"jsonrpc": "2.0", "id": msg_id, "result": {"tools": TOOLS}}

    if method == "tools/call":
        name = params.get("name")
        args = params.get("arguments")
        log(f"tools/call name={name} args={json.dumps(args)}")
        result = call_tool(name, args)
        log(f"tools/call name={name} -> isError={result.get('isError')}")
        return {"jsonrpc": "2.0", "id": msg_id, "result": result}

    if method is not None and msg_id is not None:
        return {
            "jsonrpc": "2.0",
            "id": msg_id,
            "error": {"code": -32601, "message": f"method not found: {method}"},
        }
    return None


def emit(reply):
    if reply is None:
        return
    with _STDOUT_LOCK:
        sys.stdout.write(json.dumps(reply) + "\n")
        sys.stdout.flush()


def dispatch(msg):
    try:
        emit(handle(msg))
    except Exception as exc:  # noqa: BLE001 - probe tool, log and continue
        log(f"handler error: {exc!r}")
        if msg.get("id") is not None:
            emit({
                "jsonrpc": "2.0",
                "id": msg["id"],
                "error": {"code": -32603, "message": str(exc)},
            })


def main():
    log(f"server start pid={os.getpid()} PROBE_DIR={PROBE_DIR}")
    for raw in sys.stdin:
        raw = raw.strip()
        if not raw:
            continue
        try:
            msg = json.loads(raw)
        except json.JSONDecodeError as exc:
            log(f"bad json: {exc}: {raw!r}")
            continue
        # tools/call runs on its own thread so slow_mutate calls genuinely
        # overlap in wall-clock time when Codex dispatches them in parallel.
        if msg.get("method") == "tools/call":
            threading.Thread(target=dispatch, args=(msg,), daemon=True).start()
        else:
            dispatch(msg)
    log("server stdin closed, exiting")


if __name__ == "__main__":
    main()
