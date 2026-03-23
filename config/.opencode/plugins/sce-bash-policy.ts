import path from "node:path";
import { fileURLToPath } from "node:url";
import type { Plugin } from "@opencode-ai/plugin";

import {
  evaluateBashCommandPolicy,
  formatPolicyBlockMessage,
} from "./bash-policy/runtime.ts";

export const SceBashPolicyPlugin: Plugin = async ({ directory, worktree }) => {
  const pluginDirectory = path.dirname(fileURLToPath(import.meta.url));
  const repoRoot = worktree ?? directory ?? process.cwd();

  return {
    "tool.execute.before": async (input, output) => {
      if (input.tool !== "bash") {
        return;
      }

      const args = output?.args;
      if (args === undefined || args === null) {
        return;
      }

      const command = (args as { command?: unknown }).command;
      if (typeof command !== "string" || command.length === 0) {
        return;
      }

      const result = await evaluateBashCommandPolicy({
        command,
        worktree: repoRoot,
        pluginDirectory,
      });
      if (result.allowed) {
        return;
      }

      throw new Error(formatPolicyBlockMessage(result.policy));
    },
  };
};
