export type Confidence = "very-high" | "high" | "medium" | "low" | "none";

export type MappingSignal =
  | "unique-patch-id"
  | "provenance-trailer"
  | "monotonic-sequence"
  | "content-similarity";

export interface CommitFacts {
  sha: string;
  position: number;
  patchId: string | null;
  subject: string;
  authorEmail: string;
  authorTimestamp: number;
  trailers: Record<string, string[]>;
  changedPaths: string[];
  diffstat: { insertions: number; deletions: number };
}

export interface MappingDecision {
  originalSha: string;
  rebasedSha: string | null;
  confidence: Confidence;
  signals: MappingSignal[];
  copyable: boolean;
  reason: string;
}

export function mapCommits(
  _originals: CommitFacts[],
  _candidates: CommitFacts[],
): MappingDecision[] {
  throw new Error("not implemented");
}
