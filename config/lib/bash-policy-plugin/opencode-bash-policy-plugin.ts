import { spawnSync } from "node:child_process";
import type { Plugin } from "@opencode-ai/plugin";

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

export const SceBashPolicyPlugin: Plugin = async () => {
	return {
		"tool.execute.before": async (input, output) => {
			if (input.tool !== "bash") {
				return;
			}

			const args = output?.args;
			if (args === undefined || args === null) {
				return;
			}

			const command = (args as { command?: unknown }).command;
			if (typeof command !== "string" || command.length === 0) {
				return;
			}

			const policyResult = evaluateBashCommandPolicy(command);
			if (!policyResult) {
				// Fail open: if the policy check cannot be performed, allow the command.
				return;
			}

			if (policyResult.decision === "deny" && policyResult.reason) {
				throw new Error(policyResult.reason);
			}
		},
	};
};
