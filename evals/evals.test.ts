import { afterAll, afterEach, beforeEach, describe, expect, test } from "bun:test";
import { createOpencodeClient } from "@opencode-ai/sdk/v2";
import { mkdirSync, writeFileSync } from "node:fs";
import {
  buildTokenBreakdown,
  checkDirStructure,
  cleanupModelContext,
  createModelContext,
  getErrorMessage,
  getHeadCommitHash,
  getSelectedModels,
  runStep,
  setupModelContext,
  toFileSafeSegment,
  type RunStepResult,
  type TokenBreakdown,
} from "./test-setup";

const EVAL_TIMEOUT = 300_000;
const JSON_INDENT_SPACES = 2;
const MS_PER_SECOND = 1_000;
const RUNTIME_SECONDS_DECIMAL_PLACES = 2;

type EvalStepName =
  | "health-check"
  | "session-create"
  | "first-prompt"
  | "check-dir-structure";

type PromptName = "first";

type PromptMetrics = {
  prompt: PromptName;
  runtimeMs: number;
  messageID?: string;
  tokens: TokenBreakdown;
};

type EvalStepResult = RunStepResult<EvalStepName>;

type EvalFailure = {
  step: EvalStepName | "unknown";
  message: string;
  missingPaths?: string[];
};

type EvalResult = {
  model: {
    providerID: string;
    modelID: string;
    fullModel: string;
  };
  run: {
    headCommitHash: string;
    timestampMs: number;
    sessionID?: string;
    success: boolean;
    failure?: EvalFailure;
  };
  metrics: {
    totalRuntimeMs: number;
    runtimePerReplyMs: Record<PromptName, number>;
    totalTokensSpent: number;
    tokenSpentPerPrompt: Record<PromptName, number>;
  };
  prompts: PromptMetrics[];
  directoryCheck: {
    presentPaths: string[];
    missingPaths: string[];
  };
  steps: EvalStepResult[];
};

type ModelReportEntry = {
  fullModel: string;
  passed: boolean;
  totalRuntimeMs: number;
  totalTokensSpent: number;
};

const resultsDir = `${import.meta.dir}/.results`;
const headCommitHash = getHeadCommitHash();
const modelReportEntries: ModelReportEntry[] = [];

function writeEvalResultFile(result: EvalResult): string {
  mkdirSync(resultsDir, { recursive: true });

  const fileName = `${toFileSafeSegment(result.model.fullModel)}-${result.run.headCommitHash}-${result.run.timestampMs}.json`;
  const filePath = `${resultsDir}/${fileName}`;

  writeFileSync(
    filePath,
    `${JSON.stringify(result, null, JSON_INDENT_SPACES)}\n`,
    "utf8",
  );

  return filePath;
}

function writeModelReportFile(entries: ModelReportEntry[]): string {
  mkdirSync(resultsDir, { recursive: true });

  const fileName = `summary-${headCommitHash}-${Date.now()}.json`;
  const filePath = `${resultsDir}/${fileName}`;

  writeFileSync(
    filePath,
    `${JSON.stringify(entries, null, JSON_INDENT_SPACES)}\n`,
    "utf8",
  );

  return filePath;
}

const selectedModels = getSelectedModels();
const requiredContextPaths = [
  "context/overview.md",
  "context/architecture.md",
  "context/patterns.md",
  "context/glossary.md",
  "context/context-map.md",
  "context/plans",
  "context/handovers",
  "context/decisions",
  "context/tmp",
  "context/tmp/.gitignore",
];

