import { beforeEach, describe, expect, mock, test } from "bun:test";

type SpawnCall = {
	command: string;
	args: string[];
	payload: Record<string, unknown>;
};

let spawnCalls: SpawnCall[] = [];
let mockResult: { status: number | null; error?: Error } = { status: 0 };

mock.module("node:child_process", () => ({
	spawnSync: (command: string, args: string[], options: { input?: string }) => {
		spawnCalls.push({
			command,
			args,
			payload: options.input ? JSON.parse(options.input) : {},
		});
		if (mockResult.error) {
			return { status: null, stdout: "", stderr: "", error: mockResult.error };
		}
		return {
			status: mockResult.status,
			stdout: "",
			stderr: "",
			error: undefined,
		};
	},
}));

const { SceMutationScopePlugin } = await import(
	"./opencode-sce-mutation-scope-plugin.ts"
);

function makePlugin() {
	return SceMutationScopePlugin(
		{ directory: "/repo", worktree: "/repo" } as never,
		undefined,
	);
}

beforeEach(() => {
	spawnCalls = [];
	mockResult = { status: 0 };
});

describe("SceMutationScopePlugin", () => {
	test("forwards a write tool.execute.before as a fail-closed ToolExecuteBefore", async () => {
		const plugin = await makePlugin();
		await plugin["chat.params"]?.(
			{
				sessionID: "ses_1",
				agent: "build",
				model: { providerID: "opencode", api: { id: "big-pickle" } },
			} as never,
			{} as never,
		);

		await plugin["tool.execute.before"]?.(
			{ tool: "write", sessionID: "ses_1", callID: "call_1" } as never,
			{ args: {} } as never,
		);

		expect(spawnCalls).toHaveLength(1);
		expect(spawnCalls[0].args).toEqual(["hooks", "opencode-mutation-scope"]);
		expect(spawnCalls[0].payload).toEqual({
			hook_event_name: "ToolExecuteBefore",
			session_id: "ses_1",
			call_id: "call_1",
			cwd: "/repo",
			tool_name: "write",
			model: "opencode/big-pickle",
		});
	});

	test("throws when the adapter exits non-zero on a write-ahead Start", async () => {
		mockResult = { status: 1 };
		const plugin = await makePlugin();

		let thrown: unknown;
		try {
			await plugin["tool.execute.before"]?.(
				{ tool: "edit", sessionID: "ses_1", callID: "call_2" } as never,
				{ args: {} } as never,
			);
		} catch (error) {
			thrown = error;
		}
		expect((thrown as Error).message).toBe(
			"SCE could not establish OpenCode mutation attribution for this tool execution.",
		);
	});

	test("fails closed when the sce CLI is missing", async () => {
		mockResult = {
			status: null,
			error: Object.assign(new Error("not found"), { code: "ENOENT" }),
		};
		const plugin = await makePlugin();

		let thrown: unknown;
		try {
			await plugin["tool.execute.before"]?.(
				{ tool: "apply_patch", sessionID: "ses_1", callID: "call_3" } as never,
				{ args: {} } as never,
			);
		} catch (error) {
			thrown = error;
		}
		expect(spawnCalls).toHaveLength(1);
		expect((thrown as Error).message).toBe(
			"SCE could not establish OpenCode mutation attribution for this tool execution.",
		);
	});

	test("ignores read and bash in tool.execute.before", async () => {
		const plugin = await makePlugin();
		await plugin["tool.execute.before"]?.(
			{ tool: "read", sessionID: "ses_1", callID: "call_4" } as never,
			{ args: {} } as never,
		);
		await plugin["tool.execute.before"]?.(
			{ tool: "bash", sessionID: "ses_1", callID: "call_5" } as never,
			{ args: {} } as never,
		);
		expect(spawnCalls).toHaveLength(0);
	});

	test("anchors the bash Start to shell.env", async () => {
		const plugin = await makePlugin();
		await plugin["shell.env"]?.(
			{ cwd: "/repo", sessionID: "ses_1", callID: "call_6" } as never,
			{ env: {} } as never,
		);
		expect(spawnCalls).toHaveLength(1);
		expect(spawnCalls[0].payload).toEqual({
			hook_event_name: "ShellEnv",
			session_id: "ses_1",
			call_id: "call_6",
			cwd: "/repo",
			model: null,
		});
	});

	test("shell.env with a missing sessionID fails closed without invoking the adapter", async () => {
		const plugin = await makePlugin();
		let thrown: unknown;
		try {
			await plugin["shell.env"]?.(
				{ cwd: "/repo", callID: "call_missing_session" } as never,
				{ env: {} } as never,
			);
		} catch (error) {
			thrown = error;
		}
		expect((thrown as Error).message).toBe(
			"SCE could not establish OpenCode mutation attribution for this tool execution.",
		);
		expect(spawnCalls).toHaveLength(0);
	});

	test("shell.env with an empty sessionID fails closed without invoking the adapter", async () => {
		const plugin = await makePlugin();
		let thrown: unknown;
		try {
			await plugin["shell.env"]?.(
				{ cwd: "/repo", sessionID: "", callID: "call_empty_session" } as never,
				{ env: {} } as never,
			);
		} catch (error) {
			thrown = error;
		}
		expect((thrown as Error).message).toBe(
			"SCE could not establish OpenCode mutation attribution for this tool execution.",
		);
		expect(spawnCalls).toHaveLength(0);
	});

	test("shell.env with a missing callID fails closed without invoking the adapter", async () => {
		const plugin = await makePlugin();
		let thrown: unknown;
		try {
			await plugin["shell.env"]?.(
				{ cwd: "/repo", sessionID: "ses_missing_call" } as never,
				{ env: {} } as never,
			);
		} catch (error) {
			thrown = error;
		}
		expect((thrown as Error).message).toBe(
			"SCE could not establish OpenCode mutation attribution for this tool execution.",
		);
		expect(spawnCalls).toHaveLength(0);
	});

	test("shell.env with an empty callID fails closed without invoking the adapter", async () => {
		const plugin = await makePlugin();
		let thrown: unknown;
		try {
			await plugin["shell.env"]?.(
				{ cwd: "/repo", sessionID: "ses_empty_call", callID: "" } as never,
				{ env: {} } as never,
			);
		} catch (error) {
			thrown = error;
		}
		expect((thrown as Error).message).toBe(
			"SCE could not establish OpenCode mutation attribution for this tool execution.",
		);
		expect(spawnCalls).toHaveLength(0);
	});

	test("shell.env failure fails closed", async () => {
		mockResult = { status: 3 };
		const plugin = await makePlugin();
		let thrown: unknown;
		try {
			await plugin["shell.env"]?.(
				{ cwd: "/repo", sessionID: "ses_1", callID: "call_7" } as never,
				{ env: {} } as never,
			);
		} catch (error) {
			thrown = error;
		}
		expect(thrown).toBeInstanceOf(Error);
	});

	test("tool.execute.after forwards a best-effort Close and never throws", async () => {
		mockResult = { status: 1 };
		const plugin = await makePlugin();
		await plugin["tool.execute.after"]?.(
			{
				tool: "bash",
				sessionID: "ses_1",
				callID: "call_8",
				args: {},
			} as never,
			{ title: "", output: "", metadata: {} } as never,
		);
		expect(spawnCalls).toHaveLength(1);
		expect(spawnCalls[0].payload.hook_event_name).toBe("ToolExecuteAfter");
	});

	test("a tool-part error event forwards a best-effort ToolError", async () => {
		const plugin = await makePlugin();
		await plugin.event?.({
			event: {
				type: "message.part.updated",
				properties: {
					part: {
						type: "tool",
						sessionID: "ses_1",
						callID: "call_9",
						tool: "write",
						state: { status: "error" },
					},
				},
			},
		} as never);
		expect(spawnCalls).toHaveLength(1);
		expect(spawnCalls[0].payload).toMatchObject({
			hook_event_name: "ToolError",
			session_id: "ses_1",
			call_id: "call_9",
			tool_name: "write",
		});
	});

	test("a later chat.params with no valid model clears the cached session model rather than reusing it", async () => {
		const plugin = await makePlugin();
		await plugin["chat.params"]?.(
			{
				sessionID: "ses_1",
				agent: "build",
				model: { providerID: "opencode", api: { id: "big-pickle" } },
			} as never,
			{} as never,
		);
		await plugin["chat.params"]?.(
			{
				sessionID: "ses_1",
				agent: "build",
				model: {},
			} as never,
			{} as never,
		);

		await plugin["tool.execute.before"]?.(
			{ tool: "write", sessionID: "ses_1", callID: "call_11" } as never,
			{ args: {} } as never,
		);
		expect(spawnCalls[0].payload.model).toBeNull();
	});

	test("the title agent model is not observed", async () => {
		const plugin = await makePlugin();
		await plugin["chat.params"]?.(
			{
				sessionID: "ses_1",
				agent: "title",
				model: { providerID: "opencode", api: { id: "tiny" } },
			} as never,
			{} as never,
		);
		await plugin["tool.execute.before"]?.(
			{ tool: "write", sessionID: "ses_1", callID: "call_10" } as never,
			{ args: {} } as never,
		);
		expect(spawnCalls[0].payload.model).toBeNull();
	});
});
