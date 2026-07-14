import { spawnSync } from "node:child_process";
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
}
