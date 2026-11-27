use std::fmt::{self, Write};

use markdown::mdast::{ListItem, Node};
use markdown::ParseOptions;
use sha2::{Digest, Sha256};

use super::{ContextSync, PlanStatus, Task, TaskStatus, Validation, ValidationStatus};

const SECTION_DEPTH: u8 = 2;
const TASK_STACK_HEADING: &str = "Task stack";
const VALIDATION_REPORT_HEADING: &str = "Validation Report";
const CONTEXT_SYNC_FIELD: &str = "Context synchronization:";
const STATUS_MARKER: &str = "(status:";
const VALIDATION_STATUS_FIELD: &str = "Status:";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlanParseError {
    Markdown(String),
    MissingTaskStack,
    DuplicateSection {
        heading: &'static str,
        line: usize,
    },
    EmptyTaskStack {
        line: usize,
    },
    MalformedTask {
        line: usize,
    },
    DuplicateTaskId {
        id: String,
        line: usize,
    },
    MissingStatusMarker {
        id: String,
        line: usize,
    },
    UnsupportedStatus {
        id: String,
        value: String,
        line: usize,
    },
    StatusMismatch {
        id: String,
        checked: bool,
        line: usize,
    },
    UnsupportedContextSync {
        id: String,
        value: String,
        line: usize,
    },
    DuplicateContextSync {
        id: String,
        line: usize,
    },
    MissingValidationStatus {
        line: usize,
    },
    DuplicateValidationStatus {
        line: usize,
    },
    UnsupportedValidationStatus {
        value: String,
        line: usize,
    },
}

impl fmt::Display for PlanParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Markdown(message) => write!(f, "failed to parse Markdown: {message}"),
            Self::MissingTaskStack => write!(f, "plan has no '## {TASK_STACK_HEADING}' section"),
            Self::DuplicateSection { heading, line } => {
                write!(f, "line {line}: duplicate '## {heading}' section")
            }
            Self::EmptyTaskStack { line } => {
                write!(f, "line {line}: '## {TASK_STACK_HEADING}' contains no tasks")
            }
            Self::MalformedTask { line } => write!(
                f,
                "line {line}: malformed task entry; expected '- [ ] T01: Title (status:todo)'"
            ),
            Self::DuplicateTaskId { id, line } => {
                write!(f, "line {line}: task {id} is declared more than once")
            }
            Self::MissingStatusMarker { id, line } => write!(
                f,
                "line {line}: task {id} has no '(status:todo)' or '(status:done)' marker"
            ),
            Self::UnsupportedStatus { id, value, line } => write!(
                f,
                "line {line}: task {id} has unsupported status '{value}'; expected 'todo' or 'done'"
            ),
            Self::StatusMismatch { id, checked, line } => {
                let (checkbox, expected) = if *checked {
                    ("checked", "done")
                } else {
                    ("unchecked", "todo")
                };
                write!(
                    f,
                    "line {line}: task {id} is {checkbox} but its status marker is not 'status:{expected}'"
                )
            }
            Self::UnsupportedContextSync { id, value, line } => write!(
                f,
                "line {line}: task {id} has unsupported context synchronization '{value}'; expected 'pending', 'synced', or 'blocked'"
            ),
            Self::DuplicateContextSync { id, line } => write!(
                f,
                "line {line}: task {id} declares context synchronization more than once"
            ),
            Self::MissingValidationStatus { line } => write!(
                f,
                "line {line}: '## {VALIDATION_REPORT_HEADING}' has no '**Status:**' field"
            ),
            Self::DuplicateValidationStatus { line } => write!(
                f,
                "line {line}: '## {VALIDATION_REPORT_HEADING}' declares '**Status:**' more than once"
            ),
            Self::UnsupportedValidationStatus { value, line } => write!(
                f,
                "line {line}: unsupported validation status '{value}'; expected 'validated' or 'failed'"
            ),
        }
    }
}

impl std::error::Error for PlanParseError {}

