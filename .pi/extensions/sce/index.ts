import type { ChildProcess, ChildProcessByStdio } from "node:child_process";
import { spawnSync } from "node:child_process";
import { randomUUID } from "node:crypto";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import {
	dirname,
	isAbsolute,
	join,
	relative,
	resolve as resolvePath,
} from "node:path";
import type { Readable, Writable } from "node:stream";
import { fileURLToPath } from "node:url";
import {
	type ExtensionAPI,
	isToolCallEventType,
} from "@earendil-works/pi-coding-agent";

interface JsonPolicyResult {
	status: string;
	decision: string;
	command: string;
	normalized_argv?: string[];
	reason?: string;
	policy_id?: string;
}

const SCE_INSTALL_URL =
	"https://sce.crocoder.dev/docs/getting-started#install-cli";
const TOOL_NAME = "pi" as const;

type SpawnFn = typeof import("node:child_process").spawn;

function nodeSpawn(
	command: string,
	args: readonly string[],
	options: { cwd: string; stdio: readonly ["pipe", "ignore", "ignore"] },
): ChildProcessByStdio<Writable, null, null>;
function nodeSpawn(
	command: string,
	args: readonly string[],
	options: { cwd: string; stdio: readonly ["pipe", "pipe", "ignore"] },
): ChildProcessByStdio<Writable, Readable, null>;
function nodeSpawn(
	command: string,
	args: readonly string[],
	options: Record<string, unknown>,
): ChildProcess {
	const spawnImpl = createRequire(import.meta.url)("node:child_process")
		.spawn as SpawnFn;
	return spawnImpl(command, args as string[], options as never);
}

type ConversationTraceMessageItem = {
	type: "message";
	session_id: string;
	message_id: string;
	role: "user" | "assistant";
	generated_at_unix_ms: number;
};

type ConversationTraceMessagePartItem = {
	type: "message.part";
	session_id: string;
	message_id: string;
	part_type: "text" | "reasoning" | "patch";
	text: string;
	generated_at_unix_ms: number;
};

type ConversationTraceItem =
	| ConversationTraceMessageItem
	| ConversationTraceMessagePartItem;

type ConversationTracePayload = {
	tool_name: typeof TOOL_NAME;
	payloads: ConversationTraceItem[];
};

type DiffTracePayload = {
	sessionID: string;
	diff: string;
	time: number;
	model_id: string | null;
	tool_name: typeof TOOL_NAME;
	tool_version: string | null;
};

type PendingFileMutation = {
	absolutePath: string;
	diffLabel: string;
	before: string | undefined;
};

/**
 * Evaluate a bash command against SCE bash-tool policy by delegating to the
 * Rust `sce policy bash` command. Returns the parsed JSON result, or null if
 * the policy check could not be performed (fail-open).
 */
function evaluateBashCommandPolicy(command: string): JsonPolicyResult | null {
	try {
		const result = spawnSync(
			"sce",
			["policy", "bash", "--input", "normalized", "--output", "json"],
			{
				input: JSON.stringify({ command }),
				encoding: "utf8",
				timeout: 10_000,
			},
		);

		if (result.error) {
			if ((result.error as NodeJS.ErrnoException).code === "ENOENT") {
				console.warn(`sce CLI not found. Install it from ${SCE_INSTALL_URL}`);
			}
			return null;
		}

		if (result.status !== 0) {
			return null;
		}

		const stdout = result.stdout?.trim();
		if (!stdout) {
			return null;
		}

		const parsed: JsonPolicyResult = JSON.parse(stdout);
		return parsed;
	} catch {
		return null;
	}
}

/**
 * Send a conversation-trace payload to `sce hooks conversation-trace`,
 * fire-and-forget. Fail-open: stderr is ignored so that sce intake errors do
 * not leak into the Pi TUI, and the returned promise never rejects.
 */
