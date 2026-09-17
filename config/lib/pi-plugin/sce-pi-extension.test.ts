import {
	afterAll,
	afterEach,
	beforeEach,
	describe,
	expect,
	mock,
	test,
} from "bun:test";
import { EventEmitter } from "node:events";
import { createRequire } from "node:module";
import type {
	AttemptKey,
	PiMutationScopePayload,
	RetryScheduleFn,
} from "./sce-pi-extension.ts";

type SpawnSyncCall = {
	command: string;
	args: string[];
	payload: Record<string, unknown> | undefined;
};

type SpawnCall = {
	command: string;
	args: string[];
	child: FakeChild;
};

class FakeChild extends EventEmitter {
	stdin = {
		writes: [] as string[],
		write: (data: string) => {
			this.stdin.writes.push(data);
			return true;
		},
		end: (data?: string) => {
			if (data) {
				this.stdin.writes.push(data);
			}
		},
	};
	stdout = new EventEmitter();

	emitLine(payload: Record<string, unknown>): void {
		this.stdout.emit("data", Buffer.from(`${JSON.stringify(payload)}\n`));
	}

	killed = false;
	kill(): void {
		this.killed = true;
	}
}

let spawnSyncCalls: SpawnSyncCall[] = [];
let spawnSyncResult: {
	status: number | null;
	error?: NodeJS.ErrnoException;
} = { status: 0 };

let spawnCalls: SpawnCall[] = [];

mock.module("node:child_process", () => ({
	spawnSync: (command: string, args: string[], options: { input?: string }) => {
		spawnSyncCalls.push({
			command,
			args,
			payload: options.input ? JSON.parse(options.input) : undefined,
		});
		if (spawnSyncResult.error) {
			return {
				status: null,
				stdout: "",
				stderr: "",
				error: spawnSyncResult.error,
			};
		}
		return {
			status: spawnSyncResult.status,
			stdout: "",
			stderr: "",
			error: undefined,
		};
	},
}));

const realChildProcess = createRequire(import.meta.url)("node:child_process");
const originalSpawn = realChildProcess.spawn;
realChildProcess.spawn = (command: string, args: string[]) => {
	const child = new FakeChild();
	spawnCalls.push({ command, args, child });
	return child;
};
afterAll(() => {
	realChildProcess.spawn = originalSpawn;
});

mock.module("node:fs/promises", () => ({
	readdir: async (dir: string) => {
		if (dir.endsWith("/.pi/extensions")) {
			return ["sce"];
		}
		return [];
	},
	readFile: async () => {
		throw new Error("not used in these tests");
	},
	writeFile: async () => {},
	mkdtemp: async (prefix: string) => `${prefix}fake`,
	rm: async () => {},
}));

mock.module("@earendil-works/pi-coding-agent", () => ({
	isToolCallEventType: (toolName: string, event: { toolName: string }) =>
		event.toolName === toolName,
}));

const { default: sceExtension, createTerminalDeliveryTracker } = await import(
	"./sce-pi-extension.ts"
);

type Handler = (event: unknown, ctx?: unknown) => unknown;

function makeApi() {
	const handlers = new Map<string, Handler[]>();
	const api = {
		on: (event: string, handler: Handler) => {
			const list = handlers.get(event) ?? [];
			list.push(handler);
			handlers.set(event, list);
		},
	};
	return { api, handlers };
}

function flush(): Promise<void> {
	return new Promise((resolve) => setImmediate(resolve));
}

function ctxFor(cwd: string, sessionId = "ses_1") {
	return {
		cwd,
		sessionManager: { getSessionId: () => sessionId },
		model: undefined,
	};
}

beforeEach(() => {
	spawnSyncCalls = [];
	spawnSyncResult = { status: 0 };
	spawnCalls = [];
});

