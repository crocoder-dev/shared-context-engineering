import path from "node:path";
import { fileURLToPath } from "node:url";

import {
  evaluateBashCommandPolicy,
  formatPolicyBlockMessage,
} from "../lib/bash-policy-runtime.js";

export const SceBashPolicyPlugin = async ({ directory, worktree }) => {
  const pluginDirectory = path.dirname(fileURLToPath(import.meta.url));
  const repoRoot = worktree || directory || process.cwd();

  return {
    "tool.execute.before": async (input, output) => {
      if (input.tool !== "bash") {
        return;
      }

      const command = output?.args?.command;
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
