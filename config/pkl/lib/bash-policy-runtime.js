import path from "node:path";
import os from "node:os";
import { promises as fs } from "node:fs";

const ENV_ASSIGNMENT_PATTERN = /^[A-Za-z_][A-Za-z0-9_]*=.*/;
const WRAPPER_BINARIES = new Set(["env", "/usr/bin/env", "command", "nohup", "sudo"]);
const NO_MATCH = {
  allowed: true,
};

let cachedPresetCatalogPromise;

export async function evaluateBashCommandPolicy({
  command,
  worktree,
  pluginDirectory,
}) {
  const normalizedArgv = tokenizeAndNormalizeCommand(command);
  if (!normalizedArgv) {
    return NO_MATCH;
  }

  const policyConfig = await loadResolvedBashPolicyConfig({ worktree });
  if (!policyConfig) {
    return NO_MATCH;
  }

  const presetCatalog = await loadPresetCatalog(pluginDirectory);
  const activePolicies = buildActivePolicies(policyConfig, presetCatalog);
  const match = selectMatchingPolicy(activePolicies, normalizedArgv);
  if (!match) {
    return {
      allowed: true,
      normalizedArgv,
    };
  }

  return {
    allowed: false,
    normalizedArgv,
    policy: match,
  };
}

export function formatPolicyBlockMessage(match) {
  return `Blocked by SCE bash-tool policy '${match.id}': ${match.message}`;
}

async function loadResolvedBashPolicyConfig({ worktree }) {
  const configPaths = getConfigSearchPaths(worktree);
  let resolved = null;

  for (const configPath of configPaths) {
    const parsed = await readBashPolicyConfig(configPath);
    if (!parsed) {
      continue;
    }

    if (parsed.presets) {
      resolved = resolved ?? {};
      resolved.presets = parsed.presets;
    }
    if (parsed.custom) {
      resolved = resolved ?? {};
      resolved.custom = parsed.custom;
    }
  }

  if (!resolved) {
    return null;
  }

  const presets = resolved.presets ?? [];
  const custom = resolved.custom ?? [];
  if (presets.length === 0 && custom.length === 0) {
    return null;
  }

  return { presets, custom };
}

function getConfigSearchPaths(worktree) {
  const searchPaths = [];
  const globalConfigRoot = resolveGlobalConfigRoot();
  if (globalConfigRoot) {
    searchPaths.push(path.join(globalConfigRoot, "sce", "config.json"));
  }
  searchPaths.push(path.join(worktree, ".sce", "config.json"));
  return searchPaths;
}

function resolveGlobalConfigRoot() {
  const platform = process.platform;
  if (platform === "linux") {
    const xdgStateHome = process.env.XDG_STATE_HOME;
    if (xdgStateHome) {
      return xdgStateHome;
    }
    const home = os.homedir();
    return home ? path.join(home, ".local", "state") : null;
  }

  if (platform === "darwin") {
    const home = os.homedir();
    return home ? path.join(home, "Library", "Application Support") : null;
  }

  if (platform === "win32") {
    return process.env.APPDATA ?? null;
  }

  const xdgStateHome = process.env.XDG_STATE_HOME;
  if (xdgStateHome) {
    return xdgStateHome;
  }

  const xdgDataHome = process.env.XDG_DATA_HOME;
  if (xdgDataHome) {
    return xdgDataHome;
  }

  const home = os.homedir();
  return home ? path.join(home, ".local", "state") : null;
}

async function readBashPolicyConfig(configPath) {
  let raw;
  try {
    raw = await fs.readFile(configPath, "utf8");
  } catch (error) {
    if (error && error.code === "ENOENT") {
      return null;
    }
    return null;
  }

  let parsed;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return null;
  }

  return extractBashPolicyConfig(parsed);
}

function extractBashPolicyConfig(parsed) {
  if (!isPlainObject(parsed)) {
    return null;
  }

  const policies = parsed.policies;
  if (!isPlainObject(policies)) {
    return null;
  }

  const bash = policies.bash;
  if (!isPlainObject(bash)) {
    return null;
  }

  const presets = Array.isArray(bash.presets)
    ? bash.presets.filter((value) => typeof value === "string")
    : undefined;
  const custom = Array.isArray(bash.custom)
    ? bash.custom
        .map(parseCustomPolicy)
        .filter((value) => value !== null)
    : undefined;

  return {
    presets,
    custom,
  };
}

