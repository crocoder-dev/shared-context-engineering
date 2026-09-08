from pathlib import Path
import subprocess

PR_HEAD = "1d6552b6244155677f11890675569c5d6160d589"
PR_BRANCH = "codex/compress-workflow-preambles"
TARGETS = [".pi", ".claude", ".agents"]


def run(*args):
    subprocess.run(list(args), check=True)


def replace_exact(text, old, new, expected=1, label="replacement"):
    count = text.count(old)
    if count != expected:
        raise SystemExit(f"{label}: expected {expected} occurrence(s), found {count}")
    return text.replace(old, new)


run("git", "config", "user.name", "David Abram")
run("git", "config", "user.email", "david@crocoder.dev")
run("git", "checkout", "--detach", PR_HEAD)

before_lines = {}
for target in TARGETS:
    paths = [
        Path(target) / "skills/sce-commit/SKILL.md",
        Path(target) / "skills/sce-commit/references/atomic-commit.md",
    ]
    before_lines[target] = sum(len(path.read_text().splitlines()) for path in paths)

old_package_exec = """    Follow the **Bypass execution handoff** in the Atomic commit reference:

    1. Create the commit-message temp file outside the repository working tree,
       and write the returned `message` verbatim to it using a file-writing
       operation. Do not interpolate the multiline message into shell source or a
       shell command.
    2. Run `git commit -F <message-file>` exactly once.
    3. Only after that command succeeds, retrieve the commit hash explicitly with
       `git rev-parse --verify HEAD^{commit}`. Do not parse Git's human-readable
       output.
    4. Delete the temp file after the commit attempt, including on failure, where
       practical.
"""
new_package_exec = """    Follow the **Bypass execution handoff** in the Atomic commit reference exactly
    as written. That handoff is the sole owner of the execution sequence; do not
    reconstruct, supplement, or restate it here.
"""

old_package_failure = """    On failure, report Git's failure unchanged and do not retry, amend, stage more
    files, or fabricate a commit hash.
"""
new_package_failure = """    The handoff owns commit-failure handling. This workflow owns only the matching
    user-visible result layout above.
"""

old_composite_exec = """Follow the **Bypass execution handoff** in `references/atomic-commit.md`:

1. Create the commit-message temp file outside the repository working tree, and
   write the returned `message` verbatim to it using a file-writing operation. Do
   not interpolate the multiline message into shell source or a shell command.
2. Run `git commit -F <message-file>` exactly once.
3. Only after that command succeeds, retrieve the commit hash explicitly with
   `git rev-parse --verify HEAD^{commit}`. Do not parse Git's human-readable
   output.
4. Delete the temp file after the commit attempt, including on failure, where
   practical.
"""
new_composite_exec = """Follow the **Bypass execution handoff** in `references/atomic-commit.md` exactly
as written. That handoff is the sole owner of the execution sequence; do not
reconstruct, supplement, or restate it here.
"""

old_composite_failure = """Do not retry, do not amend, do not stage additional files, and do not fabricate a
commit hash.
"""
new_composite_failure = """The handoff owns commit-failure handling. This workflow owns only the matching
user-visible result layout above.
"""

old_handoff_intro = """    This phase returns the message; the invoking `/commit` workflow performs the
    bypass commit. When the mode is `bypass`, the invoking workflow must:
"""
new_handoff_intro = """    This subsection is the sole definition of the bypass execution sequence. This
    phase returns the message; when the mode is `bypass`, the invoking `/commit`
    workflow performs this handoff exactly as written:
"""

old_handoff_intro_rendered = """This phase returns the message; the invoking `/commit` workflow performs the
bypass commit. When the mode is `bypass`, the invoking workflow must:
"""
new_handoff_intro_rendered = """This subsection is the sole definition of the bypass execution sequence. This
phase returns the message; when the mode is `bypass`, the invoking `/commit`
workflow performs this handoff exactly as written:
"""

source_path = Path("config/pkl/base/workflow-commit.pkl")
text = source_path.read_text()
text = replace_exact(text, old_package_exec, new_package_exec, 1, "package bypass execution duplicate")
text = replace_exact(text, old_package_failure, new_package_failure, 1, "package bypass failure duplicate")
text = replace_exact(text, old_composite_exec, new_composite_exec, 1, "composite bypass execution duplicate")
text = replace_exact(text, old_composite_failure, new_composite_failure, 1, "composite bypass failure duplicate")
text = replace_exact(text, old_handoff_intro, new_handoff_intro, 1, "atomic handoff ownership intro")
source_path.write_text(text)

