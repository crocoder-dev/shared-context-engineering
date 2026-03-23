import { describe, expect, test, beforeEach, afterEach } from "bun:test";
import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  evaluateBashCommandPolicy,
  clearPresetCatalogCache,
  formatPolicyBlockMessage,
} from "./bash-policy-runtime.js";

const TEST_PRESETS = {
  schema_version: 1,
  presets: [
    {
      id: "forbid-git-all",
      match: {
        argv_prefixes: [["git"]],
      },
      message: "This repository blocks `git` via SCE bash-tool policy.",
    },
    {
      id: "forbid-git-commit",
      match: {
        argv_prefixes: [["git", "add"], ["git", "commit"], ["git", "push"]],
      },
      message: "This repository blocks direct `git add`, `git commit`, and `git push`.",
    },
    {
      id: "use-pnpm-over-npm",
      match: {
        argv_prefixes: [["npm"]],
      },
      message: "This repository prefers `pnpm` over `npm`.",
    },
    {
      id: "use-bun-over-npm",
      match: {
        argv_prefixes: [["npm"]],
      },
      message: "This repository prefers `bun` over `npm`.",
    },
    {
      id: "use-nix-flake-over-cargo",
      match: {
        argv_prefixes: [["cargo"]],
      },
      message: "This repository prefers Nix flake entrypoints over direct `cargo` commands.",
    },
  ],
  mutually_exclusive: [["use-pnpm-over-npm", "use-bun-over-npm"]],
} as const;

function createTempDir(): string {
  return mkdtempSync(join(tmpdir(), "bash-policy-test-"));
}

function createTestPresetFile(dir: string, presets: unknown = TEST_PRESETS): string {
  const presetPath = join(dir, "presets.json");
  writeFileSync(presetPath, JSON.stringify(presets));
  return presetPath;
}

function createTestConfig(worktree: string, presets: string[]): void {
  const configDir = join(worktree, ".sce");
  if (!existsSync(configDir)) {
    mkdirSync(configDir, { recursive: true });
  }
  const configPath = join(configDir, "config.json");
  writeFileSync(configPath, JSON.stringify({ policies: { bash: { presets } } }));
}

let tempDir: string;

beforeEach(() => {
  clearPresetCatalogCache();
  tempDir = createTempDir();
});

afterEach(() => {
  rmSync(tempDir, { recursive: true, force: true });
});

