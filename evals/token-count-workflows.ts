import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { getEncoding, type TiktokenEncoding } from "js-tiktoken";
import { execFileSync } from "node:child_process";

type Workflow = "plan" | "execute";
type ArtifactClass = "agent" | "command" | "skill" | "context_artifact";
type ScopeRule =
  | { type: "entire-file" }
  | { type: "canonical-body-subsection"; owner_path: string };

type Surface = {
  surface_id: string;
  workflow: Workflow;
  artifact_class: ArtifactClass;
  path: string;
  scope_rule: ScopeRule;
  conditional: boolean;
};

type Manifest = {
  manifest_version: string;
  plan_name: string;
  task_id: string;
  surfaces: Surface[];
};

type SurfaceRow = {
  surface_id: string;
  workflow: Workflow;
  artifact_class: ArtifactClass;
  path: string;
  scope_rule: ScopeRule;
  tokenizer: string;
  tokens: number;
  baseline_tokens: number | null;
  delta_tokens: number | null;
  conditional: boolean;
};

type Summary = {
  run_id: string;
  timestamp_utc: string;
  git_sha: string;
  plan_name: string;
  task_id: string;
  tokenizer: string;
  requested_tokenizer: string;
  resolved_tokenizer: string;
  manifest_path: string;
  baseline_path: string | null;
  plan_total_tokens: number;
  execute_total_tokens: number;
  combined_total_tokens: number;
  combined_delta_tokens: number | null;
  notes: string[];
};

type Report = {
  summary: Summary;
  surfaces: SurfaceRow[];
};

const SCRIPT_DIR = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = resolve(SCRIPT_DIR, "..");
const DEFAULT_MANIFEST_PATH = "context/sce/workflow-token-footprint-manifest.json";
const OUTPUT_DIR = "context/tmp/token-footprint";

function fail(message: string): never {
  throw new Error(message);
}

function normalizeNewlines(text: string): string {
  return text.replace(/\r\n?/g, "\n");
}

function parseArgs(argv: string[]) {
  let manifestPath = DEFAULT_MANIFEST_PATH;
  let baselinePath: string | null = null;
  let runId: string | null = null;
  let requestedTokenizer: TiktokenEncoding = "o200k_base";

  for (let index = 0; index < argv.length; index += 1) {
    const token = argv[index];
    if (!token) {
      continue;
    }
    if (token === "--manifest") {
      manifestPath = argv[index + 1] ?? fail("Missing value for --manifest");
      index += 1;
      continue;
    }
    if (token === "--baseline") {
      baselinePath = argv[index + 1] ?? fail("Missing value for --baseline");
      index += 1;
      continue;
    }
    if (token === "--run-id") {
      runId = argv[index + 1] ?? fail("Missing value for --run-id");
      index += 1;
      continue;
    }
    if (token === "--tokenizer") {
      const value = argv[index + 1] ?? fail("Missing value for --tokenizer");
      if (value !== "o200k_base" && value !== "cl100k_base") {
        fail(`Unsupported tokenizer '${value}'. Expected o200k_base or cl100k_base.`);
      }
      requestedTokenizer = value;
      index += 1;
      continue;
    }
    fail(`Unknown argument '${token}'.`);
  }

  return {
    manifestPath,
    baselinePath,
    runId,
    requestedTokenizer,
  };
}

async function readJsonFile<T>(absolutePath: string): Promise<T> {
  const raw = normalizeNewlines(await readFile(absolutePath, "utf8"));
  return JSON.parse(raw) as T;
}

function parseOwnerPath(ownerPath: string, surfaceId: string): string {
  const match = ownerPath.match(/^agents\["([^"]+)"\]\.canonicalBody$/);
  const agentSlug = match?.[1];
  if (!agentSlug) {
    fail(
      `surface_id=${surfaceId}: unsupported owner_path '${ownerPath}' for canonical-body-subsection extraction`,
    );
  }
  return agentSlug;
}

