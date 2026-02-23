import { existsSync, mkdirSync } from "node:fs";
import { createOpencodeClient } from "@opencode-ai/sdk/v2";

export type SelectedModel = {
  providerID: string;
  modelID: string;
};

export type ModelTestContext = {
  providerID: string;
  modelID: string;
  fullModel: string;
  modelEnv: NodeJS.ProcessEnv;
  baseUrl: string;
  runDir: string;
};

export type TokenBreakdown = {
  total: number;
  input: number;
  output: number;
  reasoning: number;
  cacheRead: number;
  cacheWrite: number;
};

export type RunStepStatus = "success" | "failed";

export type RunStepResult<TStepName extends string> = {
  name: TStepName;
  status: RunStepStatus;
  startedAtMs: number;
  endedAtMs: number;
  durationMs: number;
  error?: string;
};

const scriptDir = import.meta.dir;
const selectModelsScript = `${scriptDir}/select-models.sh`;
const setupOpencodeScript = `${scriptDir}/setup-opencode.sh`;
const runsRootDir = `${scriptDir}/model-runs`;
const serverHostname = "127.0.0.1";
const baseServerPort = 6200;

function runAndCapture(command: string[], env: NodeJS.ProcessEnv = process.env): string {
  const result = Bun.spawnSync(command, {
    cwd: scriptDir,
    env,
    stdout: "pipe",
    stderr: "inherit",
  });

  if (result.exitCode !== 0) {
    throw new Error(`Command failed (${result.exitCode}): ${command.join(" ")}`);
  }

  return new TextDecoder().decode(result.stdout).trim();
}

function runWithLogs(command: string[], env: NodeJS.ProcessEnv = process.env): void {
  const result = Bun.spawnSync(command, {
    cwd: scriptDir,
    env,
    stdout: "inherit",
    stderr: "inherit",
  });

  if (result.exitCode !== 0) {
    throw new Error(`Command failed (${result.exitCode}): ${command.join(" ")}`);
  }
}

function toSlug(model: string): string {
  return model.replace(/[^a-zA-Z0-9._-]+/g, "-");
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

export function getSelectedModels(): SelectedModel[] {
  const raw = runAndCapture(["bash", selectModelsScript]);

  try {
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) {
      throw new Error("Expected an array");
    }

    return parsed.map((entry) => {
      if (
        !entry ||
        typeof entry !== "object" ||
        typeof entry.providerID !== "string" ||
        typeof entry.modelID !== "string"
      ) {
        throw new Error("Invalid model entry shape");
      }

      return {
        providerID: entry.providerID,
        modelID: entry.modelID,
      };
    });
  } catch (error) {
    throw new Error(`Failed to parse model list from ${selectModelsScript}: ${error}`);
  }
}

export function createModelContext(model: SelectedModel, index: number): ModelTestContext {
  const fullModel = `${model.providerID}/${model.modelID}`;
  const modelSlug = toSlug(fullModel);
  const runDir = `${runsRootDir}/${modelSlug}`;
  const serverLogFile = `${runDir}/opencode-server.log`;
  const serverPidFile = `${runDir}/opencode-server.pid`;
  const serverPort = String(baseServerPort + index);

  mkdirSync(runDir, { recursive: true });

  return {
    providerID: model.providerID,
    modelID: model.modelID,
    fullModel,
    baseUrl: `http://${serverHostname}:${serverPort}`,
    runDir,
    modelEnv: {
      ...process.env,
      OPENCODE_MODEL: fullModel,
      RUN_DIR: runDir,
      SERVER_PID_FILE: serverPidFile,
      SERVER_LOG_FILE: serverLogFile,
      SERVER_HOSTNAME: serverHostname,
      SERVER_PORT: serverPort,
    },
  };
}

export async function setupModelContext(ctx: ModelTestContext): Promise<void> {
  runWithLogs(["bash", setupOpencodeScript, "up"], ctx.modelEnv);
  await waitForSdkConnection(ctx.baseUrl);
}

export function cleanupModelContext(ctx: ModelTestContext): void {
  runWithLogs(["bash", setupOpencodeScript, "down"], ctx.modelEnv);
}

export function checkDirStructure(
  workspaceDir: string,
  requiredContextPaths: string[],
): {
  presentPaths: string[];
  missingPaths: string[];
} {
  const presentPaths = requiredContextPaths.filter(
    (relativePath) => existsSync(`${workspaceDir}/${relativePath}`),
  );
  const missingPaths = requiredContextPaths.filter(
    (relativePath) => !presentPaths.includes(relativePath),
  );

  return { presentPaths, missingPaths };
}

export function getHeadCommitHash(cwd = `${scriptDir}/..`): string {
  const result = Bun.spawnSync(["git", "rev-parse", "--short", "HEAD"], {
    cwd,
    stdout: "pipe",
    stderr: "pipe",
  });

  if (result.exitCode !== 0) {
    return "unknown-commit";
  }

  return new TextDecoder().decode(result.stdout).trim() || "unknown-commit";
}

export function toFileSafeSegment(value: string): string {
  return value.replace(/[^a-zA-Z0-9._-]+/g, "-");
}

export function getErrorMessage(error: unknown): string {
  if (error instanceof Error) {
    return error.message;
  }

  return String(error);
}

export function buildTokenBreakdown(
  tokens:
    | {
        total?: number;
        input: number;
        output: number;
        reasoning: number;
        cache: {
          read: number;
          write: number;
        };
      }
    | undefined,
): TokenBreakdown {
  const input = tokens?.input ?? 0;
  const output = tokens?.output ?? 0;
  const reasoning = tokens?.reasoning ?? 0;
  const cacheRead = tokens?.cache.read ?? 0;
  const cacheWrite = tokens?.cache.write ?? 0;
  const total =
    tokens?.total ?? input + output + reasoning + cacheRead + cacheWrite;

  return {
    total,
    input,
    output,
    reasoning,
    cacheRead,
    cacheWrite,
  };
}

export async function runStep<T, TStepName extends string>(
  steps: RunStepResult<TStepName>[],
  name: TStepName,
  fn: () => Promise<T>,
): Promise<T> {
  const startedAtMs = Date.now();

  try {
    const output = await fn();
    const endedAtMs = Date.now();

    steps.push({
      name,
      status: "success",
      startedAtMs,
      endedAtMs,
      durationMs: endedAtMs - startedAtMs,
    });

    return output;
  } catch (error) {
    const endedAtMs = Date.now();

    steps.push({
      name,
      status: "failed",
      startedAtMs,
      endedAtMs,
      durationMs: endedAtMs - startedAtMs,
      error: getErrorMessage(error),
    });

    throw error;
  }
}

async function waitForSdkConnection(baseUrl: string, timeoutMs = 10_000): Promise<void> {
  const startedAt = Date.now();
  const client = createOpencodeClient({
    baseUrl,
    throwOnError: true,
  });

  while (Date.now() - startedAt < timeoutMs) {
    try {
      const health = await client.global.health();
      if (health.data?.healthy) {
        return;
      }
    } catch {
      // Keep retrying until timeout.
    }

    await sleep(250);
  }

  throw new Error(`Timed out waiting for opencode sdk connection: ${baseUrl}`);
}