describe("evaluateBashCommandPolicy", () => {
  describe("with forbid-git-all preset", () => {
    test("blocks git commands", async () => {
      const presetPath = createTestPresetFile(tempDir, {
        presets: [TEST_PRESETS.presets[0]],
      });
      createTestConfig(tempDir, ["forbid-git-all"]);

      const result = await evaluateBashCommandPolicy({
        command: "git status",
        worktree: tempDir,
        pluginDirectory: tempDir,
        presetCatalogPath: presetPath,
      });

      expect(result.allowed).toBe(false);
      if (!result.allowed) {
        expect(result.policy.id).toBe("forbid-git-all");
      }
    });

    test("blocks git with wrapper commands", async () => {
      const presetPath = createTestPresetFile(tempDir, {
        presets: [TEST_PRESETS.presets[0]],
      });
      createTestConfig(tempDir, ["forbid-git-all"]);

      const result = await evaluateBashCommandPolicy({
        command: "sudo git commit",
        worktree: tempDir,
        pluginDirectory: tempDir,
        presetCatalogPath: presetPath,
      });

      expect(result.allowed).toBe(false);
      if (!result.allowed) {
        expect(result.normalizedArgv).toEqual(["git", "commit"]);
      }
    });

    test("allows non-git commands", async () => {
      const presetPath = createTestPresetFile(tempDir, {
        presets: [TEST_PRESETS.presets[0]],
      });
      createTestConfig(tempDir, ["forbid-git-all"]);

      const result = await evaluateBashCommandPolicy({
        command: "npm install",
        worktree: tempDir,
        pluginDirectory: tempDir,
        presetCatalogPath: presetPath,
      });

      expect(result.allowed).toBe(true);
    });
  });

  describe("with forbid-git-commit preset", () => {
    test("blocks git add", async () => {
      const presetPath = createTestPresetFile(tempDir, {
        presets: [TEST_PRESETS.presets[1]],
      });
      createTestConfig(tempDir, ["forbid-git-commit"]);

      const result = await evaluateBashCommandPolicy({
        command: "git add .",
        worktree: tempDir,
        pluginDirectory: tempDir,
        presetCatalogPath: presetPath,
      });

      expect(result.allowed).toBe(false);
    });

    test("blocks git commit", async () => {
      const presetPath = createTestPresetFile(tempDir, {
        presets: [TEST_PRESETS.presets[1]],
      });
      createTestConfig(tempDir, ["forbid-git-commit"]);

      const result = await evaluateBashCommandPolicy({
        command: "git commit -m 'test'",
        worktree: tempDir,
        pluginDirectory: tempDir,
        presetCatalogPath: presetPath,
      });

      expect(result.allowed).toBe(false);
    });

    test("allows git status (not in blocklist)", async () => {
      const presetPath = createTestPresetFile(tempDir, {
        presets: [TEST_PRESETS.presets[1]],
      });
      createTestConfig(tempDir, ["forbid-git-commit"]);

      const result = await evaluateBashCommandPolicy({
        command: "git status",
        worktree: tempDir,
        pluginDirectory: tempDir,
        presetCatalogPath: presetPath,
      });

      expect(result.allowed).toBe(true);
    });
  });

  describe("with use-nix-flake-over-cargo preset", () => {
    test("blocks cargo commands", async () => {
      const presetPath = createTestPresetFile(tempDir, {
        presets: [TEST_PRESETS.presets[4]],
      });
      createTestConfig(tempDir, ["use-nix-flake-over-cargo"]);

      const result = await evaluateBashCommandPolicy({
        command: "cargo build",
        worktree: tempDir,
        pluginDirectory: tempDir,
        presetCatalogPath: presetPath,
      });

      expect(result.allowed).toBe(false);
      if (!result.allowed) {
        expect(result.policy.id).toBe("use-nix-flake-over-cargo");
      }
    });

    test("blocks cargo test", async () => {
      const presetPath = createTestPresetFile(tempDir, {
        presets: [TEST_PRESETS.presets[4]],
      });
      createTestConfig(tempDir, ["use-nix-flake-over-cargo"]);

      const result = await evaluateBashCommandPolicy({
        command: "cargo test",
        worktree: tempDir,
        pluginDirectory: tempDir,
        presetCatalogPath: presetPath,
      });

      expect(result.allowed).toBe(false);
    });
  });

  describe("command normalization", () => {
    test("strips env wrapper", async () => {
      const presetPath = createTestPresetFile(tempDir, { presets: [] });

      const result = await evaluateBashCommandPolicy({
        command: "env FOO=bar npm install",
        worktree: tempDir,
        pluginDirectory: tempDir,
        presetCatalogPath: presetPath,
      });

      expect(result.allowed).toBe(true);
      if (result.allowed && result.normalizedArgv) {
        expect(result.normalizedArgv[0]).toBe("npm");
      }
    });

    test("strips nohup wrapper", async () => {
      const presetPath = createTestPresetFile(tempDir, { presets: [] });

      const result = await evaluateBashCommandPolicy({
        command: "nohup npm start &",
        worktree: tempDir,
        pluginDirectory: tempDir,
        presetCatalogPath: presetPath,
      });

      expect(result.allowed).toBe(true);
      if (result.allowed && result.normalizedArgv) {
        expect(result.normalizedArgv[0]).toBe("npm");
      }
    });

    test("strips sudo wrapper", async () => {
      const presetPath = createTestPresetFile(tempDir, { presets: [] });

      const result = await evaluateBashCommandPolicy({
        command: "sudo rm -rf /tmp/test",
        worktree: tempDir,
        pluginDirectory: tempDir,
        presetCatalogPath: presetPath,
      });

      expect(result.allowed).toBe(true);
      if (result.allowed && result.normalizedArgv) {
        expect(result.normalizedArgv[0]).toBe("rm");
      }
    });

    test("normalizes path to basename", async () => {
      const presetPath = createTestPresetFile(tempDir, { presets: [] });

      const result = await evaluateBashCommandPolicy({
        command: "/usr/bin/git status",
        worktree: tempDir,
        pluginDirectory: tempDir,
        presetCatalogPath: presetPath,
      });

      expect(result.allowed).toBe(true);
      if (result.allowed && result.normalizedArgv) {
        expect(result.normalizedArgv[0]).toBe("git");
      }
    });
  });

  describe("policy priority", () => {
    test("longer argv_prefix matches first", async () => {
      const presetPath = createTestPresetFile(tempDir, {
        presets: [TEST_PRESETS.presets[0], TEST_PRESETS.presets[1]],
      });
      createTestConfig(tempDir, ["forbid-git-all", "forbid-git-commit"]);

      const result = await evaluateBashCommandPolicy({
        command: "git commit -m 'test'",
        worktree: tempDir,
        pluginDirectory: tempDir,
        presetCatalogPath: presetPath,
      });

      expect(result.allowed).toBe(false);
      if (!result.allowed) {
        expect(result.policy.id).toBe("forbid-git-commit");
      }
    });
  });

  describe("with empty preset catalog", () => {
    test("allows all commands when no presets configured", async () => {
      const presetPath = createTestPresetFile(tempDir, { presets: [] });

      const commands = ["git commit", "npm install", "cargo build", "rm -rf /"];

      for (const command of commands) {
        const result = await evaluateBashCommandPolicy({
          command,
          worktree: tempDir,
          pluginDirectory: tempDir,
          presetCatalogPath: presetPath,
        });

        expect(result.allowed).toBe(true);
      }
    });
  });
});