/// Extracts persisted task and validation state from SCE plan Markdown.
pub fn parse(source: &str) -> Result<PlanStatus, PlanParseError> {
    let root = markdown::to_mdast(source, &ParseOptions::gfm())
        .map_err(|message| PlanParseError::Markdown(message.to_string()))?;
    let nodes = root.children().map_or(&[][..], Vec::as_slice);

    let mut tasks = None;
    let mut validation = None;
    for (index, node) in nodes.iter().enumerate() {
        let Node::Heading(heading) = node else {
            continue;
        };
        if heading.depth != SECTION_DEPTH {
            continue;
        }
        let end = nodes[index + 1..]
            .iter()
            .position(|next| matches!(next, Node::Heading(h) if h.depth <= SECTION_DEPTH))
            .map_or(nodes.len(), |offset| index + 1 + offset);
        let body = &nodes[index + 1..end];
        let line = line_of(node);

        match node.to_string().trim() {
            TASK_STACK_HEADING => {
                if tasks.is_some() {
                    return Err(PlanParseError::DuplicateSection {
                        heading: TASK_STACK_HEADING,
                        line,
                    });
                }
                tasks = Some(parse_tasks(body, line)?);
            }
            VALIDATION_REPORT_HEADING => {
                if validation.is_some() {
                    return Err(PlanParseError::DuplicateSection {
                        heading: VALIDATION_REPORT_HEADING,
                        line,
                    });
                }
                let report_start = start_offset(node);
                let report_end = nodes.get(end).map_or(source.len(), start_offset);
                validation = Some(Validation {
                    status: parse_validation_status(body, line)?,
                    report_sha256: sha256_hex(&source[report_start..report_end]),
                });
            }
            _ => {}
        }
    }

    Ok(PlanStatus {
        tasks: tasks.ok_or(PlanParseError::MissingTaskStack)?,
        validation,
    })
}

fn parse_tasks(body: &[Node], heading_line: usize) -> Result<Vec<Task>, PlanParseError> {
    let mut tasks: Vec<Task> = Vec::new();
    for item in body.iter().filter_map(list_items).flatten() {
        let line = line_of(item);
        let Node::ListItem(item) = item else {
            return Err(PlanParseError::MalformedTask { line });
        };
        let task = parse_task(item, line)?;
        if tasks.iter().any(|existing| existing.id == task.id) {
            return Err(PlanParseError::DuplicateTaskId { id: task.id, line });
        }
        tasks.push(task);
    }
    if tasks.is_empty() {
        return Err(PlanParseError::EmptyTaskStack { line: heading_line });
    }
    Ok(tasks)
}

fn parse_task(item: &ListItem, line: usize) -> Result<Task, PlanParseError> {
    let malformed = || PlanParseError::MalformedTask { line };
    let checked = item.checked.ok_or_else(malformed)?;
    let lead = lead_text(&item.children).ok_or_else(malformed)?;
    let lead = lead.split_whitespace().collect::<Vec<_>>().join(" ");
    let (id, rest) = lead
        .split_once(':')
        .filter(|(id, _)| is_task_id(id))
        .ok_or_else(malformed)?;
    let id = id.to_string();

    let Some((title, marker)) = rest
        .trim_end()
        .strip_suffix(')')
        .and_then(|rest| rest.rsplit_once(STATUS_MARKER))
    else {
        return Err(PlanParseError::MissingStatusMarker { id, line });
    };
    let title = title.trim();
    if title.is_empty() {
        return Err(malformed());
    }
    let status = match marker.trim() {
        "todo" => TaskStatus::Todo,
        "done" => TaskStatus::Done,
        value => {
            return Err(PlanParseError::UnsupportedStatus {
                id,
                value: value.to_string(),
                line,
            })
        }
    };
    if checked != (status == TaskStatus::Done) {
        return Err(PlanParseError::StatusMismatch { id, checked, line });
    }

    let mut context_sync = None;
    for field in item.children.iter().filter_map(list_items).flatten() {
        let Some(value) = field
            .children()
            .and_then(|children| lead_text(children))
            .and_then(|text| {
                text.trim()
                    .strip_prefix(CONTEXT_SYNC_FIELD)
                    .map(str::to_string)
            })
        else {
            continue;
        };
        let line = line_of(field);
        if context_sync.is_some() {
            return Err(PlanParseError::DuplicateContextSync { id, line });
        }
        context_sync = Some(match value.trim() {
            "pending" => ContextSync::Pending,
            "synced" => ContextSync::Synced,
            "blocked" => ContextSync::Blocked,
            _ => {
                return Err(PlanParseError::UnsupportedContextSync {
                    id,
                    value: value.trim().to_string(),
                    line,
                })
            }
        });
    }

    Ok(Task {
        id,
        title: title.to_string(),
        status,
        context_sync,
    })
}

