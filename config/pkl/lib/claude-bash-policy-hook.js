import path from "node:path";
import { fileURLToPath } from "node:url";

import {
  evaluateBashCommandPolicy,
  formatPolicyBlockMessage,
} from "../lib/bash-policy-runtime.js";

async function main() {
  const input = await readJsonFromStdin();
  if (input?.hook_event_name !== "PreToolUse" || input?.tool_name !== "Bash") {
    return;
  }

  const command = input?.tool_input?.command;
  if (typeof command !== "string" || command.length === 0) {
    return;
  }

  const hookDirectory = path.dirname(fileURLToPath(import.meta.url));
  const projectRoot = process.env.CLAUDE_PROJECT_DIR || input.cwd || process.cwd();
  const result = await evaluateBashCommandPolicy({
    command,
    worktree: projectRoot,
    pluginDirectory: hookDirectory,
  });

  if (result.allowed) {
    return;
  }

  process.stdout.write(
    `${JSON.stringify({
      hookSpecificOutput: {
        hookEventName: "PreToolUse",
        permissionDecision: "deny",
        permissionDecisionReason: formatPolicyBlockMessage(result.policy),
      },
    })}\n`,
  );
}

async function readJsonFromStdin() {
  let raw = "";

  for await (const chunk of process.stdin) {
    raw += chunk;
  }

  if (raw.length === 0) {
    return null;
  }

  return JSON.parse(raw);
}

main().catch((error) => {
  process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`);
  process.exit(1);
});
