import { afterEach, describe, expect, it } from "vitest";

import { readInputs } from "../src/main.js";

const INPUT_ENV_KEYS = [
  "INPUT_GITHUB-TOKEN",
  "INPUT_NOTES-REF",
  "INPUT_REMOTE",
  "INPUT_TARGET-BRANCH",
  "INPUT_SEARCH-DEPTH",
  "INPUT_FAIL-ON-UNMAPPED",
  "INPUT_DRY-RUN",
] as const;

function setInputs(values: Partial<Record<(typeof INPUT_ENV_KEYS)[number], string>>): void {
  for (const key of INPUT_ENV_KEYS) {
    const value = values[key];
    if (value === undefined) {
      delete process.env[key];
    } else {
      process.env[key] = value;
    }
  }
}

afterEach(() => {
  setInputs({});
});

describe("readInputs", () => {
  it("parses all inputs", () => {
    setInputs({
      "INPUT_GITHUB-TOKEN": "token-value",
      "INPUT_NOTES-REF": "refs/notes/sce-agent-trace",
      "INPUT_REMOTE": "origin",
      "INPUT_TARGET-BRANCH": "main",
      "INPUT_SEARCH-DEPTH": "50",
      "INPUT_FAIL-ON-UNMAPPED": "false",
      "INPUT_DRY-RUN": "true",
    });

    expect(readInputs()).toEqual({
      githubToken: "token-value",
      notesRef: "refs/notes/sce-agent-trace",
      remote: "origin",
      targetBranch: "main",
      searchDepth: 50,
      failOnUnmapped: false,
      dryRun: true,
    });
  });

  it("rejects a missing github-token", () => {
    setInputs({
      "INPUT_SEARCH-DEPTH": "50",
      "INPUT_FAIL-ON-UNMAPPED": "false",
      "INPUT_DRY-RUN": "false",
    });

    expect(() => readInputs()).toThrow(/github-token/);
  });

  it("rejects a non-positive search-depth", () => {
    setInputs({
      "INPUT_GITHUB-TOKEN": "token-value",
      "INPUT_SEARCH-DEPTH": "0",
      "INPUT_FAIL-ON-UNMAPPED": "false",
      "INPUT_DRY-RUN": "false",
    });

    expect(() => readInputs()).toThrow(/search-depth/);
  });
});