describe("mutation-scope tool_call Start", () => {
	test("does not call the adapter for a read-only tool", async () => {
		const { api, handlers } = makeApi();
		sceExtension(api as never);
		const [, startHandler] = handlers.get("tool_call") ?? [];
		await startHandler?.(
			{ toolName: "read", toolCallId: "c1" },
			ctxFor("/repo"),
		);
		expect(spawnSyncCalls).toHaveLength(0);
	});

	test("forwards a tracked bash tool_call as ToolCall and allows on success", async () => {
		const { api, handlers } = makeApi();
		sceExtension(api as never);
		const [, startHandler] = handlers.get("tool_call") ?? [];
		const result = await startHandler?.(
			{ toolName: "bash", toolCallId: "c1" },
			ctxFor("/repo", "ses_a"),
		);
		expect(result).toBeUndefined();
		expect(spawnSyncCalls).toHaveLength(1);
		expect(spawnSyncCalls[0].args).toEqual(["hooks", "pi-mutation-scope"]);
		expect(spawnSyncCalls[0].payload).toEqual({
			hook_event_name: "ToolCall",
			session_id: "ses_a",
			tool_call_id: "c1",
			cwd: "/repo",
			tool_name: "bash",
			model: undefined,
		});
	});

	test("blocks the tool call when the adapter denies Start", async () => {
		spawnSyncResult = { status: 1 };
		const { api, handlers } = makeApi();
		sceExtension(api as never);
		const [, startHandler] = handlers.get("tool_call") ?? [];
		const result = await startHandler?.(
			{ toolName: "edit", toolCallId: "c2" },
			ctxFor("/repo"),
		);
		expect(result).toEqual({
			block: true,
			reason:
				"SCE could not establish Pi mutation attribution for this tool execution.",
		});
	});

	test("blocks the tool call when the sce CLI is missing", async () => {
		spawnSyncResult = {
			status: null,
			error: Object.assign(new Error("not found"), { code: "ENOENT" }),
		};
		const { api, handlers } = makeApi();
		sceExtension(api as never);
		const [, startHandler] = handlers.get("tool_call") ?? [];
		const result = await startHandler?.(
			{ toolName: "write", toolCallId: "c3" },
			ctxFor("/repo"),
		);
		expect(result).toEqual({
			block: true,
			reason:
				"SCE could not establish Pi mutation attribution for this tool execution.",
		});
	});
});

describe("mutation-scope execution-evidence and Close forwarding", () => {
	test("forwards tool_execution_start for a tracked tool as telemetry", async () => {
		const { api, handlers } = makeApi();
		sceExtension(api as never);
		const [handler] = handlers.get("tool_execution_start") ?? [];
		handler?.({ toolName: "bash", toolCallId: "c1" }, ctxFor("/repo", "ses_x"));
		expect(spawnCalls).toHaveLength(1);
		expect(spawnCalls[0].args).toEqual(["hooks", "pi-mutation-scope"]);
		expect(spawnCalls[0].child.stdin.writes[0]).toContain(
			'"hook_event_name":"ToolExecutionStart"',
		);
	});

	test("does not forward tool_execution_start for an untracked tool", async () => {
		const { api, handlers } = makeApi();
		sceExtension(api as never);
		const [handler] = handlers.get("tool_execution_start") ?? [];
		handler?.({ toolName: "grep", toolCallId: "c1" }, ctxFor("/repo"));
		expect(spawnCalls).toHaveLength(0);
	});

	test("forwards a tracked tool_result as ToolResult", async () => {
		const { api, handlers } = makeApi();
		sceExtension(api as never);
		const mutationScopeHandler = (handlers.get("tool_result") ?? [])[0];
		mutationScopeHandler?.(
			{ toolName: "write", toolCallId: "c5", isError: false },
			ctxFor("/repo", "ses_y"),
		);
		expect(spawnCalls).toHaveLength(1);
		const payload = JSON.parse(spawnCalls[0].child.stdin.writes[0]);
		expect(payload).toEqual({
			hook_event_name: "ToolResult",
			session_id: "ses_y",
			tool_call_id: "c5",
			cwd: "/repo",
			tool_name: "write",
		});
	});

	test("forwards a tracked tool_execution_end as ToolExecutionEnd", async () => {
		const { api, handlers } = makeApi();
		sceExtension(api as never);
		const [handler] = handlers.get("tool_execution_end") ?? [];
		handler?.(
			{ toolName: "edit", toolCallId: "c6", isError: false },
			ctxFor("/repo", "ses_z"),
		);
		expect(spawnCalls).toHaveLength(1);
		const payload = JSON.parse(spawnCalls[0].child.stdin.writes[0]);
		expect(payload).toEqual({
			hook_event_name: "ToolExecutionEnd",
			session_id: "ses_z",
			tool_call_id: "c6",
			cwd: "/repo",
			tool_name: "edit",
		});
	});

	test("does not forward tool_execution_end for an untracked tool", async () => {
		const { api, handlers } = makeApi();
		sceExtension(api as never);
		const [handler] = handlers.get("tool_execution_end") ?? [];
		handler?.({ toolName: "ls", toolCallId: "c7" }, ctxFor("/repo"));
		expect(spawnCalls).toHaveLength(0);
	});
});

