import { spawn, spawnSync } from "node:child_process";
import { randomUUID } from "node:crypto";
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

export default function sceExtension(pi: ExtensionAPI): void {
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