function runConversationTraceHook(
	cwd: string,
	payload: ConversationTracePayload,
): Promise<void> {
	return new Promise<void>((resolve) => {
		const child = nodeSpawn("sce", ["hooks", "conversation-trace"], {
			cwd,
			stdio: ["pipe", "ignore", "ignore"],
		});

		child.on("error", (err: NodeJS.ErrnoException) => {
			if (err.code === "ENOENT") {
				console.warn(`sce CLI not found. Install it from ${SCE_INSTALL_URL}`);
			}
			resolve();
		});
		child.on("close", () => resolve());

		child.stdin.end(`${JSON.stringify(payload)}\n`);
	});
}

type MessageContentBlock = {
	type: string;
	text?: unknown;
	thinking?: unknown;
};

function extractMessageParts(
	content: string | readonly MessageContentBlock[],
): Array<{ part_type: "text" | "reasoning"; text: string }> {
	if (typeof content === "string") {
		return content.length > 0 ? [{ part_type: "text", text: content }] : [];
	}

	const parts: Array<{ part_type: "text" | "reasoning"; text: string }> = [];
	for (const block of content) {
		if (block.type === "text" && typeof block.text === "string" && block.text) {
			parts.push({ part_type: "text", text: block.text });
		} else if (
			block.type === "thinking" &&
			typeof block.thinking === "string" &&
			block.thinking
		) {
			parts.push({ part_type: "reasoning", text: block.thinking });
		}
	}
	return parts;
}

function buildMessageEndConversationTracePayload(
	sessionId: string,
	message: {
		role: string;
		content: string | readonly MessageContentBlock[];
		responseId?: string;
	},
): ConversationTracePayload | undefined {
	if (message.role !== "user" && message.role !== "assistant") {
		return undefined;
	}

	const messageId = message.responseId ?? randomUUID();
	const generatedAtUnixMs = Date.now();

	const payloads: ConversationTraceItem[] = [
		{
			type: "message",
			session_id: sessionId,
			message_id: messageId,
			role: message.role,
			generated_at_unix_ms: generatedAtUnixMs,
		},
	];

	for (const part of extractMessageParts(message.content)) {
		payloads.push({
			type: "message.part",
			session_id: sessionId,
			message_id: messageId,
			part_type: part.part_type,
			text: part.text,
			generated_at_unix_ms: generatedAtUnixMs,
		});
	}

	return { tool_name: TOOL_NAME, payloads };
}

/**
 * Resolve the installed Pi package version for diff-trace `tool_version`.
 * The package's `exports` map does not expose `package.json`, so resolve the
 * package entry point and read `package.json` from the package root instead.
 * Returns null when resolution fails (normalized diff traces permit it).
 */
async function resolvePiToolVersion(): Promise<string | null> {
	try {
		const entryPath = fileURLToPath(
			import.meta.resolve("@earendil-works/pi-coding-agent"),
		);
		const packageJsonPath = join(dirname(entryPath), "..", "package.json");
		const parsed: { version?: unknown } = JSON.parse(
			await readFile(packageJsonPath, "utf8"),
		);
		return typeof parsed.version === "string" && parsed.version.length > 0
			? parsed.version
			: null;
	} catch {
		return null;
	}
}

/**
 * Send a diff-trace payload to `sce hooks diff-trace`, fire-and-forget.
 * Fail-open: stderr is ignored and the returned promise never rejects.
 */
function runDiffTraceHook(
	cwd: string,
	payload: DiffTracePayload,
): Promise<void> {
	return new Promise<void>((resolve) => {
		const child = nodeSpawn("sce", ["hooks", "diff-trace"], {
			cwd,
			stdio: ["pipe", "ignore", "ignore"],
		});

		child.on("error", (err: NodeJS.ErrnoException) => {
			if (err.code === "ENOENT") {
				console.warn(`sce CLI not found. Install it from ${SCE_INSTALL_URL}`);
			}
			resolve();
		});
		child.on("close", () => resolve());

		child.stdin.end(`${JSON.stringify(payload)}\n`);
	});
}

async function readFileOrUndefined(path: string): Promise<string | undefined> {
	try {
		return await readFile(path, "utf8");
	} catch {
		return undefined;
	}
}

function diffLabelFor(cwd: string, absolutePath: string): string {
	const relPath = relative(cwd, absolutePath);
	return relPath.length > 0 && !relPath.startsWith("..") && !isAbsolute(relPath)
		? relPath
		: absolutePath;
}