function escapeForRegex(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function extractCanonicalBodyPkl(
  normalizedText: string,
  ownerPath: string,
  surfaceId: string,
): string {
  const agentSlug = parseOwnerPath(ownerPath, surfaceId);
  const ownerPattern = new RegExp(
    String.raw`\["${escapeForRegex(agentSlug)}"\]\s*=\s*new\s+ContentUnit\s*\{`,
    "m",
  );
  const ownerMatch = ownerPattern.exec(normalizedText);
  if (!ownerMatch || ownerMatch.index < 0) {
    fail(`surface_id=${surfaceId}: owner_path '${ownerPath}' not found in source file`);
  }

  const ownerStart = ownerMatch.index;
  const canonicalBodyPattern = /canonicalBody\s*=\s*"""/m;
  const ownerScopedText = normalizedText.slice(ownerStart);
  const canonicalBodyMatch = canonicalBodyPattern.exec(ownerScopedText);
  if (!canonicalBodyMatch || canonicalBodyMatch.index < 0) {
    fail(`surface_id=${surfaceId}: owner_path '${ownerPath}' missing canonicalBody assignment`);
  }

  const openDelimiterAbsolute = ownerStart + canonicalBodyMatch.index + canonicalBodyMatch[0].length;
  const payloadStart =
    normalizedText.charAt(openDelimiterAbsolute) === "\n"
      ? openDelimiterAbsolute + 1
      : openDelimiterAbsolute;

  const payloadAndTail = normalizedText.slice(payloadStart);
  const closingPattern = /^\s*"""/m;
  const closingMatch = closingPattern.exec(payloadAndTail);
  if (!closingMatch || closingMatch.index < 0) {
    fail(`surface_id=${surfaceId}: owner_path '${ownerPath}' missing canonicalBody closing delimiter`);
  }

  return payloadAndTail.slice(0, closingMatch.index);
}

function extractByScopeRule(sourceText: string, surface: Surface): string {
  if (surface.scope_rule.type === "entire-file") {
    return sourceText;
  }

  if (surface.scope_rule.type === "canonical-body-subsection") {
    return extractCanonicalBodyPkl(sourceText, surface.scope_rule.owner_path, surface.surface_id);
  }

  fail(`surface_id=${surface.surface_id}: unsupported scope_rule type`);
}

function resolveTokenizer(requestedTokenizer: TiktokenEncoding): {
  encodingName: TiktokenEncoding;
  notes: string[];
} {
  try {
    getEncoding(requestedTokenizer);
    return { encodingName: requestedTokenizer, notes: [] };
  } catch (error) {
    if (requestedTokenizer !== "o200k_base") {
      throw error;
    }

    try {
      getEncoding("cl100k_base");
    } catch {
      fail("Tokenizer resolution failed: neither o200k_base nor cl100k_base is available");
    }

    return {
      encodingName: "cl100k_base",
      notes: [
        "Requested tokenizer o200k_base was unavailable; fallback cl100k_base was used.",
      ],
    };
  }
}

function stableStringify(value: unknown): string {
  return JSON.stringify(value, null, 2);
}

function getGitSha(): string {
  try {
    return execFileSync("git", ["rev-parse", "HEAD"], {
      cwd: REPO_ROOT,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "ignore"],
    }).trim();
  } catch {
    return "unknown";
  }
}

function requireManifest(manifest: Manifest): void {
  if (!manifest.manifest_version) {
    fail("Manifest missing required field: manifest_version");
  }
  if (!manifest.plan_name) {
    fail("Manifest missing required field: plan_name");
  }
  if (!manifest.task_id) {
    fail("Manifest missing required field: task_id");
  }
  if (!Array.isArray(manifest.surfaces) || manifest.surfaces.length === 0) {
    fail("Manifest must include a non-empty surfaces array");
  }
}

function readBaselineRows(baseline: Report): Map<string, number> {
  const map = new Map<string, number>();
  for (const row of baseline.surfaces) {
    map.set(row.surface_id, row.tokens);
  }
  return map;
}

function buildMarkdown(report: Report): string {
  const { summary, surfaces } = report;
  const lines: string[] = [
    `# Static token accounting run: ${summary.run_id}`,
    "",
    "## Run metadata",
    "",
    `- timestamp_utc: ${summary.timestamp_utc}`,
    `- git_sha: ${summary.git_sha}`,
    `- plan_name: ${summary.plan_name}`,
    `- task_id: ${summary.task_id}`,
    `- tokenizer: ${summary.tokenizer}`,
    `- requested_tokenizer: ${summary.requested_tokenizer}`,
    `- resolved_tokenizer: ${summary.resolved_tokenizer}`,
    `- manifest_path: ${summary.manifest_path}`,
    `- baseline_path: ${summary.baseline_path ?? "null"}`,
    "",
    "## Surface counts",
    "",
    "| surface_id | workflow | artifact_class | path | scope_rule | tokenizer | tokens | baseline_tokens | delta_tokens | conditional |",
    "| --- | --- | --- | --- | --- | --- | ---: | ---: | ---: | --- |",
  ];

  for (const row of surfaces) {
    const scopeRule = row.scope_rule.type;
    lines.push(
      `| ${row.surface_id} | ${row.workflow} | ${row.artifact_class} | ${row.path} | ${scopeRule} | ${row.tokenizer} | ${row.tokens} | ${row.baseline_tokens ?? "null"} | ${row.delta_tokens ?? "null"} | ${row.conditional} |`,
    );
  }

  lines.push(
    "",
    "## Totals",
    "",
    `- plan_total_tokens: ${summary.plan_total_tokens}`,
    `- execute_total_tokens: ${summary.execute_total_tokens}`,
    `- combined_total_tokens: ${summary.combined_total_tokens}`,
    `- combined_delta_tokens: ${summary.combined_delta_tokens ?? "null"}`,
    "",
    "## Notes",
    "",
  );

  if (summary.notes.length === 0) {
    lines.push("- none");
  } else {
    for (const note of summary.notes) {
      lines.push(`- ${note}`);
    }
  }

  lines.push("");
  return lines.join("\n");
}

