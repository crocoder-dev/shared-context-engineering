import { spawn } from "node:child_process";
import type { Hooks, Plugin } from "@opencode-ai/plugin";

type OpenCodeEvent = Parameters<NonNullable<Hooks["event"]>>[0]["event"];
type ChatMessageInput = Parameters<NonNullable<Hooks["chat.message"]>>[0];
type ChatParamsInput = Parameters<NonNullable<Hooks["chat.params"]>>[0];

const REQUIRED_EVENTS = new Set(["session.diff"]);

const ALL_CAPTURED_EVENTS = REQUIRED_EVENTS;

type TraceInput = {
  event?: OpenCodeEvent;
};

type DiffTracePayload = {
  sessionID: string;
  diff: string;
  time: number;
  model_id?: string;
};

function getObject(value: unknown): Record<string, unknown> | undefined {
  return typeof value === "object" && value !== null
    ? (value as Record<string, unknown>)
    : undefined;
}

function getNonEmptyString(value: unknown): string | undefined {
  if (typeof value !== "string") {
    return undefined;
  }

  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : undefined;
}

function formatModelId(
  providerID: unknown,
  modelID: unknown,
): string | undefined {
  const provider = getNonEmptyString(providerID);
  const model = getNonEmptyString(modelID);

  return provider !== undefined && model !== undefined
    ? `${provider}/${model}`
    : undefined;
}

function extractModelIdFromChatMessageInput(
  input: ChatMessageInput,
): string | undefined {
  return formatModelId(input.model?.providerID, input.model?.modelID);
}

function extractModelIdFromChatParamsInput(
  input: ChatParamsInput,
): string | undefined {
  const providerInfo = getObject(input.provider.info);
  const modelInfo = getObject(input.model);

  const providerID =
    providerInfo === undefined
      ? undefined
      : (providerInfo.id ?? providerInfo.providerID);
  const modelID =
    modelInfo === undefined ? undefined : (modelInfo.id ?? modelInfo.modelID);

  return formatModelId(providerID, modelID);
}

function rememberSessionModelId(
  modelIdsBySessionID: Map<string, string>,
  sessionID: string,
  modelID: string | undefined,
): void {
  const normalizedSessionID = getNonEmptyString(sessionID);

  if (normalizedSessionID === undefined || modelID === undefined) {
    return;
  }

  modelIdsBySessionID.set(normalizedSessionID, modelID);
}

function extractDiffTracePayload(
  input: TraceInput,
  modelIdsBySessionID: ReadonlyMap<string, string> = new Map(),
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

  const payload: DiffTracePayload = {
    sessionID,
    diff: patches.join("\n"),
    time: Date.now(),
  };

  const modelID = modelIdsBySessionID.get(sessionID);
  if (modelID !== undefined) {
    payload.model_id = modelID;
  }

  return payload;
}

function shouldCaptureEvent(eventType: string): boolean {
  return ALL_CAPTURED_EVENTS.has(eventType);
}

async function buildTrace(
  repoRoot: string,
  input: TraceInput,
  modelIdsBySessionID: ReadonlyMap<string, string>,
): Promise<void> {
  const diffTracePayload = extractDiffTracePayload(input, modelIdsBySessionID);

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
  const modelIdsBySessionID = new Map<string, string>();

  return {
    "chat.message": async (input) => {
      rememberSessionModelId(
        modelIdsBySessionID,
        input.sessionID,
        extractModelIdFromChatMessageInput(input),
      );
    },
    "chat.params": async (input) => {
      rememberSessionModelId(
        modelIdsBySessionID,
        input.sessionID,
        extractModelIdFromChatParamsInput(input),
      );
    },
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

      await buildTrace(repoRoot, input, modelIdsBySessionID);
    },
  };
};
