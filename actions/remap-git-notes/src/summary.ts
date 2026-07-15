import type { MappingDecision } from "./mapping.js";

export interface RunReport {
  prNumber: number;
  baseBranch: string;
  targetBranch: string;
  notesRef: string;
  rebaseMergeDetected: boolean;
  decisions: MappingDecision[];
  mappedCount: number;
  copiedCount: number;
  skippedCount: number;
  unmappedCount: number;
  conflictCount: number;
  changed: boolean;
}

export function renderJobSummary(_report: RunReport): string {
  throw new Error("not implemented");
}