/**
 * Rewrite temp-file path labels in git diff header lines to the repo-relative
 * target path. Only header lines before the first `@@` hunk marker are
 * touched so that content lines starting with `--- ` / `+++ ` are preserved.
 */
function rewriteDiffLabels(
	diff: string,
	label: string,
	isCreate: boolean,
): string {
	const lines = diff.split("\n");
	for (let i = 0; i < lines.length; i++) {
		const line = lines[i];
		if (line.startsWith("@@")) {
			break;
		}
		if (line.startsWith("diff --git ")) {
			lines[i] = `diff --git a/${label} b/${label}`;
		} else if (line.startsWith("--- ")) {
			lines[i] = isCreate ? "--- /dev/null" : `--- a/${label}`;
		} else if (line.startsWith("+++ ")) {
			lines[i] = `+++ b/${label}`;
		}
	}
	return lines.join("\n");
}

/**
 * Produce a unified diff between before/after contents by writing them to
 * temp files and spawning `git diff --no-index --no-ext-diff` (exit status 1
 * means "files differ"). Returns undefined for no-op diffs or any failure;
 * temp files are always cleaned up.
 */
async function buildUnifiedDiff(
	label: string,
	before: string | undefined,
	after: string,
): Promise<string | undefined> {
	const tempDir = await mkdtemp(join(tmpdir(), "sce-pi-diff-"));
	try {
		const beforePath = join(tempDir, "before");
		const afterPath = join(tempDir, "after");
		await writeFile(beforePath, before ?? "", "utf8");
		await writeFile(afterPath, after, "utf8");

		const result = spawnSync(
			"git",
			["diff", "--no-index", "--no-ext-diff", "--", beforePath, afterPath],
			{ encoding: "utf8", timeout: 10_000 },
		);

		if (result.error || result.status !== 1) {
			return undefined;
		}
		const stdout = result.stdout;
		if (!stdout) {
			return undefined;
		}
		return rewriteDiffLabels(stdout, label, before === undefined);
	} catch {
		return undefined;
	} finally {
		await rm(tempDir, { recursive: true, force: true }).catch(() => {});
	}
}

export type PiMutationHookEventName =
	| "ToolExecutionStart"
	| "ToolCall"
	| "ToolResult"
	| "ToolExecutionEnd"
	| "ToolExecutionAbandon";

export type PiMutationScopePayload = {
	hook_event_name: PiMutationHookEventName;
	session_id: string;
	tool_call_id: string;
	cwd: string;
	tool_name: string;
	model?: string;
};

const MUTATION_SCOPE_FAIL_CLOSED_MESSAGE =
	"SCE could not establish Pi mutation attribution for this tool execution.";
const MUTATION_SCOPE_TIMEOUT_MS = 20_000;

const TRACKED_MUTATION_TOOL_NAMES = new Set(["bash", "edit", "write"]);

type MutationScopeStartOutcome = "ok" | "denied" | "cli-missing";

function forwardMutationScopeStart(
	payload: PiMutationScopePayload,
): MutationScopeStartOutcome {
	let result: ReturnType<typeof spawnSync>;
	try {
		result = spawnSync("sce", ["hooks", "pi-mutation-scope"], {
			input: JSON.stringify(payload),
			encoding: "utf8",
			timeout: MUTATION_SCOPE_TIMEOUT_MS,
		});
	} catch {
		return "denied";
	}

	if (result.error) {
		if ((result.error as NodeJS.ErrnoException).code === "ENOENT") {
			console.warn(`sce CLI not found. Install it from ${SCE_INSTALL_URL}`);
			return "cli-missing";
		}
		return "denied";
	}

	return result.status === 0 ? "ok" : "denied";
}

function forwardMutationScopeBestEffort(
	cwd: string,
	payload: PiMutationScopePayload,
): Promise<void> {
	return new Promise<void>((resolve) => {
		const child = nodeSpawn("sce", ["hooks", "pi-mutation-scope"], {
			cwd,
			stdio: ["pipe", "ignore", "ignore"],
		});

		child.on("error", (err: NodeJS.ErrnoException) => {
			if (err.code === "ENOENT") {
				console.warn(`sce CLI not found. Install it from ${SCE_INSTALL_URL}`);
			}
			resolve();
		});
		child.on("close", () => resolve());

		child.stdin.end(`${JSON.stringify(payload)}\n`);
	});
}

