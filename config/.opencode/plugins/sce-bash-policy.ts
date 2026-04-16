import { spawn } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";
import type { Plugin } from "@opencode-ai/plugin";

import {
	evaluateBashCommandPolicy,
	formatPolicyBlockMessage,
} from "./bash-policy/runtime.ts";

export const SceBashPolicyPlugin: Plugin = async ({ directory, worktree }) => {
	const pluginDirectory = path.dirname(fileURLToPath(import.meta.url));
	const repoRoot = worktree ?? directory ?? process.cwd();

	return {
		"chat.message": async (_input, output) => {
			const promptText = collectPromptText(output.parts);
			if (!promptText) {
				return;
			}

			const result = await runTraceAppendPrompt(repoRoot, promptText);
			if (result.ok) {
				return;
			}

			console.warn(
				`SCE trace append-prompt failed (exit=${result.exitCode}): ${result.errorMessage}`,
			);
		},

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

			const result = await evaluateBashCommandPolicy({
				command,
				worktree: repoRoot,
				pluginDirectory,
			});
			if (result.allowed) {
				return;
			}

			throw new Error(formatPolicyBlockMessage(result.policy));
		},
	};
};

function collectPromptText(parts: Array<{ type?: unknown; text?: unknown }>): string | null {
	const textParts = parts
		.filter((part) => part.type === "text" && typeof part.text === "string")
		.map((part) => part.text.trim())
		.filter((text) => text.length > 0);

	if (textParts.length === 0) {
		return null;
	}

	return textParts.join("\n");
}

async function runTraceAppendPrompt(
	repoRoot: string,
	promptText: string,
): Promise<
	| { ok: true }
	| { ok: false; exitCode: number | null; errorMessage: string }
> {
	return await new Promise((resolve) => {
		const child = spawn(
			"sce",
			["trace", "append-prompt", "--prompt", promptText],
			{
				cwd: repoRoot,
				stdio: ["ignore", "pipe", "pipe"],
			},
		);

		let stderr = "";
		child.stderr.on("data", (chunk) => {
			stderr += chunk.toString();
		});

		child.on("error", (error) => {
			resolve({
				ok: false,
				exitCode: null,
				errorMessage: error.message,
			});
		});

		child.on("close", (code) => {
			if (code === 0) {
				resolve({ ok: true });
				return;
			}

			resolve({
				ok: false,
				exitCode: code,
				errorMessage:
					stderr.trim().length > 0
						? stderr.trim()
						: "trace append-prompt returned a non-zero exit code",
			});
		});
	});
}
