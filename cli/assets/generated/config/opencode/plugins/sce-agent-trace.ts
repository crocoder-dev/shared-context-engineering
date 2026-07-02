import { spawn } from "node:child_process";
import type { Hooks, Plugin } from "@opencode-ai/plugin";

type OpenCodeEvent = Parameters<NonNullable<Hooks["event"]>>[0]["event"];

const SCE_INSTALL_URL =
	"https://sce.crocoder.dev/docs/getting-started#install-cli";

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

type ConversationTraceMessageUpdatedItem = {
	type: "message";
	session_id: string;
	message_id: string;
	role: EventMessageUpdated["properties"]["info"]["role"];
	generated_at_unix_ms: number;
};

type ConversationTraceMessagePartUpdatedItem = {
	type: "message.part";
	session_id: string;
	message_id: string;
	part_type: "text" | "reasoning" | "patch" | "question";
	text: unknown;
	generated_at_unix_ms: number;
};

type ConversationTraceItem =
	| ConversationTraceMessageUpdatedItem
	| ConversationTraceMessagePartUpdatedItem;

type ConversationTracePayload = {
	payloads: ConversationTraceItem[];
};

type QuestionToolAnswer = {
	question: string;
	answer: string;
};

const QUESTION_TOOL_ANSWER_SEPARATOR = ", ";

type EventMessageUpdated = Extract<
	NonNullable<TraceInput["event"]>,
	{ type: "message.updated" }
>;

type EventMessagePartUpdated = Extract<
	NonNullable<TraceInput["event"]>,
	{ type: "message.part.updated" }
>;

type EventMessagePart = EventMessagePartUpdated["properties"]["part"];
type EventMessageToolPart = Extract<EventMessagePart, { type: "tool" }>;
type EventAllowedPart =
	| Extract<EventMessagePart, { type: "text" }>
	| Extract<EventMessagePart, { type: "reasoning" }>;

function extractDiffEntries(
	eventInfo: EventMessageUpdated["properties"]["info"],
) {
	if (typeof eventInfo.summary === "object") {
		return eventInfo.summary.diffs;
	}
	return undefined;
}