function attemptMutationScopeDelivery(
	cwd: string,
	payload: PiMutationScopePayload,
): Promise<boolean> {
	return new Promise<boolean>((resolve) => {
		const child = nodeSpawn("sce", ["hooks", "pi-mutation-scope"], {
			cwd,
			stdio: ["pipe", "ignore", "ignore"],
		});

		let settled = false;
		const finish = (delivered: boolean) => {
			if (settled) {
				return;
			}
			settled = true;
			resolve(delivered);
		};

		child.on("error", (err: NodeJS.ErrnoException) => {
			if (err.code === "ENOENT") {
				console.warn(`sce CLI not found. Install it from ${SCE_INSTALL_URL}`);
			}
			finish(false);
		});
		child.on("close", (code) => finish(code === 0));

		child.stdin.end(`${JSON.stringify(payload)}\n`);
	});
}

const TERMINAL_RETRY_INITIAL_MS = 500;
const TERMINAL_RETRY_MAX_MS = 10_000;

export type AttemptKey = { sessionId: string; toolCallId: string };

function attemptMapKey(key: AttemptKey): string {
	return `s=${key.sessionId.length}:${key.sessionId}|c=${key.toolCallId.length}:${key.toolCallId}`;
}

type ResultDeliveryOutcome = "delivered" | "failed";

export type RetryScheduleFn = (run: () => void, delayMs: number) => void;

const defaultRetrySchedule: RetryScheduleFn = (run, delayMs) => {
	const timer = setTimeout(run, delayMs);
	timer.unref?.();
};

export function createTerminalDeliveryTracker(
	schedule: RetryScheduleFn = defaultRetrySchedule,
) {
	const attempts = new Map<
		string,
		{ resultDelivery: Promise<ResultDeliveryOutcome> }
	>();
	const unresolved = new Set<string>();

	function scheduleAbandonRetry(
		cwd: string,
		endPayload: PiMutationScopePayload,
		mapKey: string,
		delayMs: number,
	): void {
		const abandonPayload: PiMutationScopePayload = {
			...endPayload,
			hook_event_name: "ToolExecutionAbandon",
		};
		schedule(() => {
			void attemptMutationScopeDelivery(cwd, abandonPayload).then(
				(delivered) => {
					if (delivered) {
						unresolved.delete(mapKey);
						return;
					}
					scheduleAbandonRetry(
						cwd,
						endPayload,
						mapKey,
						Math.min(delayMs * 2, TERMINAL_RETRY_MAX_MS),
					);
				},
			);
		}, delayMs);
	}

	async function deliverEndThenFallbackToAbandon(
		cwd: string,
		endPayload: PiMutationScopePayload,
		mapKey: string,
	): Promise<void> {
		unresolved.add(mapKey);
		const delivered = await attemptMutationScopeDelivery(cwd, endPayload);
		if (delivered) {
			unresolved.delete(mapKey);
			return;
		}
		scheduleAbandonRetry(cwd, endPayload, mapKey, TERMINAL_RETRY_INITIAL_MS);
	}

	async function deliverAbandonImmediately(
		cwd: string,
		endPayload: PiMutationScopePayload,
		mapKey: string,
	): Promise<void> {
		unresolved.add(mapKey);
		const abandonPayload: PiMutationScopePayload = {
			...endPayload,
			hook_event_name: "ToolExecutionAbandon",
		};
		const delivered = await attemptMutationScopeDelivery(cwd, abandonPayload);
		if (delivered) {
			unresolved.delete(mapKey);
			return;
		}
		scheduleAbandonRetry(cwd, endPayload, mapKey, TERMINAL_RETRY_INITIAL_MS);
	}

	return {
		hasUnresolved(): boolean {
			return unresolved.size > 0;
		},

		forwardResult(
			cwd: string,
			payload: PiMutationScopePayload,
			key: AttemptKey,
		): void {
			const mapKey = attemptMapKey(key);
			const resultDelivery = attemptMutationScopeDelivery(cwd, payload).then(
				(delivered): ResultDeliveryOutcome => {
					if (delivered) {
						return "delivered";
					}
					unresolved.add(mapKey);
					return "failed";
				},
			);
			attempts.set(mapKey, { resultDelivery });
		},

		async forwardEnd(
			cwd: string,
			payload: PiMutationScopePayload,
			key: AttemptKey,
		): Promise<void> {
			const mapKey = attemptMapKey(key);
			const entry = attempts.get(mapKey);
			attempts.delete(mapKey);

			if (!entry) {
				await deliverEndThenFallbackToAbandon(cwd, payload, mapKey);
				return;
			}

			const outcome = await entry.resultDelivery;
			if (outcome === "failed") {
				await deliverAbandonImmediately(cwd, payload, mapKey);
				return;
			}

			await deliverEndThenFallbackToAbandon(cwd, payload, mapKey);
		},
	};
}

