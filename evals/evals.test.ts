import { afterAll, afterEach, beforeEach, describe, expect, test } from "bun:test";
import { createOpencodeClient, type AssistantMessage } from "@opencode-ai/sdk/v2";
import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
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
const DEFAULT_QUESTION_ANSWER = "yes";
const FIRST_PROMPT_POLL_INTERVAL_MS = 250;

type EvalStepName =
  | "health-check"
  | "session-create"
  | "first-prompt"
  | "change-to-plan"
  | "check-plan-file"
  | "check-dir-structure";

type PromptName = "first" | "change-to-plan";

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

type FirstPromptOutcome = {
  messageID: string;
  agent: string;
  tokens: {
    total?: number;
    input: number;
    output: number;
    reasoning: number;
    cache: {
      read: number;
      write: number;
    };
  };
};

type ChangeToPlanOutcome = {
  messageID: string;
  agent: string;
  tokens: {
    total?: number;
    input: number;
    output: number;
    reasoning: number;
    cache: {
      read: number;
      write: number;
    };
  };
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

function ensureBootstrappedContext(runDir: string): void {
  const contextDir = `${runDir}/context`;
  const plansDir = `${contextDir}/plans`;
  const handoversDir = `${contextDir}/handovers`;
  const decisionsDir = `${contextDir}/decisions`;
  const tmpDir = `${contextDir}/tmp`;

  mkdirSync(plansDir, { recursive: true });
  mkdirSync(handoversDir, { recursive: true });
  mkdirSync(decisionsDir, { recursive: true });
  mkdirSync(tmpDir, { recursive: true });

  const fileSeeds: Array<{ path: string; contents: string }> = [
    { path: `${contextDir}/overview.md`, contents: "# Overview\n\n" },
    { path: `${contextDir}/architecture.md`, contents: "# Architecture\n\n" },
    { path: `${contextDir}/patterns.md`, contents: "# Patterns\n\n" },
    { path: `${contextDir}/glossary.md`, contents: "# Glossary\n\n" },
    {
      path: `${contextDir}/context-map.md`,
      contents:
        "# Context Map\n\n- [Overview](./overview.md)\n- [Architecture](./architecture.md)\n- [Patterns](./patterns.md)\n- [Glossary](./glossary.md)\n- [Plans](./plans/)\n- [Handovers](./handovers/)\n- [Decisions](./decisions/)\n- [Tmp](./tmp/)\n",
    },
    { path: `${tmpDir}/.gitignore`, contents: "*\n!.gitignore\n" },
  ];

  for (const seed of fileSeeds) {
    if (!existsSync(seed.path)) {
      writeFileSync(seed.path, seed.contents, "utf8");
    }
  }
}

async function runFirstPromptWithQuestionHandling(params: {
  client: ReturnType<typeof createOpencodeClient>;
  sessionID: string;
  runDir: string;
  model: {
    providerID: string;
    modelID: string;
  };
  signal?: AbortSignal;
}): Promise<FirstPromptOutcome> {
  const { client, sessionID, runDir, model, signal } = params;
  let promptMessageID: string | undefined;
  let currentPromptText = "init context";
  let currentPromptStartedAt = Date.now();

  await client.session.promptAsync(
    {
      sessionID,
      directory: runDir,
      model,
      agent: "Shared Context Plan",
      parts: [
        {
          type: "text",
          text: currentPromptText,
        },
      ],
    },
    {
      signal,
    },
  );

  let answeredQuestion = false;

  const isApprovalQuestion = (text: string): boolean => {
    const normalized = text.toLowerCase();

    return (
      normalized.includes("need your approval") ||
      normalized.includes("if you approve") ||
      normalized.includes("do you approve") ||
      normalized.includes("recommended default") ||
      normalized.includes("approve") ||
      normalized.includes("?")
    );
  };

  while (true) {
    if (signal?.aborted) {
      throw new Error("First prompt aborted while waiting for completion");
    }

    const questions = await client.question.list(
      {
        directory: runDir,
      },
      {
        signal,
      },
    );

    const pendingSessionQuestions =
      questions.data?.filter((request) => request.sessionID === sessionID) ?? [];

    if (pendingSessionQuestions.length > 1) {
      throw new Error("Encountered multiple question requests during first prompt");
    }

    const pendingQuestion = pendingSessionQuestions[0];
    if (pendingQuestion) {
      if (answeredQuestion || pendingQuestion.questions.length !== 1) {
        throw new Error("Encountered multiple question requests during first prompt");
      }

      await client.question.reply(
        {
          requestID: pendingQuestion.id,
          directory: runDir,
          answers: [[DEFAULT_QUESTION_ANSWER]],
        },
        {
          signal,
        },
      );

      answeredQuestion = true;
    }

    const messageHistory = await client.session.messages(
      {
        sessionID,
        directory: runDir,
      },
      {
        signal,
      },
    );

    if (!promptMessageID) {
      const latestPromptMessage = (messageHistory.data ?? [])
        .filter((entry) => entry.info.role === "user" && entry.info.time.created >= currentPromptStartedAt)
        .filter((entry) =>
          entry.parts.some(
            (part) => part.type === "text" && "text" in part && part.text.trim() === currentPromptText,
          ),
        )
        .sort((left, right) => right.info.time.created - left.info.time.created)[0];

      if (latestPromptMessage) {
        promptMessageID = latestPromptMessage.info.id;
      }
    }

    if (!promptMessageID) {
      await new Promise((resolve) => setTimeout(resolve, FIRST_PROMPT_POLL_INTERVAL_MS));
      continue;
    }

    const assistantForPrompt = (messageHistory.data ?? [])
      .filter((entry) => entry.info.role === "assistant" && entry.info.parentID === promptMessageID)
      .sort((left, right) => right.info.time.created - left.info.time.created)[0];

    if (assistantForPrompt) {
      const assistantInfo = assistantForPrompt.info as AssistantMessage;

      if (assistantInfo.error) {
        throw new Error(`First reply failed: ${JSON.stringify(assistantInfo.error)}`);
      }

      if (!assistantInfo.time.completed) {
        await new Promise((resolve) => setTimeout(resolve, FIRST_PROMPT_POLL_INTERVAL_MS));
        continue;
      }

      const assistantText = assistantForPrompt.parts
        .filter((part) => part.type === "text" && "text" in part)
        .map((part) => part.text)
        .join("\n");

      if (isApprovalQuestion(assistantText)) {
        if (answeredQuestion) {
          throw new Error("Encountered multiple question requests during first prompt");
        }

        answeredQuestion = true;
        promptMessageID = undefined;
        currentPromptText = DEFAULT_QUESTION_ANSWER;
        currentPromptStartedAt = Date.now();

        await client.session.promptAsync(
          {
            sessionID,
            directory: runDir,
            model,
            agent: "Shared Context Plan",
            parts: [
              {
                type: "text",
                text: currentPromptText,
              },
            ],
          },
          {
            signal,
          },
        );

        continue;
      }

      return {
        messageID: assistantInfo.id,
        agent: assistantInfo.agent,
        tokens: assistantInfo.tokens,
      };
    }

    await new Promise((resolve) => setTimeout(resolve, FIRST_PROMPT_POLL_INTERVAL_MS));
  }
}

async function runChangeToPlanCommandWithQuestionHandling(params: {
  client: ReturnType<typeof createOpencodeClient>;
  sessionID: string;
  runDir: string;
  model: {
    providerID: string;
    modelID: string;
  };
  signal?: AbortSignal;
}): Promise<ChangeToPlanOutcome> {
  const { client, sessionID, runDir, model, signal } = params;
  let promptMessageID: string | undefined;
  const commandPrompt =
    `/change-to-plan Create a backend TypeScript server with a single hello-world endpoint and add a flake.nix that bootstraps the dev environment using Bun.`;
  const commandStartedAt = Date.now();

  await client.session.promptAsync(
    {
      sessionID,
      directory: runDir,
      model,
      parts: [
        {
          type: "text",
          text: commandPrompt,
        },
      ],
    },
    {
      signal,
    },
  );

  let answeredQuestion = false;

  while (true) {
    if (signal?.aborted) {
      throw new Error("change-to-plan prompt aborted while waiting for completion");
    }

    const questions = await client.question.list(
      {
        directory: runDir,
      },
      {
        signal,
      },
    );

    const pendingSessionQuestions =
      questions.data?.filter((request) => request.sessionID === sessionID) ?? [];

    if (pendingSessionQuestions.length > 1) {
      throw new Error("Encountered multiple question requests during change-to-plan prompt");
    }

    const pendingQuestion = pendingSessionQuestions[0];
    if (pendingQuestion) {
      if (answeredQuestion || pendingQuestion.questions.length !== 1) {
        throw new Error("Encountered multiple question requests during change-to-plan prompt");
      }

      await client.question.reply(
        {
          requestID: pendingQuestion.id,
          directory: runDir,
          answers: [[DEFAULT_QUESTION_ANSWER]],
        },
        {
          signal,
        },
      );

      answeredQuestion = true;
    }

    const messageHistory = await client.session.messages(
      {
        sessionID,
        directory: runDir,
      },
      {
        signal,
      },
    );

    if (!promptMessageID) {
      const latestPromptMessage = (messageHistory.data ?? [])
        .filter((entry) => entry.info.role === "user" && entry.info.time.created >= commandStartedAt)
        .filter((entry) =>
          entry.parts.some(
            (part) => part.type === "text" && "text" in part && part.text.trim() === commandPrompt,
          ),
        )
        .sort((left, right) => right.info.time.created - left.info.time.created)[0];

      if (latestPromptMessage) {
        promptMessageID = latestPromptMessage.info.id;
      }
    }

    if (!promptMessageID) {
      await new Promise((resolve) => setTimeout(resolve, FIRST_PROMPT_POLL_INTERVAL_MS));
      continue;
    }

    const assistantForPrompt = (messageHistory.data ?? [])
      .filter((entry) => entry.info.role === "assistant" && entry.info.parentID === promptMessageID)
      .sort((left, right) => right.info.time.created - left.info.time.created)[0];

    if (assistantForPrompt) {
      const assistantInfo = assistantForPrompt.info as AssistantMessage;

      if (assistantInfo.error) {
        throw new Error(`change-to-plan reply failed: ${JSON.stringify(assistantInfo.error)}`);
      }

      if (!assistantInfo.time.completed) {
        await new Promise((resolve) => setTimeout(resolve, FIRST_PROMPT_POLL_INTERVAL_MS));
        continue;
      }

      return {
        messageID: assistantInfo.id,
        agent: assistantInfo.agent,
        tokens: assistantInfo.tokens,
      };
    }

    await new Promise((resolve) => setTimeout(resolve, FIRST_PROMPT_POLL_INTERVAL_MS));
  }
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

      async function runModelEval(
        mode: "init-context" | "change-to-plan",
      ): Promise<void> {
        console.log(
          `Testing model ${ctx.fullModel} (${mode}) with provider ${ctx.providerID}`,
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
                title: `${mode} flow ${ctx.fullModel}`,
                permission: [
                  { permission: "read", action: "allow", pattern: "*" },
                  { permission: "glob", action: "allow", pattern: "*" },
                  { permission: "grep", action: "allow", pattern: "*" },
                  { permission: "list", action: "allow", pattern: "*" },
                  { permission: "edit", action: "allow", pattern: "*" },
                  { permission: "bash", action: "allow", pattern: "*" },
                  { permission: "question", action: "allow", pattern: "*" },
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

          if (mode === "init-context") {
            const firstReply = await runStep(steps, "first-prompt", async () => {
              const promptStartedAt = Date.now();
              const outcome = await runFirstPromptWithQuestionHandling({
                client,
                sessionID: sessionID!,
                runDir: ctx.runDir,
                model,
                signal: activeRequestAbortController?.signal,
              });
              const promptEndedAt = Date.now();

              const tokenBreakdown = buildTokenBreakdown(outcome.tokens);
              prompts.push({
                prompt: "first",
                runtimeMs: promptEndedAt - promptStartedAt,
                messageID: outcome.messageID,
                tokens: tokenBreakdown,
              });

              expect(["Shared Context Plan", "build"]).toContain(outcome.agent);
              expect(typeof outcome.messageID).toBe("string");

              return {
                data: {
                  info: {
                    agent: outcome.agent,
                  },
                },
              };
            });

            expect(firstReply.data?.info?.agent).toBe("Shared Context Plan");
          }

          if (mode === "change-to-plan") {
            ensureBootstrappedContext(ctx.runDir);

            const changeToPlanReply = await runStep(steps, "change-to-plan", async () => {
              const promptStartedAt = Date.now();
              const outcome = await runChangeToPlanCommandWithQuestionHandling({
                client,
                sessionID: sessionID!,
                runDir: ctx.runDir,
                model,
                signal: activeRequestAbortController?.signal,
              });
              const promptEndedAt = Date.now();

              const tokenBreakdown = buildTokenBreakdown(outcome.tokens);
              prompts.push({
                prompt: "change-to-plan",
                runtimeMs: promptEndedAt - promptStartedAt,
                messageID: outcome.messageID,
                tokens: tokenBreakdown,
              });

              expect(["Shared Context Plan", "build"]).toContain(outcome.agent);
              expect(typeof outcome.messageID).toBe("string");

              return {
                data: {
                  info: {
                    agent: outcome.agent,
                  },
                },
              };
            });

            expect(["Shared Context Plan", "build"]).toContain(
              changeToPlanReply.data?.info?.agent,
            );

            await runStep(steps, "check-plan-file", async () => {
              const plansDir = `${ctx.runDir}/context/plans`;
              const planFiles = readdirSync(plansDir)
                .filter((name) => name.endsWith(".md"))
                .sort();

              expect(planFiles.length).toBe(1);

              const latestPlanFilePath = `${plansDir}/${planFiles[planFiles.length - 1]}`;
              const planText = readFileSync(latestPlanFilePath, "utf8");

              expect(planText.toLowerCase()).toContain("success criteria");
              expect(planText).toContain("T01");
            });
          }

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
          const changeToPlanPrompt = prompts.find(
            (prompt) => prompt.prompt === "change-to-plan",
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
              fullModel: `${ctx.fullModel}-${mode}`,
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
                "change-to-plan": changeToPlanPrompt?.runtimeMs ?? 0,
              },
              totalTokensSpent,
              tokenSpentPerPrompt: {
                first: firstPrompt?.tokens.total ?? 0,
                "change-to-plan": changeToPlanPrompt?.tokens.total ?? 0,
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
            fullModel: result.model.fullModel,
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
      }

      test("runs context bootstrap flow with Shared Context", async () => {
        await runModelEval("init-context");
      }, EVAL_TIMEOUT);

      test("runs change-to-plan flow with Shared Context", async () => {
        await runModelEval("change-to-plan");
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
