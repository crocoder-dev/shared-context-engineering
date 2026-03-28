import { promises as fs } from "node:fs";
import os from "node:os";
import path from "node:path";

const ENV_ASSIGNMENT_PATTERN = /^[A-Za-z_][A-Za-z0-9_]*=.*/;
const WRAPPER_BINARIES = new Set([
	"env",
	"/usr/bin/env",
	"command",
	"nohup",
	"sudo",
]);

interface PolicyMatch {
	id: string;
	message: string;
	argvPrefix: string[];
	source: "preset" | "custom";
	order: number;
}

interface PolicyConfig {
	presets: string[];
	custom: PolicyMatch[];
}

interface PresetCatalog {
	presets: Array<{
		id: string;
		message: string;
		match: {
			argv_prefixes: string[][];
		};
	}>;
}

interface PolicyResultAllowed {
	allowed: true;
	normalizedArgv?: string[];
}

interface PolicyResultBlocked {
	allowed: false;
	normalizedArgv: string[];
	policy: PolicyMatch;
}

type PolicyResult = PolicyResultAllowed | PolicyResultBlocked;

const NO_MATCH: PolicyResultAllowed = {
	allowed: true,
};

let cachedPresetCatalogPromise: Promise<PresetCatalog> | undefined;

export async function evaluateBashCommandPolicy({
	command,
	worktree,
	pluginDirectory,
	presetCatalogPath,
}: {
	command: string;
	worktree: string;
	pluginDirectory: string;
	presetCatalogPath?: string;
}): Promise<PolicyResult> {
	const segments = parseCommandSegments(command);
	if (!segments || segments.length === 0) {
		return NO_MATCH;
	}

	const policyConfig = await loadResolvedBashPolicyConfig({ worktree });
	if (!policyConfig) {
		return NO_MATCH;
	}

	const presetCatalog = await loadPresetCatalog(
		pluginDirectory,
		presetCatalogPath,
	);
	const activePolicies = buildActivePolicies(policyConfig, presetCatalog);

	for (const segment of segments) {
		const normalizedArgv = normalizeSegment(segment);
		if (!normalizedArgv || normalizedArgv.length === 0) {
			continue;
		}

		const match = selectMatchingPolicy(activePolicies, normalizedArgv);
		if (match) {
			return {
				allowed: false,
				normalizedArgv,
				policy: match,
			};
		}
	}

	return {
		allowed: true,
	};
}

function normalizeSegment(segment: string[]): string[] | null {
	if (segment.length === 0) {
		return null;
	}

	const normalized = [...segment];
	dropLeadingEnvAssignments(normalized);

	while (normalized.length > 0) {
		const executable = normalized[0];
		if (executable === undefined || !WRAPPER_BINARIES.has(executable)) {
			break;
		}

		normalized.shift();
		dropLeadingEnvAssignments(normalized);
	}

	if (normalized.length === 0) {
		return null;
	}

	normalized[0] = path.basename(normalized[0] ?? "");
	return normalized;
}

export function formatPolicyBlockMessage(match: PolicyMatch): string {
	return `Blocked by SCE bash-tool policy '${match.id}': ${match.message}`;
}

