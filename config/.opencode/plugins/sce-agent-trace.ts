import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import type { Plugin } from "@opencode-ai/plugin";

const REQUIRED_EVENTS = new Set([
  "session.diff",
  "message.updated",
  "message.part.updated",
]);

const ALL_CAPTURED_EVENTS = REQUIRED_EVENTS;

type TraceInput = {
  event?: {
    type?: unknown;
  };
};

function formatTimestamp(date: Date): string {
  return date.toISOString().replace(/[:.]/g, "-");
}

function buildTraceFileName(traceName: string, date: Date): string {
  return `${formatTimestamp(date)}-${traceName}.json`;
}

function getTraceName(input: unknown): string {
  if (typeof input !== "object" || input === null) {
    return "unknown";
  }

  const traceInput = input as TraceInput;

  if (
    typeof traceInput.event === "object" &&
    traceInput.event !== null &&
    typeof traceInput.event.type === "string" &&
    traceInput.event.type.length > 0
  ) {
    return traceInput.event.type;
  }

  return "unknown";
}

function shouldCaptureEvent(eventType: string): boolean {
  return ALL_CAPTURED_EVENTS.has(eventType);
}

async function buildTrace(traceDirectory: string, input: unknown): Promise<void> {
  const now = new Date();
  const filePath = path.join(traceDirectory, buildTraceFileName(getTraceName(input), now));
  const body = JSON.stringify({ input }, null, 2);

  await mkdir(traceDirectory, { recursive: true });
  await writeFile(filePath, body, "utf8");
}

export const SceAgentTracePlugin: Plugin = async ({ directory, worktree }) => {
  const repoRoot = worktree ?? directory ?? process.cwd();
  const traceDirectory = path.join(repoRoot, "context", "tmp");

  return {
    event: async (input) => {
      const traceInput = input as TraceInput;
      const eventType =
        typeof traceInput.event === "object" &&
        traceInput.event !== null &&
        typeof traceInput.event.type === "string"
          ? traceInput.event.type
          : undefined;

      if (eventType === undefined || !shouldCaptureEvent(eventType)) {
        return;
      }

      await buildTrace(traceDirectory, input);
    },
  };
};
