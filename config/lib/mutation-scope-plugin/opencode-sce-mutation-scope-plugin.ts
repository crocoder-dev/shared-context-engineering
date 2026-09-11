import { spawnSync } from "node:child_process";
import type { Hooks, Plugin } from "@opencode-ai/plugin";

type OpenCodeEvent = Parameters<NonNullable<Hooks["event"]>>[0]["event"];

const SCE_INSTALL_URL =
	"https://sce.crocoder.dev/docs/getting-started#install-cli";

const FAIL_CLOSED_MESSAGE =
	"SCE could not establish OpenCode mutation attribution for this tool execution.";

const ADAPTER_TIMEOUT_MS = 20_000;

const WRITE_AHEAD_START_TOOLS = new Set(["write", "edit", "apply_patch"]);
const CLOSE_TOOLS = new Set(["bash", "write", "edit", "apply_patch"]);

type AdapterPayload = {
	hook_event_name:
		| "ToolExecuteBefore"
		| "ShellEnv"
		| "ToolExecuteAfter"
		| "ToolError";
	session_id?: string;
	call_id?: string;
	cwd: string;
	tool_name?: string;
	model?: string | null;
};

type AdapterOutcome = "ok" | "failed" | "cli-missing";

function forwardToAdapter(payload: AdapterPayload): AdapterOutcome {
	let result: ReturnType<typeof spawnSync>;
	try {
		result = spawnSync("sce", ["hooks", "opencode-mutation-scope"], {
			input: JSON.stringify(payload),
			encoding: "utf8",
			timeout: ADAPTER_TIMEOUT_MS,
		});
	} catch {
		return "failed";
	}

	if (result.error) {
		if ((result.error as NodeJS.ErrnoException).code === "ENOENT") {
			console.warn(`sce CLI not found. Install it from ${SCE_INSTALL_URL}`);
			return "cli-missing";
		}
		return "failed";
	}

	return result.status === 0 ? "ok" : "failed";
}

function forwardFailClosed(payload: AdapterPayload): void {
	if (forwardToAdapter(payload) !== "ok") {
		throw new Error(FAIL_CLOSED_MESSAGE);
	}
}

function forwardBestEffort(payload: AdapterPayload): void {
	forwardToAdapter(payload);
}

function toolErrorPart(
	event: OpenCodeEvent,
): { sessionID: string; callID: string; tool: string } | undefined {
	if (event.type !== "message.part.updated") {
		return undefined;
	}
	const part = event.properties.part;
	if (part.type !== "tool" || part.state.status !== "error") {
		return undefined;
	}
	if (
		typeof part.sessionID !== "string" ||
		typeof part.callID !== "string" ||
		typeof part.tool !== "string" ||
		part.sessionID.length === 0 ||
		part.callID.length === 0 ||
		part.tool.length === 0
	) {
		return undefined;
	}
	return { sessionID: part.sessionID, callID: part.callID, tool: part.tool };
}

export const SceMutationScopePlugin: Plugin = async ({
	directory,
	worktree,
}) => {
	const repoRoot = worktree ?? directory ?? process.cwd();
	const observedModelBySessionId: Map<string, string> = new Map();

	return {
		"chat.params": async (input) => {
			if (input.agent === "title") {
				return;
			}
			const providerId = input.model?.providerID;
			const apiId = input.model?.api?.id;
			if (
				typeof providerId === "string" &&
				providerId.length > 0 &&
				typeof apiId === "string" &&
				apiId.length > 0
			) {
				observedModelBySessionId.set(input.sessionID, `${providerId}/${apiId}`);
			} else {
				observedModelBySessionId.delete(input.sessionID);
			}
		},

		"tool.execute.before": async (input) => {
			if (!WRITE_AHEAD_START_TOOLS.has(input.tool)) {
				return;
			}
			forwardFailClosed({
				hook_event_name: "ToolExecuteBefore",
				session_id: input.sessionID,
				call_id: input.callID,
				cwd: repoRoot,
				tool_name: input.tool,
				model: observedModelBySessionId.get(input.sessionID) ?? null,
			});
		},

		"shell.env": async (input) => {
			if (
				typeof input.sessionID !== "string" ||
				typeof input.callID !== "string" ||
				input.sessionID.length === 0 ||
				input.callID.length === 0
			) {
				throw new Error(FAIL_CLOSED_MESSAGE);
			}
			forwardFailClosed({
				hook_event_name: "ShellEnv",
				session_id: input.sessionID,
				call_id: input.callID,
				cwd: repoRoot,
				model: observedModelBySessionId.get(input.sessionID) ?? null,
			});
		},

		"tool.execute.after": async (input) => {
			if (!CLOSE_TOOLS.has(input.tool)) {
				return;
			}
			forwardBestEffort({
				hook_event_name: "ToolExecuteAfter",
				session_id: input.sessionID,
				call_id: input.callID,
				cwd: repoRoot,
				tool_name: input.tool,
			});
		},

		event: async ({ event }) => {
			const errorPart = toolErrorPart(event);
			if (errorPart !== undefined) {
				forwardBestEffort({
					hook_event_name: "ToolError",
					session_id: errorPart.sessionID,
					call_id: errorPart.callID,
					cwd: repoRoot,
					tool_name: errorPart.tool,
				});
				return;
			}

			if (event.type === "session.deleted") {
				observedModelBySessionId.delete(event.properties.info.id);
			}
		},
	};
};
