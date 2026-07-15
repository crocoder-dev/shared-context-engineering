export interface GitResult {
  stdout: string;
  stderr: string;
  exitCode: number;
}

export interface CommitMetadata {
  sha: string;
  subject: string;
  authorName: string;
  authorEmail: string;
  authorTimestamp: number;
  trailers: Record<string, string[]>;
  changedPaths: string[];
  diffstat: { insertions: number; deletions: number };
}

export async function runGit(_args: string[]): Promise<GitResult> {
  throw new Error("not implemented");
}

export async function computePatchId(_sha: string): Promise<string | null> {
  throw new Error("not implemented");
}

export async function readCommitMetadata(_sha: string): Promise<CommitMetadata> {
  throw new Error("not implemented");
}

export async function listCandidateRange(
  _anchorSha: string | null,
  _branch: string,
  _count: number,
  _searchDepth: number,
): Promise<string[]> {
  throw new Error("not implemented");
}