function extractDiffTracePayload(
	event: EventMessageUpdated,
): DiffTracePayload | undefined {
	const eventInfo = event.properties.info;
	// Only capture user messages (filter out assistant, system, etc.)
	if (eventInfo.role !== "user") {
		return undefined;
	}

	const diffEntries = extractDiffEntries(eventInfo);

	if (!diffEntries || diffEntries.length === 0) {
		return undefined;
	}

	const patches: string[] = [];
	for (const entry of diffEntries) {
		if ("patch" in entry && typeof entry.patch === "string") {
			patches.push(entry.patch);
		}
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

function extractQuestionToolAnswers(
	eventPart: EventMessageToolPart,
): QuestionToolAnswer[] | undefined {
	const state = eventPart.state;

	if (state.status !== "completed") {
		return undefined;
	}

	const questions =
		"questions" in state.input && Array.isArray(state.input.questions)
			? state.input.questions
			: [];
	const answers =
		"answers" in state.metadata && Array.isArray(state.metadata.answers)
			? state.metadata.answers
			: [];

	if (questions.length === 0 || questions.length !== answers.length) {
		return undefined;
	}

	const result: QuestionToolAnswer[] = [];

	questions.forEach((q, index) => {
		const question =
			"question" in q && typeof q.question === "string" ? q.question : "";
		if (question) {
			const answer = Array.isArray(answers[index]) ? answers[index] : [];
			result.push({
				question,
				answer: answer.join(QUESTION_TOOL_ANSWER_SEPARATOR),
			});
		}
	});

	return result;
}

function buildConversationTracePayload(
	event: EventMessageUpdated,
): ConversationTracePayload {
	const eventInfo = event.properties.info;

	return {
		payloads: [
			{
				type: "message",
				session_id: eventInfo.sessionID,
				message_id: eventInfo.id,
				role: eventInfo.role,
				generated_at_unix_ms: Date.now(),
			},
		],
	};
}

export function buildMessagePartConversationTracePayload(
	eventPart: EventAllowedPart,
): ConversationTracePayload {
	return {
		payloads: [
			{
				type: "message.part",
				session_id: eventPart.sessionID,
				message_id: eventPart.messageID,
				part_type: eventPart.type,
				text: "text" in eventPart ? eventPart.text : "",
				generated_at_unix_ms: Date.now(),
			},
		],
	};
}

function buildQuestionToolConversationTracePayload(
	eventPart: EventMessageToolPart,
): ConversationTracePayload | undefined {
	const pairedAnswers = extractQuestionToolAnswers(eventPart);

	if (pairedAnswers === undefined) {
		return undefined;
	}

	return {
		payloads: [
			{
				type: "message.part",
				session_id: eventPart.sessionID,
				message_id: eventPart.messageID,
				part_type: "question",
				text: JSON.stringify(pairedAnswers),
				generated_at_unix_ms: Date.now(),
			},
		],
	};
}

function buildPatchConversationTracePayload(
	event: EventMessageUpdated,
): ConversationTracePayload | undefined {
	const eventInfo = event.properties.info;
	const diffEntries = extractDiffEntries(eventInfo);

	if (!diffEntries || diffEntries.length === 0) {
		return undefined;
	}

	const patchMessageId = `${eventInfo.id}-patch`;
	const payloads: ConversationTraceItem[] = [];

	payloads.push({
		type: "message",
		session_id: eventInfo.sessionID,
		message_id: patchMessageId,
		role: eventInfo.role,
		generated_at_unix_ms: Date.now(),
	});

	for (const entry of diffEntries) {
		if ("patch" in entry && typeof entry.patch === "string") {
			payloads.push({
				type: "message.part",
				session_id: eventInfo.sessionID,
				message_id: patchMessageId,
				part_type: "patch",
				text: entry.patch,
				generated_at_unix_ms: Date.now(),
			});
		}
	}

	return { payloads };
}

export async function recordConversationTrace(
	repoRoot: string,
	event: EventMessageUpdated | EventMessagePartUpdated,
): Promise<void> {
	if (
		event.type === "message.part.updated" &&
		event.properties.part.type === "tool" &&
		event.properties.part.tool === "question"
	) {
		const questionToolPayload = buildQuestionToolConversationTracePayload(
			event.properties.part,
		);
		if (questionToolPayload !== undefined) {
			await runConversationTraceHook(repoRoot, questionToolPayload);
			return;
		}
	}

	if (
		event.type === "message.part.updated" &&
		(event.properties.part.type === "reasoning" ||
			event.properties.part.type === "text") &&
		event.properties.part.text
	) {
		await runConversationTraceHook(
			repoRoot,
			buildMessagePartConversationTracePayload(event.properties.part),
		);
		return;
	}

	if (event.type === "message.updated") {
		const patchPayload = buildPatchConversationTracePayload(event);

		if (patchPayload !== undefined) {
			await runConversationTraceHook(repoRoot, patchPayload);
			return;
		}

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
	await new Promise<void>((resolve) => {
		const child = spawn("sce", ["hooks", "diff-trace"], {
			cwd: repoRoot,
			// Fail-open: stderr is ignored so that sce intake errors
			// (connection refused, timeout, etc.) do not leak into the
			// OpenCode TUI. Resolve unconditionally on any outcome.
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

async function runConversationTraceHook(
	repoRoot: string,
	payload: ConversationTracePayload,
): Promise<void> {
	await new Promise<void>((resolve) => {
		const child = spawn("sce", ["hooks", "conversation-trace"], {
			cwd: repoRoot,
			// Fail-open: stderr is ignored so that sce intake errors
			// (connection refused, timeout, etc.) do not leak into the
			// OpenCode TUI. Resolve unconditionally on any outcome.
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

export const SceAgentTracePlugin: Plugin = async ({ directory, worktree }) => {
	const repoRoot = worktree ?? directory ?? process.cwd();
	const clientVersionsBySessionId: Map<string, string> = new Map();
	const processedDiffsMessageIds: Set<string> = new Set();

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
				const eventInfo = input.event.properties.info;
				const diffEntries = extractDiffEntries(eventInfo);
				const hasDiffs = diffEntries !== undefined && diffEntries.length > 0;

				if (hasDiffs) {
					const dedupKey = `${eventInfo.sessionID}:${eventInfo.id}`;
					if (processedDiffsMessageIds.has(dedupKey)) {
						return;
					}
					processedDiffsMessageIds.add(dedupKey);
				}

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
