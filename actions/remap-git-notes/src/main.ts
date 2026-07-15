import * as core from "@actions/core";

export interface ActionInputs {
  githubToken: string;
  notesRef: string;
  remote: string;
  targetBranch: string;
  searchDepth: number;
  failOnUnmapped: boolean;
  dryRun: boolean;
}

export function readInputs(): ActionInputs {
  const searchDepth = Number.parseInt(core.getInput("search-depth"), 10);
  if (!Number.isInteger(searchDepth) || searchDepth <= 0) {
    throw new Error("search-depth must be a positive integer");
  }
  return {
    githubToken: core.getInput("github-token", { required: true }),
    notesRef: core.getInput("notes-ref"),
    remote: core.getInput("remote"),
    targetBranch: core.getInput("target-branch"),
    searchDepth,
    failOnUnmapped: core.getBooleanInput("fail-on-unmapped"),
    dryRun: core.getBooleanInput("dry-run"),
  };
}

export async function run(): Promise<void> {
  try {
    const inputs = readInputs();
    core.info(
      `remap-git-notes scaffold: notes-ref=${inputs.notesRef} remote=${inputs.remote} ` +
        `search-depth=${inputs.searchDepth} dry-run=${inputs.dryRun}`,
    );
    core.info("No remapping logic implemented yet; exiting cleanly.");
  } catch (error) {
    core.setFailed(error instanceof Error ? error.message : String(error));
  }
}

if (process.env.VITEST === undefined) {
  await run();
}
