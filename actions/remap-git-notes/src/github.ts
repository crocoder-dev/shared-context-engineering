export interface PullRequestEvent {
  merged: boolean;
  prNumber: number;
  baseBranch: string;
  headBranch: string;
  mergeCommitSha: string | null;
}

export interface PrCommit {
  sha: string;
  position: number;
}

export function parsePullRequestEvent(_eventPayload: unknown): PullRequestEvent {
  throw new Error("not implemented");
}

export async function listPrCommits(
  _token: string,
  _prNumber: number,
): Promise<PrCommit[]> {
  throw new Error("not implemented");
}
