import { spawn, spawnSync } from "node:child_process";
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
	payloads: ConversationTraceItem[];
};

type DiffTracePayload = {
	sessionID: string;
	diff: string;
	time: number;
	model_id: string | null;
	tool_name: "pi";
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
		const child = spawn("sce", ["hooks", "conversation-trace"], {
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

	return { payloads };
}

/**
 * Resolve the installed Pi package version for diff-trace `tool_version`.
 * The package's `exports` map does not expose `package.json`, so resolve the
 * package entry point and read `package.json` from the package root instead.
 * Returns null when resolution fails (normalized diff traces permit it).
 */
async function resolvePiToolVersion(): Promise<string | null> {
	try {
		const require_ = createRequire(import.meta.url);
		const entryPath = require_.resolve("@earendil-works/pi-coding-agent");
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
		const child = spawn("sce", ["hooks", "diff-trace"], {
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

export default function sceExtension(pi: ExtensionAPI): void {
	const pendingFileMutations = new Map<string, PendingFileMutation>();
	const piToolVersionPromise = resolvePiToolVersion();

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
			tool_name: "pi",
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