const GUARD_ESTABLISH_TIMEOUT_MS = 10_000;

const GUARD_UNAVAILABLE_MESSAGE =
	"SCE could not establish the worktree external-mutation guard for this command.";
const WINDOWS_UNSUPPORTED_MESSAGE =
	"SCE does not support guarded user_bash execution on Windows in this release; run this command outside Pi.";

function guardRefusal(output: string) {
	return {
		result: { output, exitCode: 1, cancelled: false, truncated: false },
	};
}

type GuardLine =
	| { status: "armed" }
	| { stream: "stdout" | "stderr"; data: string }
	| { status: "result"; exit_code: number | null };

class LineReader {
	private buffer = "";
	private readonly onLine: (line: string) => void;

	constructor(onLine: (line: string) => void) {
		this.onLine = onLine;
	}

	push(chunk: Buffer | string): void {
		this.buffer += chunk.toString();
		let index = this.buffer.indexOf("\n");
		while (index !== -1) {
			const line = this.buffer.slice(0, index);
			this.buffer = this.buffer.slice(index + 1);
			if (line.length > 0) {
				this.onLine(line);
			}
			index = this.buffer.indexOf("\n");
		}
	}
}

class ExternalMutationGuardSession {
	private readonly child: ReturnType<typeof nodeSpawn>;
	private readonly pendingLines: GuardLine[] = [];
	private readonly waiters: Array<(line: GuardLine | undefined) => void> = [];
	private closed = false;

	constructor(cwd: string) {
		this.child = nodeSpawn("sce", ["hooks", "external-mutation-guard"], {
			cwd,
			stdio: ["pipe", "pipe", "ignore"],
		});
		const reader = new LineReader((line) => {
			try {
				this.deliver(JSON.parse(line) as GuardLine);
			} catch {}
		});
		this.child.stdout?.on("data", (chunk: Buffer) => reader.push(chunk));
		this.child.on("close", () => {
			this.closed = true;
			this.deliver(undefined);
		});
		this.child.on("error", () => {
			this.closed = true;
			this.deliver(undefined);
		});
	}

	private deliver(line: GuardLine | undefined): void {
		const waiter = this.waiters.shift();
		if (waiter) {
			waiter(line);
		} else if (line !== undefined) {
			this.pendingLines.push(line);
		}
	}

	private nextLine(): Promise<GuardLine | undefined> {
		const queued = this.pendingLines.shift();
		if (queued) {
			return Promise.resolve(queued);
		}
		if (this.closed) {
			return Promise.resolve(undefined);
		}
		return new Promise((resolve) => this.waiters.push(resolve));
	}

	private send(payload: Record<string, unknown>): void {
		this.child.stdin?.write(`${JSON.stringify(payload)}\n`);
	}

	async waitForArmed(timeoutMs: number): Promise<boolean> {
		this.send({ operation: "arm" });
		const timedOut = Symbol("timeout");
		const timeout = new Promise<typeof timedOut>((resolve) => {
			setTimeout(() => resolve(timedOut), timeoutMs);
		});
		const outcome = await Promise.race([this.nextLine(), timeout]);
		return (
			outcome !== undefined &&
			outcome !== timedOut &&
			"status" in outcome &&
			outcome.status === "armed"
		);
	}