const FAIL_CLOSED_REASON =
	"SCE could not establish Pi mutation attribution for this tool execution.";

describe("terminal transport ordering (Problem 1)", () => {
	test("ToolExecutionEnd is withheld until its own ToolResult delivery settles, then sent in order", async () => {
		const { api, handlers } = makeApi();
		sceExtension(api as never);
		const [resultHandler] = handlers.get("tool_result") ?? [];
		const [endHandler] = handlers.get("tool_execution_end") ?? [];

		resultHandler?.(
			{ toolName: "bash", toolCallId: "c1", isError: false },
			ctxFor("/repo", "ses_order"),
		);
		expect(spawnCalls).toHaveLength(1);

		endHandler?.(
			{ toolName: "bash", toolCallId: "c1", isError: false },
			ctxFor("/repo", "ses_order"),
		);
		await flush();
		expect(spawnCalls).toHaveLength(1);

		spawnCalls[0].child.emit("close", 0);
		await flush();
		await flush();

		expect(spawnCalls).toHaveLength(2);
		expect(
			JSON.parse(spawnCalls[0].child.stdin.writes[0]).hook_event_name,
		).toBe("ToolResult");
		expect(
			JSON.parse(spawnCalls[1].child.stdin.writes[0]).hook_event_name,
		).toBe("ToolExecutionEnd");
	});
});

