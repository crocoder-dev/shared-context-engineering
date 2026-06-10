import { spawn } from "node:child_process";
import path from "node:path";

export const CLAUDE_AGENT_TRACE_EVENT_NAMES = [
	"SessionStart",
	"UserPromptSubmit",
	"PostToolUse",
	"Stop",
] as const;

export type ClaudeAgentTraceEventName =
	(typeof CLAUDE_AGENT_TRACE_EVENT_NAMES)[number];

export type ClaudeDiffTracePayload = {
	sessionID: string;
	diff: string;
	time: number;
	model_id?: string;
	tool_name: "claude";
	tool_version: string | null;
};

export type ClaudeDiffTraceSkipReason =
	| "unsupported_event"
	| "event_without_diff_trace"
	| "invalid_payload"
	| "event_name_mismatch"
	| "unsupported_tool"
	| "unsupported_write_payload"
	| "missing_file_path"
	| "missing_file_content"
	| "unsupported_edit_payload"
	| "missing_session_id";

export type ClaudeDiffTraceDerivationResult =
	| {
			status: "derived";
			payload: ClaudeDiffTracePayload;
	  }
	| {
			status: "skipped";
			reason: ClaudeDiffTraceSkipReason;
	  };

export type ClaudeHookPayloadParseResult =
	| {
			status: "ok";
			payload: unknown;
	  }
	| {
			status: "error";
			message: string;
	  };

export type DeriveClaudeDiffTraceInput = {
	eventName: string;
	payload: unknown;
	now?: () => number;
	toolVersion?: string | null;
};

type JsonObject = Record<string, unknown>;

type DiffBuildResult =
	| {
			status: "built";
			diff: string;
	  }
	| {
			status: "skipped";
			reason:
				| "unsupported_tool"
				| "unsupported_write_payload"
				| "missing_file_path"
				| "missing_file_content"
				| "unsupported_edit_payload";
	  };

const CLAUDE_MODEL_ID_PREFIX = "claude/";

export function isClaudeAgentTraceEventName(
	value: string,
): value is ClaudeAgentTraceEventName {
	return CLAUDE_AGENT_TRACE_EVENT_NAMES.includes(
		value as ClaudeAgentTraceEventName,
	);
}

export function parseClaudeHookPayloadJson(
	input: string,
): ClaudeHookPayloadParseResult {
	try {
		return {
			status: "ok",
			payload: JSON.parse(input),
		};
	} catch (error) {
		return {
			status: "error",
			message: error instanceof Error ? error.message : String(error),
		};
	}
}

export function deriveClaudeDiffTracePayload(
	input: DeriveClaudeDiffTraceInput,
): ClaudeDiffTraceDerivationResult {
	if (!isClaudeAgentTraceEventName(input.eventName)) {
		return skipped("unsupported_event");
	}

	if (input.eventName !== "PostToolUse") {
		return skipped("event_without_diff_trace");
	}

	const payload = asObject(input.payload);
	if (payload === undefined) {
		return skipped("invalid_payload");
	}

	const payloadEventName = stringField(payload, "hook_event_name");
	if (payloadEventName !== undefined && payloadEventName !== input.eventName) {
		return skipped("event_name_mismatch");
	}

	const diffResult = buildClaudePostToolUseDiff(payload);
	if (diffResult.status === "skipped") {
		return skipped(diffResult.reason);
	}

	const sessionId = stringField(payload, "session_id", "sessionID");
	if (sessionId === undefined) {
		return skipped("missing_session_id");
	}

	return {
		status: "derived",
		payload: {
			sessionID: sessionId,
			diff: diffResult.diff,
			time: currentTimeMs(input.now),
			tool_name: "claude",
			tool_version: extractClaudeToolVersion(input.toolVersion, payload),
		},
	};
}

export function normalizeClaudeModelId(model: string): string | undefined {
	const normalized = model.trim();
	if (normalized.length === 0) {
		return undefined;
	}

	if (normalized.startsWith(CLAUDE_MODEL_ID_PREFIX)) {
		return normalized;
	}

	return `${CLAUDE_MODEL_ID_PREFIX}${normalized}`;
}