	exec(
		command: string,
		cwd: string,
		options: {
			onData: (data: Buffer) => void;
			signal?: AbortSignal;
			timeout?: number;
			env?: NodeJS.ProcessEnv;
		},
	): Promise<{ exitCode: number | null }> {
		const env: Record<string, string> = {};
		if (options.env) {
			for (const [key, value] of Object.entries(options.env)) {
				if (typeof value === "string") {
					env[key] = value;
				}
			}
		}
		this.send({ operation: "exec", command, cwd, env });

		const onAbort = () => this.send({ operation: "cancel" });
		options.signal?.addEventListener("abort", onAbort);
		const timeoutHandle = options.timeout
			? setTimeout(onAbort, options.timeout)
			: undefined;

		return (async () => {
			try {
				for (;;) {
					const line = await this.nextLine();
					if (line === undefined) {
						throw new Error(
							"SCE lost contact with the external-mutation-guard supervisor before it reported a command result.",
						);
					}
					if ("stream" in line) {
						options.onData(Buffer.from(line.data));
						continue;
					}
					if (line.status === "result") {
						return { exitCode: line.exit_code };
					}
				}
			} finally {
				options.signal?.removeEventListener("abort", onAbort);
				if (timeoutHandle) {
					clearTimeout(timeoutHandle);
				}
			}
		})();
	}

	terminate(): void {
		this.child.kill();
	}
}