async function main(): Promise<void> {
  const { manifestPath, baselinePath, runId, requestedTokenizer } = parseArgs(
    Bun.argv.slice(2),
  );

  const manifestAbsolutePath = resolve(REPO_ROOT, manifestPath);
  const manifest = await readJsonFile<Manifest>(manifestAbsolutePath);
  requireManifest(manifest);

  const { encodingName, notes: tokenizerNotes } = resolveTokenizer(requestedTokenizer);
  const encoding = getEncoding(encodingName);

  const timestampUtc = new Date().toISOString();
  const resolvedRunId = runId ?? "latest";
  const gitSha = getGitSha();

  let baselineRows = new Map<string, number>();
  let baselineSummaryTokenizer: string | null = null;
  let baselineAbsolutePath: string | null = null;

  if (baselinePath) {
    baselineAbsolutePath = resolve(REPO_ROOT, baselinePath);
    const baseline = await readJsonFile<Report>(baselineAbsolutePath);
    baselineRows = readBaselineRows(baseline);
    baselineSummaryTokenizer =
      baseline.summary.resolved_tokenizer ?? baseline.summary.tokenizer ?? null;
  }

  if (baselineSummaryTokenizer && baselineSummaryTokenizer !== encodingName) {
    fail(
      `Baseline tokenizer mismatch: baseline=${baselineSummaryTokenizer} current=${encodingName}`,
    );
  }

  const rows: SurfaceRow[] = [];
  let planTotal = 0;
  let executeTotal = 0;
  let baselineTotal = 0;
  let canComputeCombinedDelta = Boolean(baselinePath);

  for (const surface of manifest.surfaces) {
    const surfaceAbsolutePath = resolve(REPO_ROOT, surface.path);
    const sourceRaw = await readFile(surfaceAbsolutePath, "utf8");
    const sourceNormalized = normalizeNewlines(sourceRaw);
    const extractedPayload = extractByScopeRule(sourceNormalized, surface);
    const tokens = encoding.encode(extractedPayload).length;

    const baselineTokens = baselineRows.has(surface.surface_id)
      ? baselineRows.get(surface.surface_id) ?? null
      : null;
    const deltaTokens = baselineTokens === null ? null : tokens - baselineTokens;

    if (surface.workflow === "plan") {
      planTotal += tokens;
    }
    if (surface.workflow === "execute") {
      executeTotal += tokens;
    }

    if (baselineTokens === null) {
      canComputeCombinedDelta = false;
    } else {
      baselineTotal += baselineTokens;
    }

    rows.push({
      surface_id: surface.surface_id,
      workflow: surface.workflow,
      artifact_class: surface.artifact_class,
      path: surface.path,
      scope_rule: surface.scope_rule,
      tokenizer: encodingName,
      tokens,
      baseline_tokens: baselineTokens,
      delta_tokens: deltaTokens,
      conditional: surface.conditional,
    });
  }

  const combinedTotal = planTotal + executeTotal;
  const combinedDelta = canComputeCombinedDelta ? combinedTotal - baselineTotal : null;

  const summary: Summary = {
    run_id: resolvedRunId,
    timestamp_utc: timestampUtc,
    git_sha: gitSha,
    plan_name: manifest.plan_name,
    task_id: manifest.task_id,
    tokenizer: encodingName,
    requested_tokenizer: requestedTokenizer,
    resolved_tokenizer: encodingName,
    manifest_path: manifestPath,
    baseline_path: baselinePath,
    plan_total_tokens: planTotal,
    execute_total_tokens: executeTotal,
    combined_total_tokens: combinedTotal,
    combined_delta_tokens: combinedDelta,
    notes: tokenizerNotes,
  };

  const report: Report = {
    summary,
    surfaces: rows,
  };

  const outputDirectoryPath = resolve(REPO_ROOT, OUTPUT_DIR);
  await mkdir(outputDirectoryPath, { recursive: true });

  const latestJsonPath = resolve(outputDirectoryPath, "workflow-token-count-latest.json");
  const latestMarkdownPath = resolve(outputDirectoryPath, "workflow-token-count-latest.md");

  await writeFile(latestJsonPath, `${stableStringify(report)}\n`, "utf8");
  await writeFile(latestMarkdownPath, buildMarkdown(report), "utf8");

  if (runId) {
    const archiveJsonPath = resolve(outputDirectoryPath, `workflow-token-count-${runId}.json`);
    await writeFile(archiveJsonPath, `${stableStringify(report)}\n`, "utf8");
  }

  console.log(`Wrote ${latestJsonPath}`);
  console.log(`Wrote ${latestMarkdownPath}`);
  if (runId) {
    console.log(`Wrote ${resolve(outputDirectoryPath, `workflow-token-count-${runId}.json`)}`);
  }
}

await main();