function buildClaudePostToolUseDiff(payload: JsonObject): DiffBuildResult {
	const toolName = stringField(payload, "tool_name");
	if (toolName === "Write") {
		return buildWriteCreateDiff(payload);
	}

	if (toolName === "Edit") {
		return buildEditStructuredPatchDiff(payload);
	}

	return skipped("unsupported_tool");
}

function buildWriteCreateDiff(payload: JsonObject): DiffBuildResult {
	const toolInput = asObject(payload.tool_input);
	const toolResponse = asObject(payload.tool_response);
	if (toolInput === undefined || toolResponse === undefined) {
		return skipped("unsupported_write_payload");
	}

	const originalFile = valueField(
		toolResponse,
		"originalFile",
		"original_file",
	);
	if (originalFile !== null) {
		return skipped("unsupported_write_payload");
	}

	const filePath = normalizePatchPath(
		stringField(toolInput, "file_path", "filePath") ??
			stringField(toolResponse, "file_path", "filePath"),
		stringField(payload, "cwd"),
	);
	if (filePath === undefined) {
		return skipped("missing_file_path");
	}

	const content = stringValueField(toolInput, "content", "newFile", "new_file");
	if (content === undefined) {
		return skipped("missing_file_content");
	}

	return {
		status: "built",
		diff: renderWriteCreateDiff(filePath, content),
	};
}

function buildEditStructuredPatchDiff(payload: JsonObject): DiffBuildResult {
	const toolInput = asObject(payload.tool_input);
	const toolResponse = asObject(payload.tool_response);
	if (toolInput === undefined || toolResponse === undefined) {
		return skipped("unsupported_edit_payload");
	}

	const structuredPatch = valueField(
		toolResponse,
		"structuredPatch",
		"structured_patch",
	);
	if (structuredPatch === undefined || structuredPatch === null) {
		return skipped("unsupported_edit_payload");
	}

	const patchObject = asObject(structuredPatch);
	const filePath = normalizePatchPath(
		stringField(toolInput, "file_path", "filePath") ??
			(patchObject === undefined
				? undefined
				: stringField(patchObject, "file_path", "filePath", "path")),
		stringField(payload, "cwd"),
	);
	if (filePath === undefined) {
		return skipped("missing_file_path");
	}

	const hunkValues = structuredPatchHunks(structuredPatch);
	const renderedHunks = hunkValues
		.map(renderStructuredPatchHunk)
		.filter((hunk): hunk is string => hunk !== undefined);

	if (renderedHunks.length === 0) {
		return skipped("unsupported_edit_payload");
	}

	return {
		status: "built",
		diff: renderEditStructuredPatchDiff(filePath, renderedHunks),
	};
}

function renderWriteCreateDiff(filePath: string, content: string): string {
	const contentLines = splitFileContent(content);
	const diffLines = [
		`diff --git a/${filePath} b/${filePath}`,
		"new file mode 100644",
		"--- /dev/null",
		`+++ b/${filePath}`,
	];

	if (contentLines.length > 0) {
		diffLines.push(`@@ -0,0 +1,${contentLines.length} @@`);
		for (const line of contentLines) {
			diffLines.push(`+${line}`);
		}
	}

	return `${diffLines.join("\n")}\n`;
}

function renderEditStructuredPatchDiff(
	filePath: string,
	renderedHunks: string[],
): string {
	return `${[
		`Index: ${filePath}`,
		"===================================================================",
		`--- a/${filePath}`,
		`+++ b/${filePath}`,
		...renderedHunks,
	].join("\n")}\n`;
}

function renderStructuredPatchHunk(hunkValue: unknown): string | undefined {
	const hunk = asObject(hunkValue);
	if (hunk === undefined) {
		return undefined;
	}

	const lines = arrayField(hunk, "lines", "body", "changes")
		?.map(renderStructuredPatchLine)
		.filter((line): line is string => line !== undefined);
	if (lines === undefined || lines.length === 0 || !hasTouchedLine(lines)) {
		return undefined;
	}

	const oldStart = numericField(
		hunk,
		"oldStart",
		"old_start",
		"oldLine",
		"old_line",
	);
	const newStart = numericField(
		hunk,
		"newStart",
		"new_start",
		"newLine",
		"new_line",
	);
	if (oldStart === undefined || newStart === undefined) {
		return undefined;
	}

	const oldCount =
		numericField(hunk, "oldCount", "old_count", "oldLines", "old_lines") ??
		countOldHunkLines(lines);
	const newCount =
		numericField(hunk, "newCount", "new_count", "newLines", "new_lines") ??
		countNewHunkLines(lines);

	return [
		`@@ -${oldStart},${oldCount} +${newStart},${newCount} @@`,
		...lines,
	].join("\n");
}