function parseCustomPolicy(value) {
  if (!isPlainObject(value) || !isPlainObject(value.match)) {
    return null;
  }

  const argvPrefix = value.match.argv_prefix;
  if (
    typeof value.id !== "string" ||
    value.id.length === 0 ||
    typeof value.message !== "string" ||
    value.message.length === 0 ||
    !Array.isArray(argvPrefix) ||
    argvPrefix.some((token) => typeof token !== "string" || token.length === 0)
  ) {
    return null;
  }

  return {
    id: value.id,
    message: value.message,
    argvPrefix,
    source: "custom",
    order: 0,
  };
}

async function loadPresetCatalog(pluginDirectory) {
  if (!cachedPresetCatalogPromise) {
    const presetCatalogPath = path.resolve(pluginDirectory, "../lib/bash-policy-presets.json");
    cachedPresetCatalogPromise = fs
      .readFile(presetCatalogPath, "utf8")
      .then((raw) => JSON.parse(raw))
      .catch(() => ({ presets: [] }));
  }

  return cachedPresetCatalogPromise;
}

function buildActivePolicies(policyConfig, presetCatalog) {
  const presetOrder = new Map();
  const presetPolicies = [];

  for (const [index, preset] of presetCatalog.presets.entries()) {
    presetOrder.set(preset.id, index);
  }

  for (const presetId of policyConfig.presets) {
    const presetIndex = presetOrder.get(presetId);
    if (presetIndex === undefined) {
      continue;
    }

    const preset = presetCatalog.presets[presetIndex];
    for (const argvPrefix of preset.match.argv_prefixes) {
      presetPolicies.push({
        id: preset.id,
        message: preset.message,
        argvPrefix,
        source: "preset",
        order: presetIndex,
      });
    }
  }

  const customPolicies = policyConfig.custom.map((policy, index) => ({
    ...policy,
    source: "custom",
    order: index,
  }));

  return [...presetPolicies, ...customPolicies];
}

function selectMatchingPolicy(activePolicies, normalizedArgv) {
  let bestMatch = null;

  for (const policy of activePolicies) {
    if (!argvStartsWith(normalizedArgv, policy.argvPrefix)) {
      continue;
    }

    if (!bestMatch || comparePolicyPriority(policy, bestMatch) < 0) {
      bestMatch = policy;
    }
  }

  return bestMatch;
}

function comparePolicyPriority(left, right) {
  if (left.argvPrefix.length !== right.argvPrefix.length) {
    return right.argvPrefix.length - left.argvPrefix.length;
  }

  if (left.source !== right.source) {
    return left.source === "custom" ? -1 : 1;
  }

  return left.order - right.order;
}

function argvStartsWith(argv, prefix) {
  if (prefix.length > argv.length) {
    return false;
  }

  return prefix.every((token, index) => argv[index] === token);
}

function tokenizeAndNormalizeCommand(command) {
  const tokenized = tokenizeShellCommand(command);
  if (!tokenized || tokenized.length === 0) {
    return null;
  }

  const normalized = [...tokenized];
  dropLeadingEnvAssignments(normalized);

  while (normalized.length > 0) {
    const executable = normalized[0];
    if (!WRAPPER_BINARIES.has(executable)) {
      break;
    }

    normalized.shift();
    dropLeadingEnvAssignments(normalized);
  }

  if (normalized.length === 0) {
    return null;
  }

  normalized[0] = path.basename(normalized[0]);
  return normalized;
}

function dropLeadingEnvAssignments(argv) {
  while (argv.length > 0 && ENV_ASSIGNMENT_PATTERN.test(argv[0])) {
    argv.shift();
  }
}

function tokenizeShellCommand(command) {
  const tokens = [];
  let current = "";
  let quote = null;
  let escaping = false;

  for (const character of command) {
    if (escaping) {
      current += character;
      escaping = false;
      continue;
    }

    if (character === "\\" && quote !== "'") {
      escaping = true;
      continue;
    }

    if (quote) {
      if (character === quote) {
        quote = null;
      } else {
        current += character;
      }
      continue;
    }

    if (character === '"' || character === "'") {
      quote = character;
      continue;
    }

    if (/\s/.test(character)) {
      if (current.length > 0) {
        tokens.push(current);
        current = "";
      }
      continue;
    }

    current += character;
  }

  if (escaping || quote) {
    return null;
  }

  if (current.length > 0) {
    tokens.push(current);
  }

  return tokens;
}

function isPlainObject(value) {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}
