import { spawn } from "node:child_process";
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import type { Hooks, Plugin } from "@opencode-ai/plugin";

type OpenCodeEvent = Parameters<NonNullable<Hooks["event"]>>[0]["event"];

const REQUIRED_EVENTS = new Set(["message.part.updated"]);

const ALL_CAPTURED_EVENTS = REQUIRED_EVENTS;

type TraceInput = {
  event?: OpenCodeEvent;
};

type DiffTracePayload = {
  sessionID: string;
  diff: string;
  time: number;
};

function extractDiffTracePayload(
  input: TraceInput,
): DiffTracePayload | undefined {
  const event = input.event;
  if (event === undefined || event.type !== "message.part.updated") {
    return undefined;
  }

  const part = event.properties.part;
  if (part.type !== "tool") {
    return undefined;
  }

  const state = part.state;
  if (state.status !== "completed") {
    return undefined;
  }

  const diff = state.metadata.diff;
  if (typeof diff !== "string" || diff.trim().length === 0) {
    return undefined;
  }

  return {
    sessionID: part.sessionID,
    diff,
    time: state.time.end,
  };
}

function formatTimestamp(date: Date): string {
  return date.toISOString().replace(/[:.]/g, "-");
}

function buildTraceFileName(traceName: string, date: Date): string {
  return `${formatTimestamp(date)}-${traceName}.json`;
}

function getTraceName(input: TraceInput): string {
  if (typeof input !== "object" || input === null) {
    return "unknown";
  }

  if (
    typeof input.event === "object" &&
    input.event !== null &&
    typeof input.event.type === "string" &&
    input.event.type.length > 0
  ) {
    return input.event.type;
  }

  return "unknown";
}

function shouldCaptureEvent(eventType: string): boolean {
  return ALL_CAPTURED_EVENTS.has(eventType);
}

async function buildTrace(
  traceDirectory: string,
  repoRoot: string,
  input: TraceInput,
): Promise<void> {
  const now = new Date();
  const filePath = path.join(
    traceDirectory,
    buildTraceFileName(getTraceName(input), now),
  );
  const body = JSON.stringify({ input }, null, 2);
  const diffTracePayload = extractDiffTracePayload(input);

  await mkdir(traceDirectory, { recursive: true });
  await writeFile(filePath, body, "utf8");

  if (diffTracePayload === undefined) {
    return;
  }

  await runDiffTraceHook(repoRoot, diffTracePayload);
}

async function runDiffTraceHook(
  repoRoot: string,
  payload: DiffTracePayload,
): Promise<void> {
  await new Promise<void>((resolve, reject) => {
    const child = spawn("sce", ["hooks", "diff-trace"], {
      cwd: repoRoot,
      stdio: ["pipe", "ignore", "inherit"],
    });

    child.on("error", reject);

    child.on("close", (code, signal) => {
      if (code === 0) {
        resolve();
        return;
      }

      const reason =
        signal === null ? `exit code ${String(code)}` : `signal ${signal}`;
      reject(
        new Error(`Command 'sce hooks diff-trace' failed with ${reason}.`),
      );
    });

    child.stdin.end(`${JSON.stringify(payload)}\n`);
  });
}

export const SceAgentTracePlugin: Plugin = async ({ directory, worktree }) => {
  const repoRoot = worktree ?? directory ?? process.cwd();
  const traceDirectory = path.join(repoRoot, "context", "tmp");

  return {
    event: async (input) => {
      const eventType =
        typeof input.event === "object" &&
        input.event !== null &&
        typeof input.event.type === "string"
          ? input.event.type
          : undefined;

      if (eventType === undefined || !shouldCaptureEvent(eventType)) {
        return;
      }

      await buildTrace(traceDirectory, repoRoot, input);
    },
  };
};
