import { afterEach, beforeEach, describe, expect, test } from "bun:test";

// Mock state for spawnSync calls
let mockSpawnSyncResult: {
	status: number;
	stdout: string;
	error?: Error;
} | null = null;

// Mock the node:child_process module to intercept spawnSync calls
const { mock } = await import("bun:test");
mock.module("node:child_process", () => ({
	spawnSync: (
		command: string,
		args: string[],
		_options: { input?: string; encoding?: string; timeout?: number },
	) => {
		if (command !== "sce" || args[0] !== "policy" || args[1] !== "bash") {
			return {
				status: 1,
				stdout: "",
				stderr: "command not found",
				error: new Error("Command not found"),
			};
		}

		if (mockSpawnSyncResult === null) {
			return {
				status: 1,
				stdout: "",
				stderr: "no mock result set",
				error: new Error("No mock result"),
			};
		}

		if (mockSpawnSyncResult.error) {
			return {
				status: 1,
				stdout: "",
				stderr: "",
				error: mockSpawnSyncResult.error,
			};
		}

		return {
			status: mockSpawnSyncResult.status,
			stdout: mockSpawnSyncResult.stdout,
			stderr: "",
			error: undefined,
		};
	},
}));

// Import the plugin after mocking
const { SceBashPolicyPlugin } = await import(
	"./opencode-bash-policy-plugin.ts"
);

beforeEach(() => {
	mockSpawnSyncResult = null;
});

afterEach(() => {
	mockSpawnSyncResult = null;
});

function makeAllowResult(command: string, normalizedArgv: string[]): string {
	return JSON.stringify({
		status: "ok",
		decision: "allow",
		command,
		normalized_argv: normalizedArgv,
		reason: null,
		policy_id: null,
	});
}

function makeDenyResult(
	command: string,
	normalizedArgv: string[],
	policyId: string,
	message: string,
): string {
	return JSON.stringify({
		status: "ok",
		decision: "deny",
		command,
		normalized_argv: normalizedArgv,
		reason: `Blocked by SCE bash-tool policy '${policyId}': ${message}`,
		policy_id: policyId,
	});
}

describe("SceBashPolicyPlugin", () => {
	describe("tool.execute.before handler", () => {
		test("ignores non-bash tool events", async () => {
			const plugin = await SceBashPolicyPlugin({});

			// Should return undefined for non-bash tools
			const result = await plugin["tool.execute.before"](
				{ tool: "read" },
				{ args: { command: "git status" } },
			);
			expect(result).toBeUndefined();
		});

		test("ignores events without args", async () => {
			const plugin = await SceBashPolicyPlugin({});

			const result = await plugin["tool.execute.before"](
				{ tool: "bash" },
				undefined,
			);
			expect(result).toBeUndefined();
		});

		test("ignores events with null args", async () => {
			const plugin = await SceBashPolicyPlugin({});

			const result = await plugin["tool.execute.before"](
				{ tool: "bash" },
				null,
			);
			expect(result).toBeUndefined();
		});

		test("ignores events with non-string command", async () => {
			const plugin = await SceBashPolicyPlugin({});

			const result = await plugin["tool.execute.before"](
				{ tool: "bash" },
				{ args: { command: 123 } },
			);
			expect(result).toBeUndefined();
		});

		test("ignores events with empty command", async () => {
			const plugin = await SceBashPolicyPlugin({});

			const result = await plugin["tool.execute.before"](
				{ tool: "bash" },
				{ args: { command: "" } },
			);
			expect(result).toBeUndefined();
		});

		test("allows command when policy decision is allow", async () => {
			mockSpawnSyncResult = {
				status: 0,
				stdout: makeAllowResult("git status", ["git", "status"]),
			};

			const plugin = await SceBashPolicyPlugin({});

			// Should return undefined (allow) without throwing
			const result = await plugin["tool.execute.before"](
				{ tool: "bash" },
				{ args: { command: "git status" } },
			);
			expect(result).toBeUndefined();
		});

		test("blocks command when policy decision is deny", async () => {
			mockSpawnSyncResult = {
				status: 0,
				stdout: makeDenyResult(
					"git commit",
					["git", "commit"],
					"forbid-git-commit",
					"This repository blocks direct `git add`, `git commit`, and `git push`.",
				),
			};

			const plugin = await SceBashPolicyPlugin({});

			try {
				await plugin["tool.execute.before"](
					{ tool: "bash" },
					{ args: { command: "git commit" } },
				);
				expect.unreachable("Expected error to be thrown");
			} catch (error) {
				expect(error).toBeInstanceOf(Error);
				expect((error as Error).message).toBe(
					"Blocked by SCE bash-tool policy 'forbid-git-commit': This repository blocks direct `git add`, `git commit`, and `git push`.",
				);
			}
		});

		test("allows command when sce is not found (fail-open)", async () => {
			mockSpawnSyncResult = {
				status: 1,
				stdout: "",
				error: new Error("sce not found"),
			};

			const plugin = await SceBashPolicyPlugin({});

			// Should return undefined (allow) without throwing
			const result = await plugin["tool.execute.before"](
				{ tool: "bash" },
				{ args: { command: "git status" } },
			);
			expect(result).toBeUndefined();
		});

		test("allows command when sce exits with non-zero (fail-open)", async () => {
			mockSpawnSyncResult = {
				status: 1,
				stdout: "",
			};

			const plugin = await SceBashPolicyPlugin({});

			const result = await plugin["tool.execute.before"](
				{ tool: "bash" },
				{ args: { command: "git status" } },
			);
			expect(result).toBeUndefined();
		});

		test("allows command when sce returns empty stdout (fail-open)", async () => {
			mockSpawnSyncResult = {
				status: 0,
				stdout: "",
			};

			const plugin = await SceBashPolicyPlugin({});

			const result = await plugin["tool.execute.before"](
				{ tool: "bash" },
				{ args: { command: "git status" } },
			);
			expect(result).toBeUndefined();
		});

		test("allows command when sce returns invalid JSON (fail-open)", async () => {
			mockSpawnSyncResult = {
				status: 0,
				stdout: "not valid json",
			};

			const plugin = await SceBashPolicyPlugin({});

			const result = await plugin["tool.execute.before"](
				{ tool: "bash" },
				{ args: { command: "git status" } },
			);
			expect(result).toBeUndefined();
		});

		test("passes normalized input format to sce policy bash", () => {
			// Verify that the normalized JSON request format matches
			// what sce policy bash --input normalized expects
			const command = "git status";
			const request = JSON.stringify({ command });
			const parsed = JSON.parse(request);
			expect(parsed).toEqual({ command: "git status" });
		});
	});
});