function renderStructuredPatchLine(lineValue: unknown): string | undefined {
	if (typeof lineValue === "string") {
		if (
			lineValue.startsWith("+") ||
			lineValue.startsWith("-") ||
			lineValue.startsWith(" ") ||
			lineValue.startsWith("\\")
		) {
			return lineValue;
		}

		return ` ${lineValue}`;
	}

	const line = asObject(lineValue);
	if (line === undefined) {
		return undefined;
	}

	const content = stringValueField(line, "content", "text", "value");
	if (content === undefined) {
		return undefined;
	}

	const kind = stringField(line, "kind", "type", "operation", "change");
	if (
		kind === "context" ||
		kind === "unchanged" ||
		kind === "equal" ||
		kind === " "
	) {
		return ` ${content}`;
	}

	if (kind === "added" || kind === "add" || kind === "insert" || kind === "+") {
		return `+${content}`;
	}

	if (
		kind === "removed" ||
		kind === "remove" ||
		kind === "delete" ||
		kind === "-"
	) {
		return `-${content}`;
	}

	return undefined;
}

function structuredPatchHunks(structuredPatch: unknown): unknown[] {
	if (Array.isArray(structuredPatch)) {
		return structuredPatch;
	}

	const patchObject = asObject(structuredPatch);
	if (patchObject === undefined) {
		return [];
	}

	const hunks = arrayField(patchObject, "hunks", "changes");
	if (hunks !== undefined) {
		return hunks;
	}

	if (arrayField(patchObject, "lines", "body") !== undefined) {
		return [patchObject];
	}

	return [];
}

function splitFileContent(content: string): string[] {
	const normalizedContent = content.replace(/\r\n/g, "\n").replace(/\r/g, "\n");
	if (normalizedContent.length === 0) {
		return [];
	}

	if (normalizedContent.endsWith("\n")) {
		return normalizedContent.slice(0, -1).split("\n");
	}

	return normalizedContent.split("\n");
}

function extractDirectPayloadModel(payload: JsonObject): string | undefined {
	const directModel = stringField(payload, "model", "model_id", "modelId");
	if (directModel !== undefined) {
		return directModel;
	}

	const modelObject = asObject(payload.model);
	if (modelObject === undefined) {
		return undefined;
	}

	return stringField(modelObject, "id", "model", "name");
}

function extractClaudeToolVersion(
	inputToolVersion: string | null | undefined,
	payload: JsonObject,
): string | null {
	for (const value of [
		inputToolVersion,
		payload.tool_version,
		payload.claude_version,
		payload.version,
	]) {
		const normalized = normalizeOptionalVersion(value);
		if (normalized !== undefined) {
			return normalized;
		}
	}

	return null;
}

function normalizeOptionalVersion(value: unknown): string | null | undefined {
	if (value === undefined) {
		return undefined;
	}

	if (value === null) {
		return null;
	}

	if (typeof value !== "string") {
		return null;
	}

	const normalized = value.trim();
	return normalized.length === 0 ? null : normalized;
}

function normalizePatchPath(
	filePath: string | undefined,
	cwd: string | undefined,
): string | undefined {
	if (filePath === undefined) {
		return undefined;
	}

	let normalized = filePath.trim();
	if (normalized.length === 0) {
		return undefined;
	}

	if (
		cwd !== undefined &&
		path.isAbsolute(normalized) &&
		path.isAbsolute(cwd.trim())
	) {
		const relativePath = path.relative(cwd.trim(), normalized);
		if (
			relativePath.length > 0 &&
			!relativePath.startsWith("..") &&
			!path.isAbsolute(relativePath)
		) {
			normalized = relativePath;
		}
	}

	normalized = normalized.replaceAll("\\", "/").replace(/^\.\/+/, "");
	return normalized.length === 0 || normalized === "." ? undefined : normalized;
}

function hasTouchedLine(lines: string[]): boolean {
	return lines.some((line) => line.startsWith("+") || line.startsWith("-"));
}

