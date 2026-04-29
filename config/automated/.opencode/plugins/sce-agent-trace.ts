import { spawn } from "node:child_process";
import type { Hooks, Plugin } from "@opencode-ai/plugin";

type OpenCodeEvent = Parameters<NonNullable<Hooks["event"]>>[0]["event"];

const REQUIRED_EVENTS = new Set(["session.diff"]);

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
  if (event === undefined || event.type !== "session.diff") {
    return undefined;
  }

  const properties = event.properties;
  if (typeof properties !== "object" || properties === null) {
    return undefined;
  }

  const propertiesObj = properties as Record<string, unknown>;

  const sessionID =
    typeof propertiesObj.sessionID === "string" &&
    propertiesObj.sessionID.trim().length > 0
      ? propertiesObj.sessionID
      : "unknown";

  const diffEntries = propertiesObj.diff;
  if (!Array.isArray(diffEntries) || diffEntries.length === 0) {
    return undefined;
  }

  const patches: string[] = [];
  for (const entry of diffEntries) {
    if (typeof entry !== "object" || entry === null) {
      continue;
    }
    const entryObj = entry as Record<string, unknown>;
    const patch =
      typeof entryObj.patch === "string"
        ? entryObj.patch
        : typeof entryObj.diff === "string"
          ? entryObj.diff
          : undefined;
    if (patch !== undefined && patch.trim().length > 0) {
      patches.push(patch);
    }
  }

  if (patches.length === 0) {
    return undefined;
  }

  return {
    sessionID,
    diff: patches.join("\n"),
    time: Date.now(),
  };
}

function shouldCaptureEvent(eventType: string): boolean {
  return ALL_CAPTURED_EVENTS.has(eventType);
}

async function buildTrace(repoRoot: string, input: TraceInput): Promise<void> {
  const diffTracePayload = extractDiffTracePayload(input);

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

      await buildTrace(repoRoot, input);
    },
  };
};