async function loadResolvedBashPolicyConfig({
	worktree,
}: {
	worktree: string;
}): Promise<PolicyConfig | null> {
	const configPaths = getConfigSearchPaths(worktree);
	let resolved: { presets?: string[]; custom?: PolicyMatch[] } | null = null;

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

function getConfigSearchPaths(worktree: string): string[] {
	const searchPaths: string[] = [];
	const globalConfigRoot = resolveGlobalConfigRoot();
	if (globalConfigRoot) {
		searchPaths.push(path.join(globalConfigRoot, "sce", "config.json"));
	}
	searchPaths.push(path.join(worktree, ".sce", "config.json"));
	return searchPaths;
}

function resolveGlobalConfigRoot(): string | null {
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

async function readBashPolicyConfig(
	configPath: string,
): Promise<{ presets?: string[]; custom?: PolicyMatch[] } | null> {
	let raw: string;
	try {
		raw = await fs.readFile(configPath, "utf8");
	} catch (error: unknown) {
		if (
			error &&
			typeof error === "object" &&
			"code" in error &&
			error.code === "ENOENT"
		) {
			return null;
		}
		return null;
	}

	let parsed: unknown;
	try {
		parsed = JSON.parse(raw);
	} catch {
		return null;
	}

	return extractBashPolicyConfig(parsed);
}

function extractBashPolicyConfig(
	parsed: unknown,
): { presets?: string[]; custom?: PolicyMatch[] } | null {
	if (!isPlainObject(parsed)) {
		return null;
	}

	const policies = (parsed as Record<string, unknown>).policies;
	if (!isPlainObject(policies)) {
		return null;
	}

	const bash = (policies as Record<string, unknown>).bash;
	if (!isPlainObject(bash)) {
		return null;
	}

	const bashObj = bash as Record<string, unknown>;
	const presets = Array.isArray(bashObj.presets)
		? bashObj.presets.filter(
				(value: unknown): value is string => typeof value === "string",
			)
		: undefined;
	const custom = Array.isArray(bashObj.custom)
		? bashObj.custom
				.map(parseCustomPolicy)
				.filter(
					(value: PolicyMatch | null): value is PolicyMatch => value !== null,
				)
		: undefined;

	return {
		presets,
		custom,
	};
}

function parseCustomPolicy(value: unknown): PolicyMatch | null {
	if (!isPlainObject(value)) {
		return null;
	}
	const obj = value as Record<string, unknown>;
	if (!isPlainObject(obj.match)) {
		return null;
	}

	const argvPrefix = (obj.match as Record<string, unknown>).argv_prefix;
	if (
		typeof obj.id !== "string" ||
		obj.id.length === 0 ||
		typeof obj.message !== "string" ||
		obj.message.length === 0 ||
		!Array.isArray(argvPrefix) ||
		argvPrefix.length === 0 ||
		argvPrefix.some(
			(token: unknown) =>
				typeof token !== "string" || (token as string).length === 0,
		)
	) {
		return null;
	}

	return {
		id: obj.id,
		message: obj.message,
		argvPrefix: argvPrefix as string[],
		source: "custom",
		order: 0,
	};
}

async function loadPresetCatalog(
	pluginDirectory: string,
	presetCatalogPathOverride?: string,
): Promise<PresetCatalog> {
	if (presetCatalogPathOverride) {
		return fs
			.readFile(presetCatalogPathOverride, "utf8")
			.then((raw) => {
				const parsed: unknown = JSON.parse(raw);
				if (
					!isPlainObject(parsed) ||
					!Array.isArray((parsed as Record<string, unknown>).presets)
				) {
					return { presets: [] };
				}
				return parsed as unknown as PresetCatalog;
			})
			.catch((): PresetCatalog => ({ presets: [] }));
	}

	if (!cachedPresetCatalogPromise) {
		const presetCatalogPath = path.resolve(
			pluginDirectory,
			"../lib/bash-policy-presets.json",
		);
		cachedPresetCatalogPromise = fs
			.readFile(presetCatalogPath, "utf8")
			.then((raw) => JSON.parse(raw) as PresetCatalog)
			.catch((): PresetCatalog => ({ presets: [] }));
	}

	return cachedPresetCatalogPromise;
}

export function clearPresetCatalogCache(): void {
	cachedPresetCatalogPromise = undefined;
}

function buildActivePolicies(
	policyConfig: PolicyConfig,
	presetCatalog: PresetCatalog,
): PolicyMatch[] {
	const presetOrder = new Map<string, number>();
	const presetPolicies: PolicyMatch[] = [];

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

	const customPolicies = policyConfig.custom.map(
		(policy: PolicyMatch, index: number) => ({
			...policy,
			source: "custom" as const,
			order: index,
		}),
	);

	return [...presetPolicies, ...customPolicies];
}

function selectMatchingPolicy(
	activePolicies: PolicyMatch[],
	normalizedArgv: string[],
): PolicyMatch | null {
	let bestMatch: PolicyMatch | null = null;

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

function comparePolicyPriority(left: PolicyMatch, right: PolicyMatch): number {
	if (left.argvPrefix.length !== right.argvPrefix.length) {
		return right.argvPrefix.length - left.argvPrefix.length;
	}

	if (left.source !== right.source) {
		return left.source === "custom" ? -1 : 1;
	}

	return left.order - right.order;
}

function argvStartsWith(argv: string[], prefix: string[]): boolean {
	if (prefix.length > argv.length) {
		return false;
	}

	return prefix.every((token: string, index: number) => argv[index] === token);
}

function dropLeadingEnvAssignments(argv: string[]): void {
	while (argv.length > 0 && ENV_ASSIGNMENT_PATTERN.test(argv[0] ?? "")) {
		argv.shift();
	}
}

function tokenizeShellCommand(command: string): string[] | null {
	const tokens: string[] = [];
	let current = "";
	let quote: string | null = null;
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

function isPlainObject(value: unknown): value is Record<string, unknown> {
	return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

/**
 * Shell operators that split command segments.
 * These operators terminate a segment and start a new one.
 */
const SHELL_OPERATORS = new Set(["|", "&&", "||", ";", "&"]);

/**
 * Checks if a string is a shell control operator.
 */
function isShellOperator(token: string): boolean {
	return SHELL_OPERATORS.has(token);
}

/**
 * Splits a token that may contain embedded operators (e.g., "ls;" -> ["ls", ";"])
 */
function splitTokenWithEmbeddedOperators(token: string): string[] {
	const result: string[] = [];
	let current = "";
	let i = 0;

	while (i < token.length) {
		// Check for multi-character operators first (&&, ||)
		if (i + 1 < token.length) {
			const twoChar = token.slice(i, i + 2);
			if (twoChar === "&&" || twoChar === "||") {
				if (current.length > 0) {
					result.push(current);
					current = "";
				}
				result.push(twoChar);
				i += 2;
				continue;
			}
		}

		// Check for single-character operators
		const char = token[i];
		if (SHELL_OPERATORS.has(char)) {
			if (current.length > 0) {
				result.push(current);
				current = "";
			}
			result.push(char);
			i++;
			continue;
		}

		current += char;
		i++;
	}

	if (current.length > 0) {
		result.push(current);
	}

	return result;
}

/**
 * Parses a command string into segments separated by shell control operators.
 *
 * @param command - The command string to parse (e.g., "cat abc | git diff")
 * @returns An array of segments, where each segment is an array of tokens
 *          (e.g., [["cat", "abc"], ["git", "diff"]])
 *          Returns null if the command cannot be tokenized (e.g., unclosed quotes)
 *
 * Examples:
 *   "cat abc | git diff"  -> [["cat", "abc"], ["git", "diff"]]
 *   "git status && npm install" -> [["git", "status"], ["npm", "install"]]
 *   "ls; git push" -> [["ls"], ["git", "push"]]
 *   "ls &" -> [["ls"]]
 */
export function parseCommandSegments(command: string): string[][] | null {
	const tokens = tokenizeShellCommand(command);
	if (!tokens || tokens.length === 0) {
		return null;
	}

	// Flatten tokens that may contain embedded operators (e.g., "ls;" -> ["ls", ";"])
	const flattenedTokens: string[] = [];
	for (const token of tokens) {
		const splitTokens = splitTokenWithEmbeddedOperators(token);
		flattenedTokens.push(...splitTokens);
	}

	const segments: string[][] = [];
	let currentSegment: string[] = [];

	for (const token of flattenedTokens) {
		if (isShellOperator(token)) {
			// Start a new segment, skipping empty segments (consecutive operators)
			if (currentSegment.length > 0) {
				segments.push(currentSegment);
				currentSegment = [];
			}
		} else {
			currentSegment.push(token);
		}
	}

	// Don't add empty trailing segment after an operator
	if (currentSegment.length > 0) {
		segments.push(currentSegment);
	}

	return segments;
}