fn parse_validation_status(
    body: &[Node],
    heading_line: usize,
) -> Result<ValidationStatus, PlanParseError> {
    let mut status = None;
    for paragraph in body
        .iter()
        .take_while(|node| !matches!(node, Node::Heading(_)))
        .filter(|node| matches!(node, Node::Paragraph(_)))
    {
        let text = inline_text(paragraph.children().map_or(&[][..], Vec::as_slice));
        for value in text
            .lines()
            .filter_map(|line| line.trim().strip_prefix(VALIDATION_STATUS_FIELD))
        {
            let line = line_of(paragraph);
            if status.is_some() {
                return Err(PlanParseError::DuplicateValidationStatus { line });
            }
            status = Some(match value.trim() {
                "validated" => ValidationStatus::Validated,
                "failed" => ValidationStatus::Failed,
                _ => {
                    return Err(PlanParseError::UnsupportedValidationStatus {
                        value: value.trim().to_string(),
                        line,
                    })
                }
            });
        }
    }
    status.ok_or(PlanParseError::MissingValidationStatus { line: heading_line })
}

fn list_items(node: &Node) -> Option<&Vec<Node>> {
    match node {
        Node::List(list) => Some(&list.children),
        _ => None,
    }
}

fn lead_text(children: &[Node]) -> Option<String> {
    match children.first()? {
        Node::Paragraph(paragraph) => Some(inline_text(&paragraph.children)),
        _ => None,
    }
}

/// Plain text of inline content with formatting removed and hard breaks kept as newlines.
fn inline_text(children: &[Node]) -> String {
    children
        .iter()
        .map(|node| match node {
            Node::Break(_) => "\n".to_string(),
            node => node.to_string(),
        })
        .collect()
}

fn is_task_id(token: &str) -> bool {
    token
        .strip_prefix('T')
        .is_some_and(|digits| digits.len() >= 2 && digits.bytes().all(|b| b.is_ascii_digit()))
}

fn line_of(node: &Node) -> usize {
    node.position().map_or(0, |position| position.start.line)
}

fn start_offset(node: &Node) -> usize {
    node.position().map_or(0, |position| position.start.offset)
}

fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAN: &str = "# Plan: example

## Change summary

- [ ] T90: `Not a task outside the stack` (status:todo)

## Task stack

- [x] T01: `Create database schema` (status:done)
  - Task ID: T01
  - Context synchronization: synced

- [ ] **T02: Implement repository** (status:todo)
  - Dependencies: T01
  - Context synchronization blocker: none
  - Context synchronization: pending

- [x] T03: `Document it` (status:done)
  - Context synchronization: blocked

```markdown
- [ ] T91: `Fenced example` (status:todo)
```

> - [ ] T92: `Quoted example` (status:todo)

## Open questions

- [ ] T93: `Unrelated section` (status:todo)
";

    const REPORT: &str = "
## Validation Report

**Status:** validated  
**Date:** 2026-11-27

### Commands run

- `cargo test` -> exit 0

### Failed checks and follow-ups