function countOldHunkLines(lines: string[]): number {
	return lines.filter((line) => !line.startsWith("+") && !line.startsWith("\\"))
		.length;
}

function countNewHunkLines(lines: string[]): number {
	return lines.filter((line) => !line.startsWith("-") && !line.startsWith("\\"))
		.length;
}

function currentTimeMs(now: (() => number) | undefined): number {
	const value = now === undefined ? Date.now() : now();
	return Number.isFinite(value) ? Math.trunc(value) : Date.now();
}

function asObject(value: unknown): JsonObject | undefined {
	return typeof value === "object" && value !== null && !Array.isArray(value)
		? (value as JsonObject)
		: undefined;
}

function stringField(
	object: JsonObject,
	...keys: string[]
): string | undefined {
	for (const key of keys) {
		const value = object[key];
		if (typeof value !== "string") {
			continue;
		}

		const normalized = value.trim();
		if (normalized.length > 0) {
			return normalized;
		}
	}

	return undefined;
}

function stringValueField(
	object: JsonObject,
	...keys: string[]
): string | undefined {
	for (const key of keys) {
		const value = object[key];
		if (typeof value === "string") {
			return value;
		}
	}

	return undefined;
}

function numericField(
	object: JsonObject,
	...keys: string[]
): number | undefined {
	for (const key of keys) {
		const value = object[key];
		if (typeof value !== "number") {
			continue;
		}

		if (Number.isInteger(value) && value >= 0) {
			return value;
		}
	}

	return undefined;
}

function arrayField(
	object: JsonObject,
	...keys: string[]
): unknown[] | undefined {
	for (const key of keys) {
		const value = object[key];
		if (Array.isArray(value)) {
			return value;
		}
	}

	return undefined;
}

function valueField(object: JsonObject, ...keys: string[]): unknown {
	for (const key of keys) {
		if (Object.hasOwn(object, key)) {
			return object[key];
		}
	}

	return undefined;
}

function skipped<T extends ClaudeDiffTraceSkipReason>(
	reason: T,
): {
	status: "skipped";
	reason: T;
} {
	return {
		status: "skipped",
		reason,
	};
}

// ─── Runtime: child-process spawn infrastructure ───────────────────────

/**
 * Injectable spawn function signature used by the Claude hook runtime.
 * Takes a command, arguments, stdin input, and optional cwd; resolves with
 * the exit code and signal (null for normal exits).
 */
export type SpawnFn = (
	command: string,
	args: readonly string[],
	input: string,
	options?: { cwd?: string },
) => Promise<{ code: number | null; signal: string | null }>;

/**
 * Real spawn implementation using `child_process.spawn`.
 */
export function createSpawnFn(): SpawnFn {
	return (
		command: string,
		args: readonly string[],
		input: string,
		options?: { cwd?: string },
	): Promise<{ code: number | null; signal: string | null }> => {
		return new Promise((resolve, reject) => {
			const child = spawn(command, args, {
				cwd: options?.cwd,
				stdio: ["pipe", "ignore", "inherit"],
			});

			child.on("error", reject);
			child.on("close", (code: number | null, signal: string | null) => {
				resolve({ code, signal });
			});
			child.stdin.end(input);
		});
	};
}

/**
 * Read the entire contents of STDIN as a string.
 * Returns an empty string when STDIN is a TTY (no piped data).
 */
export function readStdin(): Promise<string> {
	return new Promise<string>((resolve, reject) => {
		if (process.stdin.isTTY) {
			resolve("");
			return;
		}

		const chunks: Buffer[] = [];

		process.stdin.on("data", (chunk: Buffer) => {
			chunks.push(chunk);
		});

		process.stdin.on("end", () => {
			resolve(Buffer.concat(chunks).toString("utf-8"));
		});

		process.stdin.on("error", reject);
		process.stdin.resume();
	});
}

// ─── Runtime: Claude hook event orchestration ──────────────────────────

/**
 * Context passed to the Claude hook runtime for external dependencies.
 */
export type ClaudeHookRuntimeContext = {
	/** Spawn function (real or mock) used for child-process forwarding. */
	spawn: SpawnFn;
	/** Optional working directory forwarded to spawned processes. */
	cwd?: string;
	/** Optional timestamp supplier for diff-trace derivation (defaults to Date.now). */
	now?: () => number;
};

