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
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { join } from "node:path";
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

const realChildProcessModule = createRequire(import.meta.url)(
	"node:child_process",
);
const realSpawnSync: typeof import("node:child_process").spawnSync =
	realChildProcessModule.spawnSync;

mock.module("node:child_process", () => ({
	...realChildProcessModule,
	spawnSync: (
		command: string,
		args: string[],
		options: { input?: string; cwd?: string },
	) => {
		if (command !== "sce") {
			return realSpawnSync(command, args, options as never);
		}
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

const realFsPromisesModule = createRequire(import.meta.url)("node:fs/promises");

mock.module("node:fs/promises", () => ({
	...realFsPromisesModule,
	readdir: async (dir: string) => {
		if (dir.endsWith("/.pi/extensions")) {
			return ["sce"];
		}
		return [];
	},
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

let smokeTempDirs: string[] = [];

beforeEach(() => {
	spawnSyncCalls = [];
	spawnSyncResult = { status: 0 };
	spawnCalls = [];
	smokeTempDirs = [];
});

afterEach(() => {
	for (const dir of smokeTempDirs) {
		rmSync(dir, { recursive: true, force: true });
	}
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

	test("a competing extension consuming user_bash ahead of SCE prevents SCE's handler from ever running, while tracked-tool attribution in the same session is unaffected", async () => {
		const { api, handlers } = makeApi();
		sceExtension(api as never);
		const [sceHandler] = handlers.get("user_bash") ?? [];
		expect(sceHandler).toBeDefined();

		let sceHandlerInvoked = false;
		const spiedSceHandler: Handler = (event, ctx) => {
			sceHandlerInvoked = true;
			return sceHandler?.(event, ctx);
		};

		const competingHandler: Handler = () => ({
			result: {
				output: "handled by another extension",
				exitCode: 0,
				cancelled: false,
				truncated: false,
			},
		});
		const orderedHandlers: Handler[] = [competingHandler, spiedSceHandler];

		let dispatched: unknown;
		for (const handler of orderedHandlers) {
			const result = await handler(
				{
					type: "user_bash",
					command: "echo hi",
					excludeFromContext: false,
					cwd: "/repo",
				},
				undefined,
			);
			if (result) {
				dispatched = result;
				break;
			}
		}

		expect(dispatched).toEqual({
			result: {
				output: "handled by another extension",
				exitCode: 0,
				cancelled: false,
				truncated: false,
			},
		});
		expect(sceHandlerInvoked).toBe(false);
		expect(spawnCalls).toHaveLength(0);

		const [, startHandler] = handlers.get("tool_call") ?? [];
		const callResult = await startHandler?.(
			{ toolName: "bash", toolCallId: "c_after_competing_user_bash" },
			ctxFor("/repo", "ses_after_competing_user_bash"),
		);
		expect(callResult).toBeUndefined();
		expect(spawnSyncCalls).toHaveLength(1);
	});
});

type CaptureLine = {
	tag: string;
	hook: string;
	payload: {
		event?: Record<string, unknown>;
		model?: { provider: string; id: string };
	};
};

const FIXTURES_DIR = join(
	import.meta.dir,
	"..",
	"..",
	"..",
	"cli/src/services/hooks/pi_mutation_scope/fixtures/captures",
);

function loadCaptureLines(fixtureFile: string): CaptureLine[] {
	const raw = readFileSync(join(FIXTURES_DIR, fixtureFile), "utf8");
	return raw
		.trim()
		.split("\n")
		.map((line) => JSON.parse(line) as CaptureLine)
		.filter(
			(line) => line.tag === "capture" && line.payload.event !== undefined,
		);
}

function findEvent(
	lines: CaptureLine[],
	hook: string,
	occurrence = 0,
): Record<string, unknown> {
	const matches = lines.filter((line) => line.hook === hook);
	const match = matches[occurrence];
	if (!match?.payload.event) {
		throw new Error(`fixture missing hook "${hook}" occurrence ${occurrence}`);
	}
	return match.payload.event;
}

function findRawPayload(
	lines: CaptureLine[],
	hook: string,
	occurrence = 0,
): Record<string, unknown> {
	const matches = lines.filter((line) => line.hook === hook);
	const match = matches[occurrence];
	if (!match) {
		throw new Error(`fixture missing hook "${hook}" occurrence ${occurrence}`);
	}
	return match.payload as unknown as Record<string, unknown>;
}

async function emitToolCall(
	handlers: Handler[],
	event: unknown,
	ctx: unknown,
): Promise<{ block?: boolean; reason?: string } | undefined> {
	let result: { block?: boolean; reason?: string } | undefined;
	for (const handler of handlers) {
		const handlerResult = (await handler(event, ctx)) as
			| { block?: boolean; reason?: string }
			| undefined;
		if (handlerResult) {
			result = handlerResult;
			if (result.block) {
				return result;
			}
		}
	}
	return result;
}

async function emitAll(
	handlers: Handler[],
	event: unknown,
	ctx: unknown,
): Promise<void> {
	for (const handler of handlers) {
		await handler(event, ctx);
	}
}

function ctxFromCapture(
	cwd: string,
	sessionId: string,
	model: { provider: string; id: string } | undefined,
) {
	return {
		cwd,
		sessionManager: { getSessionId: () => sessionId },
		model,
	};
}

function makeTempGitRepo(): string {
	const dir = mkdtempSync(join(tmpdir(), "sce-pi-smoke-"));
	const init = realSpawnSync("git", ["init", "--quiet"], { cwd: dir } as never);
	if (init.status !== 0) {
		throw new Error(`git init failed: ${init.stderr?.toString() ?? ""}`);
	}
	realSpawnSync("git", ["config", "user.email", "smoke@example.com"], {
		cwd: dir,
	} as never);
	realSpawnSync("git", ["config", "user.name", "Smoke Test"], {
		cwd: dir,
	} as never);
	return dir;
}

async function settleAllSpawns(rounds = 5): Promise<void> {
	for (let i = 0; i < rounds; i++) {
		for (const call of spawnCalls) {
			if (!call.child.killed) {
				call.child.emit("close", 0);
			}
		}
		await flush();
	}
}

describe("pinned Pi capture-replay smoke (T01 lifecycle fixtures, Linux)", () => {
	test("bash: tool_execution_start -> tool_call -> tool_result -> tool_execution_end reaches confirmed Close wiring, with model provenance", async () => {
		const lines = loadCaptureLines("bash-success.jsonl");
		const startEvent = findEvent(lines, "tool_execution_start");
		const callEvent = findEvent(lines, "tool_call");
		const resultEvent = findEvent(lines, "tool_result");
		const endEvent = findEvent(lines, "tool_execution_end");
		const model = findRawPayload(lines, "tool_call").model as {
			provider: string;
			id: string;
		};

		const cwd = makeTempGitRepo();
		smokeTempDirs.push(cwd);
		const { api, handlers } = makeApi();
		sceExtension(api as never);
		const ctx = ctxFromCapture(cwd, "ses_bash_smoke", model);

		await emitAll(handlers.get("tool_execution_start") ?? [], startEvent, ctx);
		expect(spawnCalls).toHaveLength(1);
		expect(
			JSON.parse(spawnCalls[0].child.stdin.writes[0]).hook_event_name,
		).toBe("ToolExecutionStart");

		const callResult = await emitToolCall(
			handlers.get("tool_call") ?? [],
			callEvent,
			ctx,
		);
		expect(callResult).toBeUndefined();
		expect(spawnSyncCalls.at(-1)?.payload).toEqual({
			hook_event_name: "ToolCall",
			session_id: "ses_bash_smoke",
			tool_call_id: (callEvent as { toolCallId: string }).toolCallId,
			cwd,
			tool_name: "bash",
			model: `${model.provider}/${model.id}`,
		});

		await emitAll(handlers.get("tool_result") ?? [], resultEvent, ctx);
		await emitAll(handlers.get("tool_execution_end") ?? [], endEvent, ctx);
		await settleAllSpawns();

		const forwarded = spawnCalls.map(
			(call) => JSON.parse(call.child.stdin.writes[0]).hook_event_name,
		);
		expect(forwarded).toEqual([
			"ToolExecutionStart",
			"ToolResult",
			"ToolExecutionEnd",
		]);
	});

	test("write: missing model yields NULL provenance (ctx.model absent for this attempt)", async () => {
		const lines = loadCaptureLines("write-success.jsonl");
		const callEvent = findEvent(lines, "tool_call");

		const cwd = makeTempGitRepo();
		smokeTempDirs.push(cwd);
		const { api, handlers } = makeApi();
		sceExtension(api as never);
		const ctx = ctxFromCapture(cwd, "ses_write_nomodel", undefined);

		await emitToolCall(handlers.get("tool_call") ?? [], callEvent, ctx);
		expect(spawnSyncCalls.at(-1)?.payload).toEqual({
			hook_event_name: "ToolCall",
			session_id: "ses_write_nomodel",
			tool_call_id: (callEvent as { toolCallId: string }).toolCallId,
			cwd,
			tool_name: "write",
			model: undefined,
		});
	});

	test("read-only and custom/unknown tools create zero mutation-scope footprint", async () => {
		const readonlyLines = loadCaptureLines("readonly-footprint.jsonl");
		const customLines = loadCaptureLines("customtool.jsonl");
		const cwd = makeTempGitRepo();
		smokeTempDirs.push(cwd);
		const { api, handlers } = makeApi();
		sceExtension(api as never);
		const ctx = ctxFromCapture(cwd, "ses_readonly", undefined);

		for (const hook of [
			"tool_execution_start",
			"tool_call",
			"tool_result",
			"tool_execution_end",
		]) {
			for (const lines of [readonlyLines, customLines]) {
				const matches = lines.filter((line) => line.hook === hook);
				for (const match of matches) {
					if (hook === "tool_call") {
						await emitToolCall(
							handlers.get("tool_call") ?? [],
							match.payload.event,
							ctx,
						);
					} else {
						await emitAll(handlers.get(hook) ?? [], match.payload.event, ctx);
					}
				}
			}
		}

		expect(spawnSyncCalls).toHaveLength(0);
		expect(spawnCalls).toHaveLength(0);
	});

	test("edit: real before/after file mutation in a real Git repo drives Start/Close and the diff-trace pipeline", async () => {
		const lines = loadCaptureLines("edit-success.jsonl");
		const startEvent = findEvent(lines, "tool_execution_start", 1);
		const callEvent = findEvent(lines, "tool_call", 1) as {
			toolCallId: string;
			toolName: string;
			input: { path: string };
		};
		const resultEvent = findEvent(lines, "tool_result", 1);
		const endEvent = findEvent(lines, "tool_execution_end", 1);
		const model = findRawPayload(lines, "tool_call", 1).model as {
			provider: string;
			id: string;
		};

		const cwd = makeTempGitRepo();
		smokeTempDirs.push(cwd);
		const filePath = join(cwd, callEvent.input.path);
		writeFileSync(filePath, "line1");

		const { api, handlers } = makeApi();
		sceExtension(api as never);
		const ctx = ctxFromCapture(cwd, "ses_edit_smoke", model);

		await emitAll(handlers.get("tool_execution_start") ?? [], startEvent, ctx);
		const callResult = await emitToolCall(
			handlers.get("tool_call") ?? [],
			callEvent,
			ctx,
		);
		expect(callResult).toBeUndefined();

		writeFileSync(filePath, "line2");

		await emitAll(handlers.get("tool_result") ?? [], resultEvent, ctx);
		await emitAll(handlers.get("tool_execution_end") ?? [], endEvent, ctx);
		await settleAllSpawns();

		const mutationScopeCalls = spawnSyncCalls.filter(
			(call) => call.args[1] === "pi-mutation-scope",
		);
		expect(mutationScopeCalls).toHaveLength(1);
		expect(mutationScopeCalls[0].payload).toEqual({
			hook_event_name: "ToolCall",
			session_id: "ses_edit_smoke",
			tool_call_id: callEvent.toolCallId,
			cwd,
			tool_name: "edit",
			model: `${model.provider}/${model.id}`,
		});

		const mutationScopeSpawns = spawnCalls.filter(
			(call) => call.args[1] === "pi-mutation-scope",
		);
		const forwarded = mutationScopeSpawns.map(
			(call) => JSON.parse(call.child.stdin.writes[0]).hook_event_name,
		);
		expect(forwarded).toEqual([
			"ToolExecutionStart",
			"ToolResult",
			"ToolExecutionEnd",
		]);

		const traceSpawns = spawnCalls.filter(
			(call) =>
				call.args[1] === "diff-trace" || call.args[1] === "conversation-trace",
		);
		expect(traceSpawns.length).toBeGreaterThan(0);
		const diffTraceSpawn = spawnCalls.find(
			(call) => call.args[1] === "diff-trace",
		);
		const diffPayload = diffTraceSpawn
			? JSON.parse(diffTraceSpawn.child.stdin.writes[0])
			: undefined;
		expect(diffPayload?.diff).toContain("-line1");
		expect(diffPayload?.diff).toContain("+line2");
	});

	test("SCE Start failure blocks a real tool_call event before execution", async () => {
		spawnSyncResult = { status: 1 };
		const lines = loadCaptureLines("bash-success.jsonl");
		const callEvent = findEvent(lines, "tool_call");
		const cwd = makeTempGitRepo();
		smokeTempDirs.push(cwd);
		const { api, handlers } = makeApi();
		sceExtension(api as never);
		const ctx = ctxFromCapture(cwd, "ses_denied", undefined);

		const result = await emitToolCall(
			handlers.get("tool_call") ?? [],
			callEvent,
			ctx,
		);
		expect(result).toEqual({
			block: true,
			reason:
				"SCE could not establish Pi mutation attribution for this tool execution.",
		});
	});

	test("later-extension rejection after a successful SCE Start produces tool_execution_end with no preceding tool_result (D7 abandon shape)", async () => {
		const lines = loadCaptureLines("probeB-later-block.jsonl");
		const startEvent = findEvent(lines, "tool_execution_start");
		const callEvent = findEvent(lines, "tool_call");
		const endEvent = findEvent(lines, "tool_execution_end");
		expect(lines.filter((line) => line.hook === "tool_result")).toHaveLength(0);

		const cwd = makeTempGitRepo();
		smokeTempDirs.push(cwd);
		const { api, handlers } = makeApi();
		sceExtension(api as never);
		const ctx = ctxFromCapture(cwd, "ses_later_block", undefined);

		await emitAll(handlers.get("tool_execution_start") ?? [], startEvent, ctx);

		const combinedHandlers: Handler[] = [
			...(handlers.get("tool_call") ?? []),
			() => ({ block: true, reason: "competing extension blocked" }),
		];
		const overall = await emitToolCall(combinedHandlers, callEvent, ctx);
		expect(overall).toEqual({
			block: true,
			reason: "competing extension blocked",
		});
		expect(spawnSyncCalls.at(-1)?.payload).toMatchObject({
			hook_event_name: "ToolCall",
		});

		await emitAll(handlers.get("tool_execution_end") ?? [], endEvent, ctx);
		await flush();

		const forwarded = spawnCalls.map(
			(call) => JSON.parse(call.child.stdin.writes[0]).hook_event_name,
		);
		expect(forwarded).toEqual(["ToolExecutionStart", "ToolExecutionEnd"]);
	});

	test("mutate-then-error (isError: true) still forwards ToolResult/ToolExecutionEnd", async () => {
		const lines = loadCaptureLines("bash-nonzero.jsonl");
		const startEvent = findEvent(lines, "tool_execution_start");
		const callEvent = findEvent(lines, "tool_call");
		const resultEvent = findEvent(lines, "tool_result") as { isError: boolean };
		const endEvent = findEvent(lines, "tool_execution_end");
		expect(resultEvent.isError).toBe(true);

		const cwd = makeTempGitRepo();
		smokeTempDirs.push(cwd);
		const { api, handlers } = makeApi();
		sceExtension(api as never);
		const ctx = ctxFromCapture(cwd, "ses_error", undefined);

		await emitAll(handlers.get("tool_execution_start") ?? [], startEvent, ctx);
		await emitToolCall(handlers.get("tool_call") ?? [], callEvent, ctx);
		await emitAll(handlers.get("tool_result") ?? [], resultEvent, ctx);
		await emitAll(handlers.get("tool_execution_end") ?? [], endEvent, ctx);
		await settleAllSpawns();

		const forwarded = spawnCalls.map(
			(call) => JSON.parse(call.child.stdin.writes[0]).hook_event_name,
		);
		expect(forwarded).toEqual([
			"ToolExecutionStart",
			"ToolResult",
			"ToolExecutionEnd",
		]);
	});
});

describe("Windows-specific pinned Pi capture-replay smoke (D13 disposition)", () => {
	const originalPlatform = process.platform;

	afterEach(() => {
		Object.defineProperty(process, "platform", { value: originalPlatform });
	});

	test("user_bash is unconditionally refused, and a tracked bash tool_call in the same session still reaches confirmed Close wiring", async () => {
		Object.defineProperty(process, "platform", { value: "win32" });

		const cwd = makeTempGitRepo();
		smokeTempDirs.push(cwd);
		const { api, handlers } = makeApi();
		sceExtension(api as never);

		const [userBashHandler] = handlers.get("user_bash") ?? [];
		const refusal = await userBashHandler?.({
			type: "user_bash",
			command: "echo hi",
			excludeFromContext: false,
			cwd,
		});
		expect(refusal).toEqual({
			result: {
				output:
					"SCE does not support guarded user_bash execution on Windows in this release; run this command outside Pi.",
				exitCode: 1,
				cancelled: false,
				truncated: false,
			},
		});
		expect(spawnCalls).toHaveLength(0);

		const lines = loadCaptureLines("bash-success.jsonl");
		const startEvent = findEvent(lines, "tool_execution_start");
		const callEvent = findEvent(lines, "tool_call");
		const resultEvent = findEvent(lines, "tool_result");
		const endEvent = findEvent(lines, "tool_execution_end");
		const model = findRawPayload(lines, "tool_call").model as {
			provider: string;
			id: string;
		};
		const ctx = ctxFromCapture(cwd, "ses_win32_bash", model);

		await emitAll(handlers.get("tool_execution_start") ?? [], startEvent, ctx);
		const callResult = await emitToolCall(
			handlers.get("tool_call") ?? [],
			callEvent,
			ctx,
		);
		expect(callResult).toBeUndefined();
		await emitAll(handlers.get("tool_result") ?? [], resultEvent, ctx);
		await emitAll(handlers.get("tool_execution_end") ?? [], endEvent, ctx);
		await settleAllSpawns();

		const forwarded = spawnCalls.map(
			(call) => JSON.parse(call.child.stdin.writes[0]).hook_event_name,
		);
		expect(forwarded).toEqual([
			"ToolExecutionStart",
			"ToolResult",
			"ToolExecutionEnd",
		]);
	});
});
