import fs from "node:fs/promises";
import path from "node:path";
import process from "node:process";
import { spawnSync } from "node:child_process";

const PROMPT_CAPTURE_DIRECTORY = path.join("sce");
const PROMPT_CAPTURE_FILE = "prompts.jsonl";

async function main() {
  const hookInput = await readJsonFromStdin();
  const promptEntry = buildPromptEntry(hookInput);
  if (!promptEntry) {
    return;
  }

  const gitDirectory = resolveGitDirectory(promptEntry.cwd);
  const outputDirectory = path.join(gitDirectory, PROMPT_CAPTURE_DIRECTORY);
  const outputPath = path.join(outputDirectory, PROMPT_CAPTURE_FILE);

  await fs.mkdir(outputDirectory, { recursive: true });
  await fs.appendFile(outputPath, `${JSON.stringify(promptEntry)}\n`, "utf8");
}

function buildPromptEntry(hookInput) {
  const prompt = firstNonEmptyString(
    process.env.USER_PROMPT,
    hookInput?.prompt,
    hookInput?.user_prompt,
    hookInput?.input,
  );
  if (!prompt) {
    return null;
  }

  return {
    session_id: firstNonEmptyString(
      process.env.SESSION_ID,
      process.env.CLAUDE_SESSION_ID,
      hookInput?.session_id,
      hookInput?.sessionId,
    ) || "unknown",
    prompt,
    cwd: firstNonEmptyString(
      process.env.CWD,
      process.env.CLAUDE_PROJECT_DIR,
      hookInput?.cwd,
    ) || process.cwd(),
    transcript_path: firstNonEmptyString(
      hookInput?.transcript_path,
      hookInput?.transcriptPath,
    ),
    timestamp: new Date().toISOString(),
  };
}

function resolveGitDirectory(cwd) {
  const gitDirResult = spawnSync("git", ["rev-parse", "--git-dir"], {
    cwd,
    encoding: "utf8",
  });

  if (gitDirResult.status !== 0) {
    throw new Error(gitDirResult.stderr.trim() || "Failed to resolve git directory for prompt capture.");
  }

  const gitDirectory = gitDirResult.stdout.trim();
  return path.resolve(cwd, gitDirectory);
}

function firstNonEmptyString(...values) {
  for (const value of values) {
    if (typeof value === "string" && value.length > 0) {
      return value;
    }
  }

  return null;
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