describe("opencode sdk connectivity per model", () => {
  test("has at least one selected model", () => {
    expect(selectedModels.length).toBeGreaterThan(0);
  });

  for (const [index, model] of selectedModels.entries()) {
    const ctx = createModelContext(model, index);

    describe(ctx.fullModel, () => {
      let started = false;
      let activeClient: ReturnType<typeof createOpencodeClient> | undefined;
      let activeSessionID: string | undefined;
      let activeRequestAbortController: AbortController | undefined;
      let testAbortTimer: ReturnType<typeof setTimeout> | undefined;

      beforeEach(async () => {
        started = false;
        activeClient = undefined;
        activeSessionID = undefined;
        activeRequestAbortController = undefined;
        testAbortTimer = undefined;
        await setupModelContext(ctx);
        started = true;
      });

      afterEach(async () => {
        if (!started) {
          return;
        }

        if (testAbortTimer) {
          clearTimeout(testAbortTimer);
          testAbortTimer = undefined;
        }

        activeRequestAbortController?.abort(
          "Stopping in-flight request during test cleanup",
        );

        if (activeClient && activeSessionID) {
          try {
            await activeClient.session.abort({
              sessionID: activeSessionID,
            });
          } catch {
            // Ignore abort errors during cleanup.
          }
        }

        cleanupModelContext(ctx);
      });

      test("runs init flow with Shared Context", async () => {
        console.log(
          `Testing model ${ctx.fullModel} with provider ${ctx.providerID}`,
        );
        const client = createOpencodeClient({
          baseUrl: ctx.baseUrl,
          throwOnError: true,
        });
        activeClient = client;
        activeRequestAbortController = new AbortController();
        testAbortTimer = setTimeout(() => {
          activeRequestAbortController?.abort(
            "Eval hit internal request deadline",
          );
        }, EVAL_TIMEOUT);
        const startedAtMs = Date.now();
        const steps: EvalStepResult[] = [];
        const prompts: PromptMetrics[] = [];
        let presentPaths: string[] = [];
        let missingPaths: string[] = [];
        let sessionID: string | undefined;
        let thrownError: unknown;

        try {
          const health = await runStep(steps, "health-check", async () => {
            const response = await client.global.health({
              signal: activeRequestAbortController?.signal,
            });

            expect(response.data?.healthy).toBeTrue();

            return response;
          });

          expect(health.data?.healthy).toBeTrue();

          const session = await runStep(steps, "session-create", async () => {
            const response = await client.session.create(
              {
                directory: ctx.runDir,
                title: `init flow ${ctx.fullModel}`,
                permission: [
                  { permission: "read", action: "allow", pattern: "*" },
                  { permission: "glob", action: "allow", pattern: "*" },
                  { permission: "grep", action: "allow", pattern: "*" },
                  { permission: "list", action: "allow", pattern: "*" },
                  { permission: "edit", action: "allow", pattern: "*" },
                  { permission: "bash", action: "allow", pattern: "*" },
                ],
              },
              {
                signal: activeRequestAbortController?.signal,
              },
            );

            return response;
          });

          sessionID = session.data?.id;
          activeSessionID = sessionID;
          expect(typeof sessionID).toBe("string");

          const model = {
            providerID: ctx.providerID,
            modelID: ctx.modelID,
          };

          const firstReply = await runStep(steps, "first-prompt", async () => {
            const promptStartedAt = Date.now();
            const response = await client.session.prompt(
              {
                sessionID: sessionID!,
                model,
                agent: "Shared Context",
                parts: [
                  {
                    type: "text",
                    text: "Init context.",
                  },
                ],
              },
              {
                signal: activeRequestAbortController?.signal,
              },
            );
            const promptEndedAt = Date.now();

            const tokenBreakdown = buildTokenBreakdown(
              response.data?.info?.tokens,
            );
            prompts.push({
              prompt: "first",
              runtimeMs: promptEndedAt - promptStartedAt,
              messageID: response.data?.info?.id,
              tokens: tokenBreakdown,
            });

            if (response.data?.info?.error) {
              throw new Error(
                `First reply failed: ${JSON.stringify(response.data.info.error)}`,
              );
            }

            expect(response.data?.info?.agent).toBe("Shared Context");
            expect(typeof response.data?.info?.id).toBe("string");

            return response;
          });

          expect(firstReply.data?.info?.agent).toBe("Shared Context");

          await runStep(steps, "check-dir-structure", async () => {
            const dirStructure = checkDirStructure(
              ctx.runDir,
              requiredContextPaths,
            );

            presentPaths = dirStructure.presentPaths;
            missingPaths = dirStructure.missingPaths;

            expect(missingPaths).toHaveLength(0);
          });
        } catch (error) {
          thrownError = error;
        } finally {
          if (testAbortTimer) {
            clearTimeout(testAbortTimer);
            testAbortTimer = undefined;
          }

          const firstPrompt = prompts.find(
            (prompt) => prompt.prompt === "first",
          );
          const failedStep = [...steps]
            .reverse()
            .find((step) => step.status === "failed");
          const totalRuntimeMs = Date.now() - startedAtMs;
          const totalTokensSpent = prompts.reduce(
            (sum, prompt) => sum + prompt.tokens.total,
            0,
          );

          const result: EvalResult = {
            model: {
              providerID: ctx.providerID,
              modelID: ctx.modelID,
              fullModel: ctx.fullModel,
            },
            run: {
              headCommitHash,
              timestampMs: Date.now(),
              sessionID,
              success: !thrownError,
              failure: thrownError
                ? {
                    step: failedStep?.name ?? "unknown",
                    message: getErrorMessage(thrownError),
                    missingPaths:
                      missingPaths.length > 0 ? missingPaths : undefined,
                  }
                : undefined,
            },
            metrics: {
              totalRuntimeMs,
              runtimePerReplyMs: {
                first: firstPrompt?.runtimeMs ?? 0,
              },
              totalTokensSpent,
              tokenSpentPerPrompt: {
                first: firstPrompt?.tokens.total ?? 0,
              },
            },
            prompts,
            directoryCheck: {
              presentPaths,
              missingPaths,
            },
            steps,
          };

          modelReportEntries.push({
            fullModel: ctx.fullModel,
            passed: result.run.success,
            totalRuntimeMs: result.metrics.totalRuntimeMs,
            totalTokensSpent: result.metrics.totalTokensSpent,
          });

          const resultPath = writeEvalResultFile(result);
          console.log("Eval result written:", resultPath);
        }

        if (thrownError) {
          throw thrownError;
        }
      }, EVAL_TIMEOUT);
    });
  }

  afterAll(() => {
    if (modelReportEntries.length === 0) {
      console.log("Model report: no results collected");
      return;
    }

    const sortedEntries = [...modelReportEntries].sort((a, b) =>
      a.fullModel.localeCompare(b.fullModel),
    );
    const totalTokensUsed = sortedEntries.reduce(
      (sum, entry) => sum + entry.totalTokensSpent,
      0,
    );
    const summaryPath = writeModelReportFile(sortedEntries);

    console.log("Model report:");
    for (const entry of sortedEntries) {
      const runtimeSeconds = (
        entry.totalRuntimeMs / MS_PER_SECOND
      ).toFixed(RUNTIME_SECONDS_DECIMAL_PLACES);
      console.log(
        `- ${entry.fullModel}: ${entry.passed ? "PASSED" : "FAILED"} in ${runtimeSeconds}s, ${entry.totalTokensSpent} tokens`,
      );
    }
    console.log("Tokens per model:");
    for (const entry of sortedEntries) {
      console.log(`- ${entry.fullModel}: ${entry.totalTokensSpent}`);
    }
    console.log(`Total tokens used: ${totalTokensUsed}`);
    console.log("Model summary written:", summaryPath);
  });
});