describe("clearPresetCatalogCache", () => {
  test("allows reloading preset catalog after clear", async () => {
    const presetPath1 = createTestPresetFile(tempDir, {
      presets: [TEST_PRESETS.presets[0]],
    });
    createTestConfig(tempDir, ["forbid-git-all"]);

    const result1 = await evaluateBashCommandPolicy({
      command: "git status",
      worktree: tempDir,
      pluginDirectory: tempDir,
      presetCatalogPath: presetPath1,
    });

    expect(result1.allowed).toBe(false);

    clearPresetCatalogCache();

    const presetPath2 = join(tempDir, "presets2.json");
    writeFileSync(presetPath2, JSON.stringify({ presets: [] }));

    const result2 = await evaluateBashCommandPolicy({
      command: "git status",
      worktree: tempDir,
      pluginDirectory: tempDir,
      presetCatalogPath: presetPath2,
    });

    expect(result2.allowed).toBe(true);
  });
});

describe("custom policies", () => {
  test("blocks command matching custom policy", async () => {
    const presetPath = createTestPresetFile(tempDir, { presets: [] });
    mkdirSync(join(tempDir, ".sce"), { recursive: true });
    writeFileSync(
      join(tempDir, ".sce", "config.json"),
      JSON.stringify({
        policies: {
          bash: {
            presets: [],
            custom: [
              {
                id: "custom-block-rm",
                message: "rm is blocked by custom policy",
                match: {
                  argv_prefix: ["rm"],
                },
              },
            ],
          },
        },
      })
    );

    const result = await evaluateBashCommandPolicy({
      command: "rm -rf /tmp",
      worktree: tempDir,
      pluginDirectory: tempDir,
      presetCatalogPath: presetPath,
    });

    expect(result.allowed).toBe(false);
    if (!result.allowed) {
      expect(result.policy.id).toBe("custom-block-rm");
      expect(result.policy.source).toBe("custom");
    }
  });

  test("custom policy takes priority over preset", async () => {
    const presetPath = createTestPresetFile(tempDir, {
      presets: [
        {
          id: "preset-block-git",
          match: { argv_prefixes: [["git"]] },
          message: "Preset: git blocked",
        },
      ],
    });
    mkdirSync(join(tempDir, ".sce"), { recursive: true });
    writeFileSync(
      join(tempDir, ".sce", "config.json"),
      JSON.stringify({
        policies: {
          bash: {
            presets: ["preset-block-git"],
            custom: [
              {
                id: "custom-block-git",
                message: "Custom: git blocked",
                match: {
                  argv_prefix: ["git"],
                },
              },
            ],
          },
        },
      })
    );

    const result = await evaluateBashCommandPolicy({
      command: "git status",
      worktree: tempDir,
      pluginDirectory: tempDir,
      presetCatalogPath: presetPath,
    });

    expect(result.allowed).toBe(false);
    if (!result.allowed) {
      expect(result.policy.source).toBe("custom");
    }
  });

  test("longer argv_prefix takes priority regardless of source", async () => {
    const presetPath = createTestPresetFile(tempDir, {
      presets: [
        {
          id: "preset-block-git",
          match: { argv_prefixes: [["git"]] },
          message: "Preset: git blocked",
        },
      ],
    });
    mkdirSync(join(tempDir, ".sce"), { recursive: true });
    writeFileSync(
      join(tempDir, ".sce", "config.json"),
      JSON.stringify({
        policies: {
          bash: {
            presets: ["preset-block-git"],
            custom: [
              {
                id: "custom-block-git-commit",
                message: "Custom: git commit blocked",
                match: {
                  argv_prefix: ["git", "commit"],
                },
              },
            ],
          },
        },
      })
    );

    const result = await evaluateBashCommandPolicy({
      command: "git commit -m 'test'",
      worktree: tempDir,
      pluginDirectory: tempDir,
      presetCatalogPath: presetPath,
    });

    expect(result.allowed).toBe(false);
    if (!result.allowed) {
      expect(result.policy.id).toBe("custom-block-git-commit");
      expect(result.policy.argvPrefix).toEqual(["git", "commit"]);
    }
  });
});

