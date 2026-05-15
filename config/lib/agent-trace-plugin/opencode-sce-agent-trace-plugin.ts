import { spawn } from "node:child_process";
import type { Hooks, Plugin } from "@opencode-ai/plugin";

type OpenCodeEvent = Parameters<NonNullable<Hooks["event"]>>[0]["event"];

const REQUIRED_EVENTS = new Set(["message.updated"]);

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
  if (event === undefined || event.type !== "message.updated") {
    return undefined;
  }

  const properties = event.properties;
  if (typeof properties !== "object" || properties === null) {
    return undefined;
  }

  const propertiesObj = properties;

  // Access properties.info (the Message object)
  const info = propertiesObj.info;
  if (typeof info !== "object" || info === null) {
    return undefined;
  }

  const infoObj = info;

  // Only capture user messages (filter out assistant, system, etc.)
  if (infoObj.role !== "user") {
    return undefined;
  }

  const sessionID =
    typeof infoObj.sessionID === "string" &&
    infoObj.sessionID.trim().length > 0
      ? infoObj.sessionID
      : "unknown";

  // Access info.summary?.diffs via explicit checks
  const summary = infoObj.summary;
  const diffEntries =
    typeof summary === "object" && summary !== null
      ? (summary).diffs
      : undefined;

  if (!Array.isArray(diffEntries) || diffEntries.length === 0) {
    return undefined;
  }

  const patches: string[] = [];
  for (const entry of diffEntries) {
    if (typeof entry !== "object" || entry === null) {
      continue;
    }
    const entryObj = entry as {patch?:string};
    const patch = entryObj.patch || "";
  
    patches.push(patch);  
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
