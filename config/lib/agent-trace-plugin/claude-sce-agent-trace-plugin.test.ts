import { describe, expect, test } from "bun:test";
import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { deriveClaudeDiffTracePayload } from "./claude-sce-agent-trace-plugin.ts";

const FIXED_TIME = 1700000000000;
const FIXED_TOOL_VERSION = "test-claude-version";
const EXPECTED_SCENARIOS = [
	"write_create_simple",
	"write_create_empty",
	"write_create_no_newline",
	"write_create_multiline",
	"edit_single_hunk",
	"edit_multi_hunk",
	"edit_only_additions",
	"edit_only_deletions",
] as const;

type ClaudePostToolUseFixture = {
	session_id: string;
};

const fixtureRoot = path.resolve(
	path.dirname(fileURLToPath(import.meta.url)),
	"../../../cli/src/services/patch/fixtures/diff_creation",
);

function discoverFixtureScenarios(): string[] {
	return readdirSync(fixtureRoot, { withFileTypes: true })
		.filter((entry) => entry.isDirectory())
		.map((entry) => entry.name)
		.sort();
}

function orderedFixtureScenarios(): string[] {
	const discovered = discoverFixtureScenarios();
	const discoveredSet = new Set(discovered);
	const expectedSet = new Set<string>(EXPECTED_SCENARIOS);
	const missing = EXPECTED_SCENARIOS.filter((name) => !discoveredSet.has(name));
	const extra = discovered.filter((name) => !expectedSet.has(name));

	if (missing.length > 0 || extra.length > 0) {
		throw new Error(
			`Unexpected Claude diff-creation fixtures. Missing: ${missing.join(", ") || "none"}. Extra: ${extra.join(", ") || "none"}.`,
		);
	}

	return EXPECTED_SCENARIOS.filter((name) => discoveredSet.has(name));
}

function loadFixture(name: string): {
	input: ClaudePostToolUseFixture;
	expected: string;
} {
	const dir = path.join(fixtureRoot, name);
	const input = JSON.parse(
		readFileSync(path.join(dir, "claude-post-tool-use.json"), "utf-8"),
	) as unknown;
	const expected = readFileSync(path.join(dir, "expected.patch"), "utf-8");

	if (!hasSessionId(input)) {
		throw new Error(`${name} fixture is missing a string session_id`);
	}

	return { input, expected };
}

function hasSessionId(value: unknown): value is ClaudePostToolUseFixture {
	return (
		typeof value === "object" &&
		value !== null &&
		"session_id" in value &&
		typeof value.session_id === "string"
	);
}

describe("deriveClaudeDiffTracePayload", () => {
	for (const name of orderedFixtureScenarios()) {
		test(`claude_derivation/${name}`, () => {
			const { input, expected } = loadFixture(name);
			const result = deriveClaudeDiffTracePayload({
				eventName: "PostToolUse",
				payload: input,
				now: () => FIXED_TIME,
				toolVersion: FIXED_TOOL_VERSION,
			});

			expect(result.status).toBe("derived");
			if (result.status !== "derived") {
				throw new Error(`Expected ${name} fixture to derive a diff trace`);
			}

			expect(result.payload.sessionID).toBe(input.session_id);
			expect(result.payload.time).toBe(FIXED_TIME);
			expect(result.payload.tool_name).toBe("claude");
			expect(result.payload.tool_version).toBe(FIXED_TOOL_VERSION);
			expect(result.payload.diff).toBe(expected);
			expect(Object.hasOwn(result.payload, "model_id")).toBe(false);
		});
	}
});