describe("malformed custom policies", () => {
  test("ignores custom policy missing id", async () => {
    const presetPath = createTestPresetFile(tempDir, { presets: [] });
    mkdirSync(join(tempDir, ".sce"), { recursive: true });
    writeFileSync(
      join(tempDir, ".sce", "config.json"),
      JSON.stringify({
        policies: {
          bash: {
            presets: [],
            custom: [
              {
                message: "Missing id",
                match: {
                  argv_prefix: ["rm"],
                },
              },
            ],
          },
        },
      })
    );

    const result = await evaluateBashCommandPolicy({
      command: "rm -rf /tmp",
      worktree: tempDir,
      pluginDirectory: tempDir,
      presetCatalogPath: presetPath,
    });

    expect(result.allowed).toBe(true);
  });

  test("ignores custom policy missing message", async () => {
    const presetPath = createTestPresetFile(tempDir, { presets: [] });
    mkdirSync(join(tempDir, ".sce"), { recursive: true });
    writeFileSync(
      join(tempDir, ".sce", "config.json"),
      JSON.stringify({
        policies: {
          bash: {
            presets: [],
            custom: [
              {
                id: "missing-message",
                match: {
                  argv_prefix: ["rm"],
                },
              },
            ],
          },
        },
      })
    );

    const result = await evaluateBashCommandPolicy({
      command: "rm -rf /tmp",
      worktree: tempDir,
      pluginDirectory: tempDir,
      presetCatalogPath: presetPath,
    });

    expect(result.allowed).toBe(true);
  });

  test("ignores custom policy missing argv_prefix", async () => {
    const presetPath = createTestPresetFile(tempDir, { presets: [] });
    mkdirSync(join(tempDir, ".sce"), { recursive: true });
    writeFileSync(
      join(tempDir, ".sce", "config.json"),
      JSON.stringify({
        policies: {
          bash: {
            presets: [],
            custom: [
              {
                id: "missing-prefix",
                message: "Missing prefix",
              },
            ],
          },
        },
      })
    );

    const result = await evaluateBashCommandPolicy({
      command: "rm -rf /tmp",
      worktree: tempDir,
      pluginDirectory: tempDir,
      presetCatalogPath: presetPath,
    });

    expect(result.allowed).toBe(true);
  });

  test("ignores custom policy with empty argv_prefix", async () => {
    const presetPath = createTestPresetFile(tempDir, { presets: [] });
    mkdirSync(join(tempDir, ".sce"), { recursive: true });
    writeFileSync(
      join(tempDir, ".sce", "config.json"),
      JSON.stringify({
        policies: {
          bash: {
            presets: [],
            custom: [
              {
                id: "empty-prefix",
                message: "Empty prefix",
                match: {
                  argv_prefix: [],
                },
              },
            ],
          },
        },
      })
    );

    const result = await evaluateBashCommandPolicy({
      command: "rm -rf /tmp",
      worktree: tempDir,
      pluginDirectory: tempDir,
      presetCatalogPath: presetPath,
    });

    expect(result.allowed).toBe(true);
  });

  test("ignores custom policy with non-string argv_prefix elements", async () => {
    const presetPath = createTestPresetFile(tempDir, { presets: [] });
    mkdirSync(join(tempDir, ".sce"), { recursive: true });
    writeFileSync(
      join(tempDir, ".sce", "config.json"),
      JSON.stringify({
        policies: {
          bash: {
            presets: [],
            custom: [
              {
                id: "invalid-prefix-type",
                message: "Invalid prefix type",
                match: {
                  argv_prefix: ["rm", 123, null],
                },
              },
            ],
          },
        },
      })
    );

    const result = await evaluateBashCommandPolicy({
      command: "rm -rf /tmp",
      worktree: tempDir,
      pluginDirectory: tempDir,
      presetCatalogPath: presetPath,
    });

    expect(result.allowed).toBe(true);
  });

  test("ignores custom policy with empty string in argv_prefix", async () => {
    const presetPath = createTestPresetFile(tempDir, { presets: [] });
    mkdirSync(join(tempDir, ".sce"), { recursive: true });
    writeFileSync(
      join(tempDir, ".sce", "config.json"),
      JSON.stringify({
        policies: {
          bash: {
            presets: [],
            custom: [
              {
                id: "empty-string-prefix",
                message: "Empty string in prefix",
                match: {
                  argv_prefix: ["rm", ""],
                },
              },
            ],
          },
        },
      })
    );

    const result = await evaluateBashCommandPolicy({
      command: "rm -rf /tmp",
      worktree: tempDir,
      pluginDirectory: tempDir,
      presetCatalogPath: presetPath,
    });

    expect(result.allowed).toBe(true);
  });
});