- None.
";

    fn task(id: &str, title: &str, status: TaskStatus, sync: Option<ContextSync>) -> Task {
        Task {
            id: id.to_string(),
            title: title.to_string(),
            status,
            context_sync: sync,
        }
    }

    fn with_task_stack(stack: &str) -> String {
        format!("# Plan\n\n## Task stack\n\n{stack}")
    }

    #[test]
    fn extracts_tasks_in_order_ignoring_examples_and_other_sections() {
        let status = parse(PLAN).unwrap();
        assert_eq!(
            status.tasks,
            vec![
                task(
                    "T01",
                    "Create database schema",
                    TaskStatus::Done,
                    Some(ContextSync::Synced)
                ),
                task(
                    "T02",
                    "Implement repository",
                    TaskStatus::Todo,
                    Some(ContextSync::Pending)
                ),
                task(
                    "T03",
                    "Document it",
                    TaskStatus::Done,
                    Some(ContextSync::Blocked)
                ),
            ]
        );
        assert_eq!(status.validation, None);
    }

    #[test]
    fn extracts_validated_and_failed_reports() {
        let validated = parse(&format!("{PLAN}{REPORT}"))
            .unwrap()
            .validation
            .unwrap();
        assert_eq!(validated.status, ValidationStatus::Validated);
        assert_eq!(validated.report_sha256.len(), 64);
        assert!(validated
            .report_sha256
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));

        let failed = REPORT.replace("validated  ", "failed  ");
        let failed = parse(&format!("{PLAN}{failed}"))
            .unwrap()
            .validation
            .unwrap();
        assert_eq!(failed.status, ValidationStatus::Failed);
    }

    #[test]
    fn report_digest_covers_exactly_the_report_section() {
        let digest = |plan: &str| parse(plan).unwrap().validation.unwrap().report_sha256;
        let base = digest(&format!("{PLAN}{REPORT}"));

        assert_eq!(base, digest(&format!("{PLAN}{REPORT}")));
        assert_eq!(base, sha256_hex(REPORT.trim_start()));
        assert_eq!(
            base,
            digest(&format!(
                "{}{REPORT}",
                PLAN.replace("Open questions", "Notes")
            ))
        );
        assert_ne!(
            base,
            digest(&format!(
                "{PLAN}{}",
                REPORT.replace("- None.", "- cargo fmt failed.")
            ))
        );
        assert_ne!(
            base,
            digest(&format!("{PLAN}{REPORT}\n### Residual risks\n\n- Some.\n"))
        );
        assert_eq!(
            base,
            digest(&format!("{PLAN}{REPORT}## Appendix\n\nLater section.\n"))
        );
    }

    #[test]
    fn legacy_task_without_sync_field_reports_none() {
        let status = parse(&with_task_stack(
            "- [x] T01: `Legacy task` (status:done)\n  - Verify: `cargo test`\n",
        ))
        .unwrap();
        assert_eq!(
            status.tasks,
            vec![task("T01", "Legacy task", TaskStatus::Done, None)]
        );
    }

    #[test]
    fn rejects_inconsistent_task_state() {
        let cases = [
            (
                "- [ ] T01: `A` (status:todo)\n- [ ] T01: `B` (status:todo)\n",
                PlanParseError::DuplicateTaskId {
                    id: "T01".into(),
                    line: 6,
                },
            ),
            (
                "- [x] T01: `A` (status:todo)\n",
                PlanParseError::StatusMismatch {
                    id: "T01".into(),
                    checked: true,
                    line: 5,
                },
            ),
            (
                "- [ ] T01: `A` (status:done)\n",
                PlanParseError::StatusMismatch {
                    id: "T01".into(),
                    checked: false,
                    line: 5,
                },
            ),
            (
                "- [ ] T01: `A`\n",
                PlanParseError::MissingStatusMarker {
                    id: "T01".into(),
                    line: 5,
                },
            ),
            (
                "- [x] T01: `A` (status:complete)\n",
                PlanParseError::UnsupportedStatus {
                    id: "T01".into(),
                    value: "complete".into(),
                    line: 5,
                },
            ),
            (
                "- T01: `A` (status:todo)\n",
                PlanParseError::MalformedTask { line: 5 },
            ),
            (
                "- [ ] Final gate: `/validate`\n",
                PlanParseError::MalformedTask { line: 5 },
            ),
            (
                "No tasks yet.\n",
                PlanParseError::EmptyTaskStack { line: 3 },
            ),
        ];
        for (stack, expected) in cases {
            assert_eq!(parse(&with_task_stack(stack)), Err(expected), "{stack}");
        }
    }

    #[test]
    fn rejects_invalid_sync_and_validation_state() {
        assert_eq!(
            parse(&with_task_stack(
                "- [ ] T01: `A` (status:todo)\n  - Context synchronization: maybe\n"
            )),
            Err(PlanParseError::UnsupportedContextSync {
                id: "T01".into(),
                value: "maybe".into(),
                line: 6,
            })
        );
        assert_eq!(
            parse(&with_task_stack(
                "- [ ] T01: `A` (status:todo)\n  - Context synchronization: pending\n  - Context synchronization: synced\n"
            )),
            Err(PlanParseError::DuplicateContextSync { id: "T01".into(), line: 7 })
        );
        for value in [
            "synced but verification failed",
            "pending, previously synced",
            "blocked — waiting on review",
            "synced.",
        ] {
            assert_eq!(
                parse(&with_task_stack(&format!(
                    "- [x] T01: `A` (status:done)\n  - Context synchronization: {value}\n"
                ))),
                Err(PlanParseError::UnsupportedContextSync {
                    id: "T01".into(),
                    value: value.into(),
                    line: 6,
                }),
                "{value}"
            );
        }

        let mut report_errors = vec![
            (
                REPORT.replace("validated  ", "passed  "),
                PlanParseError::UnsupportedValidationStatus {
                    value: "passed".into(),
                    line: 33,
                },
            ),
            (
                REPORT.replace("**Status:** validated  \n", ""),
                PlanParseError::MissingValidationStatus { line: 31 },
            ),
            (
                REPORT.replace("**Date:**", "**Status:**"),
                PlanParseError::DuplicateValidationStatus { line: 33 },
            ),
            (
                format!("{REPORT}{REPORT}"),
                PlanParseError::DuplicateSection {
                    heading: VALIDATION_REPORT_HEADING,
                    line: 44,
                },
            ),
            (
                REPORT
                    .replace("**Status:** validated  \n", "")
                    .replace("- None.", "**Status:** validated"),
                PlanParseError::MissingValidationStatus { line: 31 },
            ),
        ];
        for value in [
            "validated — actually failed",
            "validated (revalidated after T05)",
            "validated.",
        ] {
            report_errors.push((
                REPORT.replace("validated  ", &format!("{value}  ")),
                PlanParseError::UnsupportedValidationStatus {
                    value: value.into(),
                    line: 33,
                },
            ));
        }
        for (report, expected) in report_errors {
            assert_eq!(parse(&format!("{PLAN}{report}")), Err(expected), "{report}");
        }
    }

    #[test]
    fn rejects_missing_or_duplicate_task_stack() {
        assert_eq!(parse("# Plan\n"), Err(PlanParseError::MissingTaskStack));
        let stack = "- [ ] T01: `A` (status:todo)\n";
        assert_eq!(
            parse(&format!(
                "{}\n{}",
                with_task_stack(stack),
                with_task_stack(stack)
            )),
            Err(PlanParseError::DuplicateSection {
                heading: TASK_STACK_HEADING,
                line: 9
            })
        );
    }

    #[test]
    fn parses_repository_plan_excerpt() {
        let plan = "# Plan: Claude mutation-scope stable repository routing

## Task stack

- [x] **T01: Verify and document the stable Claude hook lookup anchor** (status:done)
  - Scope: read-only probe of hook `cwd` and `CLAUDE_PROJECT_DIR`.
  - Result: A real probe reproduced the defect. The module provides:
    - `persist_route` returns `Inserted`.
    - Context synchronization: nested prose, not this task's field.
  - Context impact: none.
  - Context synchronization: synced

- [ ] **T02: Add minimal durable route storage** (status:todo)
  - Dependencies: T01
  - Context synchronization blocker: Blocker: x; Required action: y
  - Context synchronization: blocked

## Validation Report

**Status:** validated
**Date:** 2026-11-24

### Commands run

- `nix flake check` -> exit 0
";
        let status = parse(plan).unwrap();
        assert_eq!(
            status.tasks,
            vec![
                task(
                    "T01",
                    "Verify and document the stable Claude hook lookup anchor",
                    TaskStatus::Done,
                    Some(ContextSync::Synced),
                ),
                task(
                    "T02",
                    "Add minimal durable route storage",
                    TaskStatus::Todo,
                    Some(ContextSync::Blocked),
                ),
            ]
        );
        assert_eq!(
            status.validation.map(|validation| validation.status),
            Some(ValidationStatus::Validated)
        );
    }
}
