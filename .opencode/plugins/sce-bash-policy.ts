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
				return;
			}

			if (policyResult.decision === "deny" && policyResult.reason) {
				throw new Error(policyResult.reason);
			}
		},
	};
};