describe("missing catalog file", () => {
  test("allows all commands when catalog file does not exist", async () => {
    createTestConfig(tempDir, ["forbid-git-all"]);
    const nonexistentPath = join(tempDir, "nonexistent-presets.json");

    const result = await evaluateBashCommandPolicy({
      command: "git status",
      worktree: tempDir,
      pluginDirectory: tempDir,
      presetCatalogPath: nonexistentPath,
    });

    expect(result.allowed).toBe(true);
  });

  test("allows command when default catalog path does not exist", async () => {
    createTestConfig(tempDir, ["forbid-git-all"]);

    const result = await evaluateBashCommandPolicy({
      command: "git status",
      worktree: tempDir,
      pluginDirectory: tempDir,
    });

    expect(result.allowed).toBe(true);
  });
});

describe("invalid catalog JSON", () => {
  test("allows all commands when catalog has invalid JSON", async () => {
    const presetPath = join(tempDir, "invalid-presets.json");
    writeFileSync(presetPath, "{ invalid json }");
    createTestConfig(tempDir, ["forbid-git-all"]);

    const result = await evaluateBashCommandPolicy({
      command: "git status",
      worktree: tempDir,
      pluginDirectory: tempDir,
      presetCatalogPath: presetPath,
    });

    expect(result.allowed).toBe(true);
  });

  test("handles malformed presets array", async () => {
    const presetPath = join(tempDir, "malformed-presets.json");
    writeFileSync(presetPath, JSON.stringify({ presets: "not-an-array" }));
    createTestConfig(tempDir, ["forbid-git-all"]);

    const result = await evaluateBashCommandPolicy({
      command: "git status",
      worktree: tempDir,
      pluginDirectory: tempDir,
      presetCatalogPath: presetPath,
    });

    expect(result.allowed).toBe(true);
  });

  test("handles missing presets field", async () => {
    const presetPath = join(tempDir, "missing-presets.json");
    writeFileSync(presetPath, JSON.stringify({ other_field: "value" }));
    createTestConfig(tempDir, ["forbid-git-all"]);

    const result = await evaluateBashCommandPolicy({
      command: "git status",
      worktree: tempDir,
      pluginDirectory: tempDir,
      presetCatalogPath: presetPath,
    });

    expect(result.allowed).toBe(true);
  });

  test("handles preset with missing id", async () => {
    const presetPath = join(tempDir, "missing-id.json");
    writeFileSync(
      presetPath,
      JSON.stringify({
        presets: [
          {
            message: "No id",
            match: { argv_prefixes: [["git"]] },
          },
        ],
      })
    );
    createTestConfig(tempDir, ["some-preset"]);

    const result = await evaluateBashCommandPolicy({
      command: "git status",
      worktree: tempDir,
      pluginDirectory: tempDir,
      presetCatalogPath: presetPath,
    });

    expect(result.allowed).toBe(true);
  });
});

describe("formatPolicyBlockMessage", () => {
  test("formats message correctly", () => {
    const policy = {
      id: "test-policy",
      message: "Test message",
      argvPrefix: ["test"],
      source: "custom" as const,
      order: 0,
    };

    const message = formatPolicyBlockMessage(policy);
    expect(message).toBe("Blocked by SCE bash-tool policy 'test-policy': Test message");
  });
});