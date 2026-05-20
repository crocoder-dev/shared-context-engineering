import { spawn } from "node:child_process";
import type { Hooks, Plugin } from "@opencode-ai/plugin";

type OpenCodeEvent = Parameters<NonNullable<Hooks["event"]>>[0]["event"];

const REQUIRED_EVENTS = new Set(["message.updated"]);

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

type EventMessageUpdated = Extract<
	NonNullable<TraceInput["event"]>,
	{ type: "message.updated" }
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

function shouldCaptureEvent(eventType: string): boolean {
	return ALL_CAPTURED_EVENTS.has(eventType);
}

async function buildTrace(
	repoRoot: string,
	input: TraceInput,
	clientVersion: string | null,
): Promise<void> {
	if (input.event?.type !== "message.updated") {
		return;
	}
	const diffTracePayload = extractDiffTracePayload(input.event);

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

export const SceAgentTracePlugin: Plugin = async ({ directory, worktree }) => {
	const repoRoot = worktree ?? directory ?? process.cwd();
	const clientVersionsBySessionId: Map<string, string> = new Map();

	return {
		event: async (input) => {
			const eventType =
				typeof input.event === "object" &&
				input.event !== null &&
				typeof input.event.type === "string"
					? input.event.type
					: undefined;

			if (eventType === undefined) {
				return;
			}

			let clientVersion: string | null = null;

			if (
				input.event.type === "session.created" ||
				input.event.type === "session.updated"
			) {
				clientVersion =
					clientVersionsBySessionId.get(input.event.properties.info.id) || null;

				if (!clientVersion) {
					clientVersion = input.event.properties.info.version;
					clientVersionsBySessionId.set(
						input.event.properties.info.id,
						clientVersion,
					);
				}
			}

			if (!shouldCaptureEvent(eventType)) {
				return;
			}

			await buildTrace(repoRoot, input, clientVersion);
		},
	};
};
