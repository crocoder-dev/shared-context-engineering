import { spawn } from "node:child_process";

import type { Hooks, Plugin } from "@opencode-ai/plugin";

type OpenCodeEvent = Parameters<NonNullable<Hooks["event"]>>[0]["event"];

const REQUIRED_EVENTS: Set<OpenCodeEvent["type"]> = new Set([
	"message.updated",
	"message.part.updated",
	"session.created",
	"session.updated",
]);

const ALL_CAPTURED_EVENTS = REQUIRED_EVENTS;

type TraceInput = {
	event?: OpenCodeEvent;
};

type DiffTracePayload = {
	sessionID: string;
	diff: string;
	time: number;
	model_id: string;
};

type ConversationTraceMessageUpdatedPayload = {
	type: "message.updated";
	session_id: string;
	message_id: string;
	role: EventMessageUpdated["properties"]["info"]["role"];
	agent: unknown;
	summary_diffs: unknown[];
	generated_at_unix_ms: number;
};

type ConversationTraceMessagePartUpdatedPayload = {
	type: "message.part.updated";
	session_id: string;
	message_id: string;
	part_type: EventMessagePartUpdated["properties"]["part"]["type"];
	text: unknown;
	generated_at_unix_ms: number;
};

type ConversationTracePayload =
	| ConversationTraceMessageUpdatedPayload
	| ConversationTraceMessagePartUpdatedPayload;

type EventMessageUpdated = Extract<
	NonNullable<TraceInput["event"]>,
	{ type: "message.updated" }
>;

type EventMessagePartUpdated = Extract<
	NonNullable<TraceInput["event"]>,
	{ type: "message.part.updated" }
>;

function extractDiffTracePayload(
	event: EventMessageUpdated,
): DiffTracePayload | undefined {
	const eventInfo = event.properties.info;
	// Only capture user messages (filter out assistant, system, etc.)
	if (eventInfo.role !== "user") {
		return undefined;
	}

	// Access info.summary?.diffs via explicit checks
	const diffEntries = eventInfo.summary?.diffs;

	if (!diffEntries || diffEntries.length === 0) {
		return undefined;
	}

	const patches: string[] = [];
	for (const entry of diffEntries) {
		const entryObj = entry as { patch?: string };

		if (!entryObj.patch) {
			continue;
		}

		patches.push(entryObj.patch);
	}

	if (patches.length === 0) {
		return undefined;
	}

	return {
		sessionID: eventInfo.sessionID,
		diff: patches.join("\n"),
		time: Date.now(),
		model_id: `${eventInfo.model.providerID}/${eventInfo.model.modelID}`,
	};
}

function shouldCaptureEvent(eventType: OpenCodeEvent["type"]): boolean {
	return ALL_CAPTURED_EVENTS.has(eventType);
}

function buildConversationTracePayload(
	event: EventMessageUpdated,
): ConversationTraceMessageUpdatedPayload {
	const eventInfo = event.properties.info;

	return {
		type: "message.updated",
		session_id: eventInfo.sessionID,
		message_id: eventInfo.id,
		role: eventInfo.role,
		agent: "agent" in eventInfo ? eventInfo.agent : null,
		summary_diffs:
			eventInfo.role === "user" ? (eventInfo.summary?.diffs ?? []) : [],
		generated_at_unix_ms: Date.now(),
	};
}

export function buildMessagePartConversationTracePayload(
	event: EventMessagePartUpdated,
): ConversationTraceMessagePartUpdatedPayload {
	const eventPart = event.properties.part;

	return {
		type: "message.part.updated",
		session_id: eventPart.sessionID,
		message_id: eventPart.messageID,
		part_type: eventPart.type,
		text: "text" in eventPart ? eventPart.text : "",
		generated_at_unix_ms: Date.now(),
	};
}

export async function recordConversationTrace(
	repoRoot: string,
	event: EventMessageUpdated | EventMessagePartUpdated,
): Promise<void> {
	if (
		event.type === "message.part.updated" &&
		(event.properties.part.type === "reasoning" ||
			event.properties.part.type === "text") &&
		event.properties.part.text
	) {
		await runConversationTraceHook(
			repoRoot,
			buildMessagePartConversationTracePayload(event),
		);
		return;
	}

	if (event.type === "message.updated") {
		await runConversationTraceHook(
			repoRoot,
			buildConversationTracePayload(event),
		);
	}
}

async function buildTrace(
	repoRoot: string,
	event: EventMessageUpdated,
	clientVersion: string | null,
): Promise<void> {
	const diffTracePayload = extractDiffTracePayload(event);

	if (diffTracePayload === undefined) {
		return;
	}

	await runDiffTraceHook(repoRoot, {
		...diffTracePayload,
		tool_name: "opencode",
		tool_version: clientVersion,
	});
}

async function runDiffTraceHook(
	repoRoot: string,
	payload: DiffTracePayload & {
		tool_name: string;
		tool_version: string | null;
	},
): Promise<void> {
	await new Promise<void>((resolve, reject) => {
		const child = spawn("sce", ["hooks", "diff-trace"], {
			cwd: repoRoot,
			stdio: ["pipe", "ignore", "inherit"],
		});

		child.on("error", reject);

		child.on("close", (code, signal) => {
			if (code === 0) {
				resolve();
				return;
			}

			const reason =
				signal === null ? `exit code ${String(code)}` : `signal ${signal}`;
			reject(
				new Error(`Command 'sce hooks diff-trace' failed with ${reason}.`),
			);
		});

		child.stdin.end(`${JSON.stringify(payload)}\n`);
	});
}

async function runConversationTraceHook(
	repoRoot: string,
	payload: ConversationTracePayload,
): Promise<void> {
	await new Promise<void>((resolve, reject) => {
		const child = spawn("sce", ["hooks", "conversation-trace"], {
			cwd: repoRoot,
			stdio: ["pipe", "ignore", "inherit"],
		});

		child.on("error", reject);

		child.on("close", (code, signal) => {
			if (code === 0) {
				resolve();
				return;
			}

			const reason =
				signal === null ? `exit code ${String(code)}` : `signal ${signal}`;
			reject(
				new Error(
					`Command 'sce hooks conversation-trace' failed with ${reason}.`,
				),
			);
		});

		child.stdin.end(`${JSON.stringify(payload)}\n`);
	});
}

export const SceAgentTracePlugin: Plugin = async ({ directory, worktree }) => {
	const repoRoot = worktree ?? directory ?? process.cwd();
	const clientVersionsBySessionId: Map<string, string> = new Map();

	return {
		event: async (input) => {
			if (!shouldCaptureEvent(input.event.type)) {
				return;
			}

			if (
				input.event.type === "session.created" ||
				input.event.type === "session.updated"
			) {
				clientVersionsBySessionId.set(
					input.event.properties.info.id,
					input.event.properties.info.version,
				);
			}

			if (input.event.type === "message.updated") {
				const clientVersion =
					clientVersionsBySessionId.get(
						input.event.properties.info.sessionID,
					) || null;
				await recordConversationTrace(repoRoot, input.event);
				await buildTrace(repoRoot, input.event, clientVersion);
			}

			if (input.event.type === "message.part.updated") {
				await recordConversationTrace(repoRoot, input.event);
			}
		},
	};
};
