export type NoteApplyResult =
  | { kind: "noop" }
  | { kind: "copied" }
  | { kind: "merged" };

export async function readNote(
  _notesRef: string,
  _sha: string,
): Promise<string | null> {
  throw new Error("not implemented");
}

export async function applyNote(
  _notesRef: string,
  _originalSha: string,
  _destinationSha: string,
  _note: string,
): Promise<NoteApplyResult> {
  throw new Error("not implemented");
}

export async function pushNotesRef(
  _remote: string,
  _notesRef: string,
): Promise<void> {
  throw new Error("not implemented");
}