for target in TARGETS:
    skill_path = Path(target) / "skills/sce-commit/SKILL.md"
    text = skill_path.read_text()
    text = replace_exact(text, old_composite_exec, new_composite_exec, 1, f"{target} bypass execution duplicate")
    text = replace_exact(text, old_composite_failure, new_composite_failure, 1, f"{target} bypass failure duplicate")
    skill_path.write_text(text)

    atomic_path = Path(target) / "skills/sce-commit/references/atomic-commit.md"
    text = atomic_path.read_text()
    text = replace_exact(text, old_handoff_intro_rendered, new_handoff_intro_rendered, 1, f"{target} atomic handoff ownership intro")
    atomic_path.write_text(text)

contract_path = Path("config/pkl/renderers/generation-contract-check.pkl")
text = contract_path.read_text()
anchor = """hidden assertNextTaskReportOwnership = (documents: Mapping) ->
"""
assertion = """local requiredCommitBypassExecutionTokens = new Listing {
  "This subsection is the sole definition of the bypass execution sequence."
  "write the returned `message` verbatim"
  "`git commit -F <message-file>` exactly once"
  "`git rev-parse --verify HEAD^{commit}`"
  "Delete the temp file after the commit attempt"
  "Never retry, amend"
}

local forbiddenCommitBypassEntrypointSequenceTokens = new Listing {
  "write the returned `message` verbatim"
  "`git commit -F <message-file>` exactly once"
  "`git rev-parse --verify HEAD^{commit}`"
  "Delete the temp file after the commit attempt"
}

hidden assertCommitBypassExecutionOwnership = (documents: Mapping) ->
  if (
    documents.every((path, text) ->
      if (path.endsWith("/skills/sce-commit/references/atomic-commit.md"))
        requiredCommitBypassExecutionTokens.every((token) -> text.contains(token))
      else if (path.endsWith("/skills/sce-commit/SKILL.md"))
        text.contains("Follow the **Bypass execution handoff** in `references/atomic-commit.md` exactly")
        && text.contains("That handoff is the sole owner of the execution sequence")
        && forbiddenCommitBypassEntrypointSequenceTokens.every((token) -> !text.contains(token))
      else true
    )
  ) "sce-commit bypass execution: atomic reference solely owns the exact execution sequence"
  else throw("sce-commit must define the exact bypass temp-file/commit/hash/cleanup sequence only in references/atomic-commit.md; SKILL.md may invoke that handoff but must not restate it")

"""
if assertion not in text:
    text = replace_exact(text, anchor, assertion + anchor, 1, "commit bypass ownership assertion insertion")
check_anchor = """  ["atomic-commit-content"] = assertAtomicCommitContent.apply(workflowDocuments)
"""
check_line = """  ["commit-bypass-execution-ownership"] = assertCommitBypassExecutionOwnership.apply(workflowDocuments)
"""
if check_line not in text:
    text = replace_exact(text, check_anchor, check_anchor + check_line, 1, "commit bypass ownership check registration")
contract_path.write_text(text)

ownership_path = Path("context/sce/dedup-ownership-table.md")
ownership = ownership_path.read_text()
ownership_anchor = "| Staged-diff analysis and commit-message authoring | `sce-atomic-commit` in `workflow-commit.pkl` | `/commit`; composed into `sce-commit`; thin OpenCode Code agent | intentional/keep |"
ownership_replacement = ownership_anchor + "\n| Bypass commit execution sequence | `references/atomic-commit.md` **Bypass execution handoff**, generated from `renderAtomicCommitSkillBody` in `workflow-commit.pkl` | `/commit` executes the handoff exactly once after `bypass_message`; the workflow owns success/failure layout selection but does not restate temp-file, commit, hash, cleanup, or retry procedure | intentional/keep |"
ownership = replace_exact(ownership, ownership_anchor, ownership_replacement, 1, "ownership table atomic commit row")
ownership_path.write_text(ownership)

after_lines = {}
reductions = {}
for target in TARGETS:
    paths = [
        Path(target) / "skills/sce-commit/SKILL.md",
        Path(target) / "skills/sce-commit/references/atomic-commit.md",
    ]
    after_lines[target] = sum(len(path.read_text().splitlines()) for path in paths)
    reductions[target] = before_lines[target] - after_lines[target]