describe("D9 terminal transport failure", () => {
	test("a failed ToolResult delivery denies Starts immediately and converts a later ToolExecutionEnd into ToolExecutionAbandon", async () => {
		const { api, handlers } = makeApi();
		sceExtension(api as never);
		const [resultHandler] = handlers.get("tool_result") ?? [];
		const [endHandler] = handlers.get("tool_execution_end") ?? [];
		const [, startHandler] = handlers.get("tool_call") ?? [];

		resultHandler?.(
			{ toolName: "bash", toolCallId: "c2", isError: false },
			ctxFor("/repo", "ses_fail"),
		);
		expect(spawnCalls).toHaveLength(1);
		spawnCalls[0].child.emit("close", 1);
		await flush();

		const denied = await startHandler?.(
			{ toolName: "bash", toolCallId: "c2-sibling" },
			ctxFor("/repo"),
		);
		expect(denied).toEqual({ block: true, reason: FAIL_CLOSED_REASON });

		endHandler?.(
			{ toolName: "bash", toolCallId: "c2", isError: false },
			ctxFor("/repo", "ses_fail"),
		);
		await flush();

		expect(spawnCalls).toHaveLength(2);
		expect(
			JSON.parse(spawnCalls[1].child.stdin.writes[0]).hook_event_name,
		).toBe("ToolExecutionAbandon");

		spawnCalls[1].child.emit("close", 0);
		await flush();

		const allowed = await startHandler?.(
			{ toolName: "bash", toolCallId: "c2-again" },
			ctxFor("/repo"),
		);
		expect(allowed).toBeUndefined();
	});

	test("denies a tracked Start while a tool_execution_end delivery is unresolved", async () => {
		const { api, handlers } = makeApi();
		sceExtension(api as never);
		const [endHandler] = handlers.get("tool_execution_end") ?? [];
		endHandler?.(
			{ toolName: "write", toolCallId: "c8", isError: false },
			ctxFor("/repo"),
		);
		const [, startHandler] = handlers.get("tool_call") ?? [];
		const result = await startHandler?.(
			{ toolName: "bash", toolCallId: "c8b" },
			ctxFor("/repo"),
		);
		expect(result).toEqual({ block: true, reason: FAIL_CLOSED_REASON });
		expect(spawnSyncCalls).toHaveLength(0);
	});

	test("clears the unresolved marker once tool_execution_end delivery succeeds", async () => {
		const { api, handlers } = makeApi();
		sceExtension(api as never);
		const [endHandler] = handlers.get("tool_execution_end") ?? [];
		endHandler?.(
			{ toolName: "write", toolCallId: "c9", isError: false },
			ctxFor("/repo"),
		);
		expect(spawnCalls).toHaveLength(1);
		spawnCalls[0].child.emit("close", 0);
		await flush();

		const [, startHandler] = handlers.get("tool_call") ?? [];
		const result = await startHandler?.(
			{ toolName: "bash", toolCallId: "c9b" },
			ctxFor("/repo"),
		);
		expect(result).toBeUndefined();
	});

	test("retries a failed tool_execution_end delivery as ToolExecutionAbandon using a synchronous retry seam", async () => {
		spawnCalls = [];
		const scheduled: Array<() => void> = [];
		const syncSchedule: RetryScheduleFn = (run) => {
			scheduled.push(run);
		};
		const tracker = createTerminalDeliveryTracker(syncSchedule);
		const key = { sessionId: "ses_d10", toolCallId: "c10" };
		const endPayload: PiMutationScopePayload = {
			hook_event_name: "ToolExecutionEnd",
			session_id: "ses_d10",
			tool_call_id: "c10",
			cwd: "/repo",
			tool_name: "bash",
		};

		const endPromise = tracker.forwardEnd("/repo", endPayload, key);
		await flush();
		expect(spawnCalls).toHaveLength(1);
		expect(
			JSON.parse(spawnCalls[0].child.stdin.writes[0]).hook_event_name,
		).toBe("ToolExecutionEnd");

		spawnCalls[0].child.emit("close", 1);
		await endPromise;
		expect(tracker.hasUnresolved()).toBe(true);
		expect(scheduled).toHaveLength(1);

		const retry = scheduled.shift();
		retry?.();
		expect(spawnCalls).toHaveLength(2);
		expect(
			JSON.parse(spawnCalls[1].child.stdin.writes[0]).hook_event_name,
		).toBe("ToolExecutionAbandon");

		spawnCalls[1].child.emit("close", 0);
		await flush();
		expect(tracker.hasUnresolved()).toBe(false);
	});

	test("a ToolExecutionEnd transport failure after a successful ToolResult retries as ToolExecutionAbandon, never a delayed End", async () => {
		spawnCalls = [];
		const scheduled: Array<() => void> = [];
		const syncSchedule: RetryScheduleFn = (run) => {
			scheduled.push(run);
		};
		const tracker = createTerminalDeliveryTracker(syncSchedule);
		const key = { sessionId: "ses_d3", toolCallId: "c3" };
		const resultPayload: PiMutationScopePayload = {
			hook_event_name: "ToolResult",
			session_id: "ses_d3",
			tool_call_id: "c3",
			cwd: "/repo",
			tool_name: "bash",
		};
		const endPayload: PiMutationScopePayload = {
			...resultPayload,
			hook_event_name: "ToolExecutionEnd",
		};

		tracker.forwardResult("/repo", resultPayload, key);
		spawnCalls[0].child.emit("close", 0);
		await flush();

		const endPromise = tracker.forwardEnd("/repo", endPayload, key);
		await flush();
		expect(spawnCalls).toHaveLength(2);
		expect(
			JSON.parse(spawnCalls[1].child.stdin.writes[0]).hook_event_name,
		).toBe("ToolExecutionEnd");

		spawnCalls[1].child.emit("close", 1);
		await endPromise;
		expect(tracker.hasUnresolved()).toBe(true);

		const retry = scheduled.shift();
		retry?.();
		expect(spawnCalls).toHaveLength(3);
		expect(
			JSON.parse(spawnCalls[2].child.stdin.writes[0]).hook_event_name,
		).toBe("ToolExecutionAbandon");
		expect(tracker.hasUnresolved()).toBe(true);

		spawnCalls[2].child.emit("close", 0);
		await flush();
		expect(tracker.hasUnresolved()).toBe(false);
	});

	test("same toolCallId in two different sessions maintain independent unresolved state", async () => {
		spawnCalls = [];
		const tracker = createTerminalDeliveryTracker();
		const keyA: AttemptKey = { sessionId: "ses_A", toolCallId: "c1" };
		const keyB: AttemptKey = { sessionId: "ses_B", toolCallId: "c1" };
		const resultPayloadA: PiMutationScopePayload = {
			hook_event_name: "ToolResult",
			session_id: "ses_A",
			tool_call_id: "c1",
			cwd: "/repo",
			tool_name: "bash",
		};
		const resultPayloadB: PiMutationScopePayload = {
			...resultPayloadA,
			session_id: "ses_B",
		};

		tracker.forwardResult("/repo", resultPayloadA, keyA);
		tracker.forwardResult("/repo", resultPayloadB, keyB);
		expect(spawnCalls).toHaveLength(2);

		spawnCalls[0].child.emit("close", 1);
		await flush();
		expect(tracker.hasUnresolved()).toBe(true);

		spawnCalls[1].child.emit("close", 1);
		await flush();
		expect(tracker.hasUnresolved()).toBe(true);

		const endA = tracker.forwardEnd(
			"/repo",
			{ ...resultPayloadA, hook_event_name: "ToolExecutionEnd" },
			keyA,
		);
		await flush();
		expect(spawnCalls).toHaveLength(3);
		expect(
			JSON.parse(spawnCalls[2].child.stdin.writes[0]).hook_event_name,
		).toBe("ToolExecutionAbandon");
		spawnCalls[2].child.emit("close", 0);
		await endA;

		expect(tracker.hasUnresolved()).toBe(true);

		const endB = tracker.forwardEnd(
			"/repo",
			{ ...resultPayloadB, hook_event_name: "ToolExecutionEnd" },
			keyB,
		);
		await flush();
		expect(spawnCalls).toHaveLength(4);
		expect(
			JSON.parse(spawnCalls[3].child.stdin.writes[0]).hook_event_name,
		).toBe("ToolExecutionAbandon");
		spawnCalls[3].child.emit("close", 0);
		await endB;

		expect(tracker.hasUnresolved()).toBe(false);
	});
});