/**
 * Run the Claude hook runtime for a single hook event.
 *
 * - `SessionStart`: Extracts `session_id` + `model_id` and forwards a
 *   normalized session-model payload to `sce hooks session-model` (best-effort).
 * - `PostToolUse`: Derives a diff-trace payload and forwards it to
 *   `sce hooks diff-trace` (best-effort). Model attribution is resolved by
 *   Rust from `session_models`; TypeScript does not look up the model.
 * - Other events: No-op (no raw capture forwarding).
 *
 * All forwarding errors are caught and logged to stderr without failing the
 * Claude hook.
 *
 * @param eventName - Validated Claude hook event name
 * @param rawJson - Raw JSON payload read from STDIN
 * @param context - Injectable dependencies
 */
export async function runClaudeHookRuntime(
	eventName: string,
	rawJson: string,
	context: ClaudeHookRuntimeContext,
): Promise<void> {
	if (eventName === "SessionStart") {
		await handleSessionStart(rawJson, context);
		return;
	}

	if (eventName !== "PostToolUse") {
		return;
	}

	try {
		const parseResult = parseClaudeHookPayloadJson(rawJson);
		if (parseResult.status !== "ok") {
			return;
		}

		const derivation = deriveClaudeDiffTracePayload({
			eventName,
			payload: parseResult.payload,
			now: context.now,
		});

		if (derivation.status !== "derived") {
			return;
		}

		await context.spawn(
			"sce",
			["hooks", "diff-trace"],
			`${JSON.stringify(derivation.payload)}\n`,
			{ cwd: context.cwd },
		);
	} catch (error) {
		console.error(
			`[sce] Diff-trace forwarding failed: ${error instanceof Error ? error.message : String(error)}`,
		);
	}
}

async function handleSessionStart(
	rawJson: string,
	context: ClaudeHookRuntimeContext,
): Promise<void> {
	try {
		const parseResult = parseClaudeHookPayloadJson(rawJson);
		if (parseResult.status !== "ok") {
			return;
		}

		const payload = asObject(parseResult.payload);
		if (payload === undefined) {
			return;
		}

		const sessionId = stringField(payload, "session_id", "sessionID");
		const modelId = extractDirectPayloadModel(payload);
		if (sessionId === undefined || modelId === undefined) {
			return;
		}

		await context.spawn(
			"sce",
			["hooks", "session-model"],
			`${JSON.stringify({
				sessionID: sessionId,
				time: currentTimeMs(context.now),
				model_id: normalizeClaudeModelId(modelId),
				tool_name: "claude",
				tool_version: extractClaudeToolVersion(undefined, payload),
			})}\n`,
			{ cwd: context.cwd },
		);
	} catch (error) {
		console.error(
			`[sce] SessionStart model attribution failed: ${error instanceof Error ? error.message : String(error)}`,
		);
	}
}

/**
 * Main entry point for `bun .claude/plugins/sce-agent-trace.ts <event-name>`.
 *
 * - Reads the event name from `process.argv[2]`.
 * - Reads the hook JSON payload from STDIN.
 * - Delegates to {@link runClaudeHookRuntime}.
 * - Exits with code 1 on missing/invalid event name or stdin read failure.
 * - Exits with code 0 otherwise (internal forwarding errors are best-effort
 *   and do not change the exit code).
 */
export async function main(): Promise<void> {
	const eventName = process.argv[2];

	if (!eventName) {
		console.error("Usage: sce-agent-trace.ts <event-name>");
		process.exit(1);
	}

	if (!isClaudeAgentTraceEventName(eventName)) {
		console.error(`Unknown Claude hook event: ${eventName}`);
		process.exit(1);
	}

	let stdinContent: string;
	try {
		stdinContent = await readStdin();
	} catch (error) {
		console.error(
			`Failed to read stdin: ${error instanceof Error ? error.message : String(error)}`,
		);
		process.exit(1);
	}

	try {
		await runClaudeHookRuntime(eventName, stdinContent, {
			spawn: createSpawnFn(),
		});
	} catch (error) {
		console.error(
			`[sce] Hook runtime error for ${eventName}: ${error instanceof Error ? error.message : String(error)}`,
		);
		process.exit(1);
	}
}

// Allow direct execution: `bun run .../sce-agent-trace.ts <event-name>`
if (import.meta.main) {
	main();
}