if len(set(reductions.values())) != 1:
    raise SystemExit(f"target reductions differ: {reductions}")
reduction_per_target = next(iter(reductions.values()))
if reduction_per_target <= 0:
    raise SystemExit(f"expected positive Markdown reduction, got {reduction_per_target}")

plan_path = Path("context/plans/compress-workflow-execution-preamble.md")
plan = plan_path.read_text()
if "T07: `Deduplicate commit bypass execution sequence`" not in plan:
    plan += f"""

## Follow-up: commit bypass execution ownership

- [x] T07: `Deduplicate commit bypass execution sequence` (status:done)
  - Scope: `sce-commit` bypass step 3, the atomic-commit bypass handoff, canonical
    Pkl, generated semantic checks, tracked Pi/Claude/Codex mirrors, and the ownership table.
  - Ownership after change: `references/atomic-commit.md` **Bypass execution handoff**
    solely defines the temp-file, verbatim-message write, single `git commit -F`,
    post-success `HEAD^{{commit}}` lookup, commit-failure behavior, and cleanup sequence.
    `/commit` invokes that handoff once and owns only success/failure layout selection.
  - Behavior preserved: `oneshot` and `skip` remain identical; bypass still commits
    at most once, never stages, never retries or amends, reads the hash only after a
    successful commit, reports Git failure unchanged, and cleans up the temp file where practical.
  - Result: `sce-commit/SKILL.md` plus `references/atomic-commit.md` shrink by
    {reduction_per_target} Markdown lines per tracked target, {reduction_per_target * len(TARGETS)}
    lines across Pi/Claude/Codex.
  - Verify: `pkl eval config/pkl/renderers/generation-contract-check.pkl`,
    `nix run .#pkl-check-generated`, targeted ownership assertions across all three
    tracked targets, and `git diff --check`.
  - Context synchronization: synced.
"""
    plan_path.write_text(plan)

required_atomic = [
    "This subsection is the sole definition of the bypass execution sequence.",
    "write the returned `message` verbatim",
    "`git commit -F <message-file>` exactly once",
    "`git rev-parse --verify HEAD^{commit}`",
    "Delete the temp file after the commit attempt",
]
forbidden_skill = [
    "write the returned `message` verbatim",
    "`git commit -F <message-file>` exactly once",
    "`git rev-parse --verify HEAD^{commit}`",
    "Delete the temp file after the commit attempt",
]
for target in TARGETS:
    skill = (Path(target) / "skills/sce-commit/SKILL.md").read_text()
    atomic = (Path(target) / "skills/sce-commit/references/atomic-commit.md").read_text()
    if "Follow the **Bypass execution handoff** in `references/atomic-commit.md` exactly" not in skill:
        raise SystemExit(f"{target}: missing bypass-handoff invocation")
    if "That handoff is the sole owner of the execution sequence" not in skill:
        raise SystemExit(f"{target}: missing bypass-handoff ownership boundary")
    for token in forbidden_skill:
        if token in skill:
            raise SystemExit(f"{target}: SKILL still duplicates bypass sequence: {token}")
    for token in required_atomic:
        if token not in atomic:
            raise SystemExit(f"{target}: atomic handoff lost required sequence token: {token}")

run("pkl", "eval", "config/pkl/renderers/generation-contract-check.pkl")
run("nix", "run", ".#pkl-check-generated")
run("git", "diff", "--check")

planned = {
    "config/pkl/base/workflow-commit.pkl",
    "config/pkl/renderers/generation-contract-check.pkl",
    "context/plans/compress-workflow-execution-preamble.md",
    "context/sce/dedup-ownership-table.md",
}
for target in TARGETS:
    planned.add(f"{target}/skills/sce-commit/SKILL.md")
    planned.add(f"{target}/skills/sce-commit/references/atomic-commit.md")

changed = set(subprocess.check_output(["git", "diff", "--name-only"], text=True).splitlines())
if changed != planned:
    raise SystemExit(f"unexpected changed paths: extra={sorted(changed-planned)} missing={sorted(planned-changed)}")

print(f"sce-commit SKILL + atomic reference Markdown reduction per tracked target: {reduction_per_target}")
print(f"total tracked Pi/Claude/Codex reduction: {reduction_per_target * len(TARGETS)}")
run("git", "add", *sorted(planned))
run("git", "commit", "-m", "config: Deduplicate commit bypass execution")
run("git", "push", "origin", f"HEAD:{PR_BRANCH}")
print(subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip())