export default function sceExtension(pi: ExtensionAPI): void {
	const pendingFileMutations = new Map<string, PendingFileMutation>();
	const piToolVersionPromise = resolvePiToolVersion();
	const terminalDelivery = createTerminalDeliveryTracker();

	pi.on("tool_call", (event) => {
		if (!isToolCallEventType("bash", event)) {
			return undefined;
		}

		const command = event.input.command;
		if (typeof command !== "string" || command.length === 0) {
			return undefined;
		}

		const policyResult = evaluateBashCommandPolicy(command);
		if (!policyResult) {
			// Fail open: if the policy check cannot be performed, allow the command.
			return undefined;
		}

		if (policyResult.decision === "deny" && policyResult.reason) {
			return { block: true, reason: policyResult.reason };
		}

		return undefined;
	});

	pi.on("tool_call", async (event, ctx) => {
		if (!TRACKED_MUTATION_TOOL_NAMES.has(event.toolName)) {
			return undefined;
		}

		if (terminalDelivery.hasUnresolved()) {
			return { block: true, reason: MUTATION_SCOPE_FAIL_CLOSED_MESSAGE };
		}

		const outcome = forwardMutationScopeStart({
			hook_event_name: "ToolCall",
			session_id: ctx.sessionManager.getSessionId(),
			tool_call_id: event.toolCallId,
			cwd: ctx.cwd,
			tool_name: event.toolName,
			model: ctx.model ? `${ctx.model.provider}/${ctx.model.id}` : undefined,
		});

		if (outcome === "ok") {
			return undefined;
		}
		return { block: true, reason: MUTATION_SCOPE_FAIL_CLOSED_MESSAGE };
	});

	pi.on("tool_execution_start", (event, ctx) => {
		if (!TRACKED_MUTATION_TOOL_NAMES.has(event.toolName)) {
			return;
		}
		void forwardMutationScopeBestEffort(ctx.cwd, {
			hook_event_name: "ToolExecutionStart",
			session_id: ctx.sessionManager.getSessionId(),
			tool_call_id: event.toolCallId,
			cwd: ctx.cwd,
			tool_name: event.toolName,
		});
	});

	pi.on("tool_result", (event, ctx) => {
		if (!TRACKED_MUTATION_TOOL_NAMES.has(event.toolName)) {
			return;
		}
		const sessionId = ctx.sessionManager.getSessionId();
		terminalDelivery.forwardResult(
			ctx.cwd,
			{
				hook_event_name: "ToolResult",
				session_id: sessionId,
				tool_call_id: event.toolCallId,
				cwd: ctx.cwd,
				tool_name: event.toolName,
			},
			{ sessionId, toolCallId: event.toolCallId },
		);
	});

	pi.on("tool_execution_end", (event, ctx) => {
		if (!TRACKED_MUTATION_TOOL_NAMES.has(event.toolName)) {
			return;
		}
		const sessionId = ctx.sessionManager.getSessionId();
		void terminalDelivery.forwardEnd(
			ctx.cwd,
			{
				hook_event_name: "ToolExecutionEnd",
				session_id: sessionId,
				tool_call_id: event.toolCallId,
				cwd: ctx.cwd,
				tool_name: event.toolName,
			},
			{ sessionId, toolCallId: event.toolCallId },
		);
	});

	pi.on("user_bash", async (event) => {
		if (process.platform === "win32") {
			return guardRefusal(WINDOWS_UNSUPPORTED_MESSAGE);
		}

		const guard = new ExternalMutationGuardSession(event.cwd);
		const armed = await guard.waitForArmed(GUARD_ESTABLISH_TIMEOUT_MS);
		if (!armed) {
			guard.terminate();
			return guardRefusal(GUARD_UNAVAILABLE_MESSAGE);
		}

		return {
			operations: {
				exec: (
					command: string,
					cwd: string,
					options: {
						onData: (data: Buffer) => void;
						signal?: AbortSignal;
						timeout?: number;
						env?: NodeJS.ProcessEnv;
					},
				) => guard.exec(command, cwd, options),
			},
		};
	});

	pi.on("tool_call", async (event, ctx) => {
		if (
			!isToolCallEventType("edit", event) &&
			!isToolCallEventType("write", event)
		) {
			return undefined;
		}

		const targetPath = event.input.path;
		if (typeof targetPath !== "string" || targetPath.length === 0) {
			return undefined;
		}

		const absolutePath = resolvePath(ctx.cwd, targetPath);
		pendingFileMutations.set(event.toolCallId, {
			absolutePath,
			diffLabel: diffLabelFor(ctx.cwd, absolutePath),
			before: await readFileOrUndefined(absolutePath),
		});
		return undefined;
	});

	pi.on("tool_result", async (event, ctx) => {
		const pending = pendingFileMutations.get(event.toolCallId);
		if (!pending) {
			return;
		}
		pendingFileMutations.delete(event.toolCallId);

		if (event.isError) {
			return;
		}

		const after = await readFileOrUndefined(pending.absolutePath);
		if (after === undefined || after === pending.before) {
			return;
		}

		const diff = await buildUnifiedDiff(
			pending.diffLabel,
			pending.before,
			after,
		);
		if (!diff) {
			return;
		}

		const sessionId = ctx.sessionManager.getSessionId();
		const generatedAtUnixMs = Date.now();
		const patchMessageId = `${event.toolCallId}-patch`;

		void runConversationTraceHook(ctx.cwd, {
			tool_name: TOOL_NAME,
			payloads: [
				{
					type: "message",
					session_id: sessionId,
					message_id: patchMessageId,
					role: "assistant",
					generated_at_unix_ms: generatedAtUnixMs,
				},
				{
					type: "message.part",
					session_id: sessionId,
					message_id: patchMessageId,
					part_type: "patch",
					text: diff,
					generated_at_unix_ms: generatedAtUnixMs,
				},
			],
		});

		void runDiffTraceHook(ctx.cwd, {
			sessionID: sessionId,
			diff,
			time: generatedAtUnixMs,
			model_id: ctx.model ? `${ctx.model.provider}/${ctx.model.id}` : null,
			tool_name: TOOL_NAME,
			tool_version: await piToolVersionPromise,
		});
	});

	pi.on("message_end", (event, ctx) => {
		const message = event.message;
		if (message.role !== "user" && message.role !== "assistant") {
			return;
		}

		const payload = buildMessageEndConversationTracePayload(
			ctx.sessionManager.getSessionId(),
			message,
		);
		if (payload) {
			void runConversationTraceHook(ctx.cwd, payload);
		}
	});
}