describe("user_bash guard", () => {
	const originalPlatform = process.platform;

	afterEach(() => {
		Object.defineProperty(process, "platform", {
			value: originalPlatform,
		});
	});

	test("refuses unconditionally on win32", async () => {
		Object.defineProperty(process, "platform", { value: "win32" });
		const { api, handlers } = makeApi();
		sceExtension(api as never);
		const [handler] = handlers.get("user_bash") ?? [];
		const result = await handler?.({
			type: "user_bash",
			command: "echo hi",
			excludeFromContext: false,
			cwd: "/repo",
		});
		expect(result).toEqual({
			result: {
				output:
					"SCE does not support guarded user_bash execution on Windows in this release; run this command outside Pi.",
				exitCode: 1,
				cancelled: false,
				truncated: false,
			},
		});
		expect(spawnCalls).toHaveLength(0);
	});

	test("returns wrapped operations once the supervisor acknowledges armed", async () => {
		const { api, handlers } = makeApi();
		sceExtension(api as never);
		const [handler] = handlers.get("user_bash") ?? [];
		const pending = handler?.({
			type: "user_bash",
			command: "echo hi",
			excludeFromContext: false,
			cwd: "/repo",
		}) as Promise<{ operations?: { exec: unknown } }>;
		await flush();

		expect(spawnCalls).toHaveLength(1);
		expect(spawnCalls[0].args).toEqual(["hooks", "external-mutation-guard"]);
		expect(spawnCalls[0].child.stdin.writes[0]).toBe(
			`${JSON.stringify({ operation: "arm" })}\n`,
		);
		spawnCalls[0].child.emitLine({ status: "armed" });

		const result = await pending;
		expect(result.operations).toBeDefined();
		expect(typeof result.operations?.exec).toBe("function");
	});

	test("refuses and terminates the supervisor when it closes before armed", async () => {
		const { api, handlers } = makeApi();
		sceExtension(api as never);
		const [handler] = handlers.get("user_bash") ?? [];
		const pending = handler?.({
			type: "user_bash",
			command: "echo hi",
			excludeFromContext: false,
			cwd: "/repo",
		}) as Promise<{ result?: { output: string } }>;
		await flush();

		spawnCalls[0].child.emit("close");

		const result = await pending;
		expect(result.result?.output).toBe(
			"SCE could not establish the worktree external-mutation guard for this command.",
		);
		expect(spawnCalls[0].child.killed).toBe(true);
	});

	test("wrapped exec relays stdout/stderr to onData and resolves on the result line", async () => {
		const { api, handlers } = makeApi();
		sceExtension(api as never);
		const [handler] = handlers.get("user_bash") ?? [];
		const pending = handler?.({
			type: "user_bash",
			command: "echo hi",
			excludeFromContext: false,
			cwd: "/repo",
		}) as Promise<{
			operations: {
				exec: (
					command: string,
					cwd: string,
					options: { onData: (data: Buffer) => void },
				) => Promise<{ exitCode: number | null }>;
			};
		}>;
		await flush();
		spawnCalls[0].child.emitLine({ status: "armed" });
		const { operations } = await pending;

		const chunks: string[] = [];
		const execPromise = operations.exec("echo hi", "/repo", {
			onData: (data) => chunks.push(data.toString()),
		});

		expect(spawnCalls[0].child.stdin.writes.at(-1)).toBe(
			`${JSON.stringify({ operation: "exec", command: "echo hi", cwd: "/repo", env: {} })}\n`,
		);
		spawnCalls[0].child.emitLine({ stream: "stdout", data: "hi\n" });
		spawnCalls[0].child.emitLine({ status: "result", exit_code: 0 });

		const execResult = await execPromise;
		expect(execResult).toEqual({ exitCode: 0 });
		expect(chunks).toEqual(["hi\n"]);
	});

	test("wrapped exec sends a cancel operation on abort", async () => {
		const { api, handlers } = makeApi();
		sceExtension(api as never);
		const [handler] = handlers.get("user_bash") ?? [];
		const pending = handler?.({
			type: "user_bash",
			command: "sleep 10",
			excludeFromContext: false,
			cwd: "/repo",
		}) as Promise<{
			operations: {
				exec: (
					command: string,
					cwd: string,
					options: { onData: (data: Buffer) => void; signal?: AbortSignal },
				) => Promise<{ exitCode: number | null }>;
			};
		}>;
		await flush();
		spawnCalls[0].child.emitLine({ status: "armed" });
		const { operations } = await pending;

		const controller = new AbortController();
		const execPromise = operations.exec("sleep 10", "/repo", {
			onData: () => {},
			signal: controller.signal,
		});
		controller.abort();
		expect(spawnCalls[0].child.stdin.writes.at(-1)).toBe(
			`${JSON.stringify({ operation: "cancel" })}\n`,
		);

		spawnCalls[0].child.emitLine({ status: "result", exit_code: null });
		await execPromise;
	});

	test("wrapped exec rejects when the control channel closes before any result frame, never resolving exitCode: null", async () => {
		const { api, handlers } = makeApi();
		sceExtension(api as never);
		const [handler] = handlers.get("user_bash") ?? [];
		const pending = handler?.({
			type: "user_bash",
			command: "echo hi",
			excludeFromContext: false,
			cwd: "/repo",
		}) as Promise<{
			operations: {
				exec: (
					command: string,
					cwd: string,
					options: { onData: (data: Buffer) => void },
				) => Promise<{ exitCode: number | null }>;
			};
		}>;
		await flush();
		spawnCalls[0].child.emitLine({ status: "armed" });
		const { operations } = await pending;

		const execPromise = operations.exec("echo hi", "/repo", {
			onData: () => {},
		});
		spawnCalls[0].child.emit("close");

		await expect(execPromise).rejects.toThrow(
			"SCE lost contact with the external-mutation-guard supervisor before it reported a command result.",
		);
		expect(spawnCalls).toHaveLength(1);
	});

	test("an explicit supervisor result with exit_code null is a valid authoritative completion, not a channel-loss fabrication", async () => {
		const { api, handlers } = makeApi();
		sceExtension(api as never);
		const [handler] = handlers.get("user_bash") ?? [];
		const pending = handler?.({
			type: "user_bash",
			command: "echo hi",
			excludeFromContext: false,
			cwd: "/repo",
		}) as Promise<{
			operations: {
				exec: (
					command: string,
					cwd: string,
					options: { onData: (data: Buffer) => void },
				) => Promise<{ exitCode: number | null }>;
			};
		}>;
		await flush();
		spawnCalls[0].child.emitLine({ status: "armed" });
		const { operations } = await pending;

		const execPromise = operations.exec("echo hi", "/repo", {
			onData: () => {},
		});
		spawnCalls[0].child.emitLine({ status: "result", exit_code: null });

		await expect(execPromise).resolves.toEqual({ exitCode: null });
	});
});
