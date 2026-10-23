use std::collections::HashSet;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Decision,
    Criterion,
    Scenario,
    Task,
}

impl Kind {
    const fn heading(self) -> &'static str {
        match self {
            Self::Decision => "## Design decisions",
            Self::Criterion => "## Acceptance criteria",
            Self::Scenario => "### Verification scenarios",
            Self::Task => "## Task stack",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Decision => "design decision",
            Self::Criterion => "acceptance criterion",
            Self::Scenario => "verification scenario",
            Self::Task => "task",
        }
    }

    const fn required_fields(self) -> &'static [&'static str] {
        match self {
            Self::Decision => &["Boundary", "Rationale", "Rejected", "Trade-offs"],
            Self::Criterion => &["Validate", "Scenarios"],
            Self::Scenario => &["Given", "When", "Then", "Check"],
            Self::Task => &["Covers"],
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Issue {
    pub line: usize,
    pub message: String,
}

pub struct Report {
    pub structured: bool,
    pub issues: Vec<Issue>,
}

struct Field<'a> {
    key: &'a str,
    value: &'a str,
    line: usize,
}

struct Entry<'a> {
    id: &'a str,
    kind: Kind,
    line: usize,
    indent: usize,
    fields: Vec<Field<'a>>,
}

impl<'a> Entry<'a> {
    fn field(&self, key: &str) -> Option<&Field<'a>> {
        self.fields.iter().find(|field| field.key == key)
    }
}

struct Heading<'a> {
    text: &'a str,
    level: usize,
    line: usize,
}

fn issue(issues: &mut Vec<Issue>, line: usize, message: impl Into<String>) {
    issues.push(Issue {
        line,
        message: message.into(),
    });
}

fn indentation(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
}

fn fence(line: &str) -> Option<(u8, usize, &str)> {
    let trimmed = line.trim_start();
    let marker = *trimmed.as_bytes().first()?;
    if marker != b'~' && marker != 0x60 {
        return None;
    }
    let count = trimmed.bytes().take_while(|b| *b == marker).count();
    (count >= 3).then_some((marker, count, &trimmed[count..]))
}

fn heading(line: &str) -> Option<(&str, usize)> {
    let level = line.bytes().take_while(|b| *b == b'#').count();
    if !(1..=6).contains(&level) || line.as_bytes().get(level) != Some(&b' ') {
        return None;
    }
    let text = line.trim_end();
    (text.len() > level + 1).then_some((text, level))
}

fn kind_of(id: &str) -> Option<Kind> {
    let (kind, digits, minimum) = if let Some(digits) = id.strip_prefix("AC") {
        (Kind::Criterion, digits, 1)
    } else if let Some(digits) = id.strip_prefix('D') {
        (Kind::Decision, digits, 1)
    } else if let Some(digits) = id.strip_prefix('S') {
        (Kind::Scenario, digits, 1)
    } else if let Some(digits) = id.strip_prefix('T') {
        (Kind::Task, digits, 2)
    } else {
        return None;
    };
    (digits.len() >= minimum
        && digits.bytes().all(|b| b.is_ascii_digit())
        && (kind == Kind::Task || !digits.starts_with('0')))
    .then_some(kind)
}

fn definition(line: &str) -> Option<(Kind, &str, usize)> {
    let indent = indentation(line);
    let rest = line[indent..].strip_prefix("- ")?;
    let rest = rest
        .strip_prefix("[ ] ")
        .or_else(|| rest.strip_prefix("[x] "))
        .unwrap_or(rest);
    let (id, suffix) = rest.split_once(':')?;
    if !suffix.is_empty() && !suffix.starts_with(' ') {
        return None;
    }
    Some((kind_of(id)?, id, indent))
}

fn field(line: &str, indent: usize, line_number: usize) -> Option<Field<'_>> {
    if indentation(line) != indent + 2 {
        return None;
    }
    let rest = line[indent + 2..].strip_prefix("- ")?;
    let (key, value) = rest.split_once(':')?;
    if !key.starts_with(|c: char| c.is_ascii_uppercase())
        || !key.chars().all(|c| c.is_ascii_alphabetic() || c == ' ' || c == '-')
    {
        return None;
    }
    Some(Field {
        key,
        value: value.trim(),
        line: line_number,
    })
}

fn field_owner(key: &str) -> Option<Kind> {
    match key {
        "Boundary" | "Rationale" | "Rejected" | "Trade-offs" => Some(Kind::Decision),
        "Validate" | "Decisions" | "Scenarios" => Some(Kind::Criterion),
        "Given" | "When" | "Then" | "Check" => Some(Kind::Scenario),
        "Covers" => Some(Kind::Task),
        _ => None,
    }
}

fn reference_kind(key: &str) -> Option<Kind> {
    match key {
        "Decisions" => Some(Kind::Decision),
        "Scenarios" => Some(Kind::Scenario),
        "Covers" => Some(Kind::Criterion),
        _ => None,
    }
}

fn scan<'a>(source: &'a str) -> (Vec<Heading<'a>>, Vec<Entry<'a>>, Vec<Issue>) {
    let mut headings = Vec::new();
    let mut entries: Vec<Entry<'a>> = Vec::new();
    let mut misplaced = Vec::new();
    let mut section = "";
    let mut current: Option<usize> = None;
    let mut open_fence: Option<(u8, usize)> = None;

    for (index, raw) in source.lines().enumerate() {
        let line = index + 1;
        let marker = fence(raw);
        if let Some((open_char, open_len)) = open_fence {
            if marker.is_some_and(|(ch, len, rest)| {
                ch == open_char && len >= open_len && rest.trim().is_empty()
            }) {
                open_fence = None;
            }
            continue;
        }
        if let Some((ch, len, _)) = marker {
            open_fence = Some((ch, len));
            continue;
        }

        if let Some((name, level)) = heading(raw) {
            headings.push(Heading {
                text: name,
                level,
                line,
            });
            section = name;
            current = None;
            continue;
        }

        if let Some((kind, id, indent)) = definition(raw) {
            current = None;
            if section == kind.heading() {
                entries.push(Entry {
                    id,
                    kind,
                    line,
                    indent,
                    fields: Vec::new(),
                });
                current = Some(entries.len() - 1);
            } else if matches!(
                section,
                "## Design decisions" | "## Acceptance criteria" | "### Verification scenarios" | "## Task stack"
            ) {
                issue(
                    &mut misplaced,
                    line,
                    format!("{id} must be defined under {}", kind.heading()),
                );
            }
            continue;
        }

        if let Some(entry_index) = current {
            let indent = entries[entry_index].indent;
            if !raw.trim().is_empty() && indentation(raw) <= indent {
                current = None;
            } else if let Some(parsed) = field(raw, indent, line) {
                entries[entry_index].fields.push(parsed);
            }
        }
    }
    (headings, entries, misplaced)
}

fn check_headings(headings: &[Heading<'_>], issues: &mut Vec<Issue>) {
    for required in ["## Acceptance criteria", "## Task stack"] {
        if !headings.iter().any(|h| h.text == required) {
            issue(issues, 1, format!("missing heading {required}"));
        }
    }
    let ordered = [
        "## Design decisions",
        "## Acceptance criteria",
        "### Verification scenarios",
        "## Task stack",
    ];
    let mut last_line = 0;
    for name in ordered {
        let found = headings.iter().filter(|h| h.text == name).collect::<Vec<_>>();
        if found.len() > 1 {
            issue(issues, found[1].line, format!("heading {name} appears more than once"));
        }
        if let Some(first) = found.first() {
            if first.line < last_line {
                issue(issues, first.line, format!("heading {name} is out of order"));
            }
            last_line = first.line;
        }
    }

    for (index, item) in headings.iter().enumerate() {
        if item.text == "### Verification scenarios" {
            let parent = headings[..index].iter().rev().find(|h| h.level <= 2);
            if !parent.is_some_and(|h| h.text == "## Acceptance criteria") {
                issue(
                    issues,
                    item.line,
                    "### Verification scenarios must be inside ## Acceptance criteria",
                );
            }
        }
    }
}

fn check_definitions(entries: &[Entry<'_>], issues: &mut Vec<Issue>) {
    let mut seen = HashSet::new();
    for entry in entries {
        if !seen.insert(entry.id) {
            issue(
                issues,
                entry.line,
                format!("{} is defined more than once", entry.id),
            );
        }
        let mut keys = HashSet::new();
        for field in &entry.fields {
            if !keys.insert(field.key) {
                issue(
                    issues,
                    field.line,
                    format!("{} repeats {}:", entry.id, field.key),
                );
            }
            if let Some(owner) = field_owner(field.key) {
                if owner != entry.kind {
                    issue(
                        issues,
                        field.line,
                        format!(
                            "{} has misplaced {}:; it belongs to a {}",
                            entry.id,
                            field.key,
                            owner.label()
                        ),
                    );
                }
            }
        }
        for key in entry.kind.required_fields() {
            match entry.field(key) {
                None => issue(
                    issues,
                    entry.line,
                    format!("{} is missing {key}:", entry.id),
                ),
                Some(value) if value.value.is_empty() => issue(
                    issues,
                    value.line,
                    format!("{} has empty {key}:", entry.id),
                ),
                Some(_) => {}
            }
        }
    }
}

fn check_references(entries: &[Entry<'_>], issues: &mut Vec<Issue>) {
    let defined: HashSet<&str> = entries.iter().map(|entry| entry.id).collect();
    let mut covered = HashSet::new();
    let mut referenced = HashSet::new();

    for entry in entries {
        for field in &entry.fields {
            if field_owner(field.key) != Some(entry.kind) {
                continue;
            }
            let Some(expected) = reference_kind(field.key) else {
                continue;
            };
            if field.key == "Scenarios" && field.value == "none (inspection)" {
                continue;
            }
            let ids: Vec<&str> = field.value.split(", ").collect();
            if ids.join(", ") != field.value
                || ids.is_empty()
                || ids.iter().any(|id| kind_of(id).is_none())
            {
                issue(
                    issues,
                    field.line,
                    format!(
                        "{}: on {} is not a comma-separated ID list",
                        field.key, entry.id
                    ),
                );
                continue;
            }
            for id in ids {
                if kind_of(id) != Some(expected) {
                    issue(
                        issues,
                        field.line,
                        format!(
                            "{}: on {} names {id}, expected a {}",
                            field.key,
                            entry.id,
                            expected.label()
                        ),
                    );
                } else if !defined.contains(id) {
                    issue(
                        issues,
                        field.line,
                        format!(
                            "{}: on {} references undefined {id}",
                            field.key, entry.id
                        ),
                    );
                } else if field.key == "Covers" {
                    covered.insert(id);
                } else if field.key == "Scenarios" {
                    referenced.insert(id);
                }
            }
        }
    }

    for entry in entries {
        if entry.kind == Kind::Criterion && !covered.contains(entry.id) {
            issue(
                issues,
                entry.line,
                format!("{} is not covered by any task", entry.id),
            );
        }
        if entry.kind == Kind::Scenario && !referenced.contains(entry.id) {
            issue(
                issues,
                entry.line,
                format!("{} is not referenced by any acceptance criterion", entry.id),
            );
        }
    }
}

pub fn check(source: &str) -> Report {
    let (headings, entries, misplaced) = scan(source);
    let structured = headings.iter().any(|h| h.text == "### Verification scenarios")
        || entries
            .iter()
            .any(|entry| entry.kind == Kind::Task && entry.field("Covers").is_some());

    if !structured {
        return Report {
            structured: false,
            issues: Vec::new(),
        };
    }

    let mut issues = misplaced;
    check_headings(&headings, &mut issues);
    check_definitions(&entries, &mut issues);
    check_references(&entries, &mut issues);
    issues.sort_by(|a, b| a.line.cmp(&b.line).then_with(|| a.message.cmp(&b.message)));

    Report {
        structured: true,
        issues,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = "# Plan: example\n\n## Design decisions\n\n- D1: Use the store\n  - Boundary: Auth\n  - Rationale: Consistency\n  - Rejected: Stateless\n  - Trade-offs: Extra read\n\n## Acceptance criteria\n\n- [ ] AC1: Revoked token rejected\n  - Validate: test\n  - Decisions: D1\n  - Scenarios: S1\n\n### Verification scenarios\n\n- S1: Revocation\n  - Given: A revoked token\n  - When: Refresh\n  - Then: 401\n  - Check: test\n\n### Full validation\n\n- test\n\n## Task stack\n\n- [ ] T01: Implement (status:todo)\n  - Task ID: T01\n  - Covers: AC1\n\n## Open questions\n\nNone.\n";

    fn has_issue(report: &Report, fragment: &str) -> bool {
        report.issues.iter().any(|issue| issue.message.contains(fragment))
    }

    #[test]
    fn accepts_structured_plan() {
        let report = check(VALID);
        assert!(report.structured);
        assert!(report.issues.is_empty(), "{:?}", report.issues);
    }

    #[test]
    fn ignores_fenced_example_with_duplicate_ids_and_heading() {
        let example = "~~~markdown\n## Design decisions\n- D1: Fake\n## Acceptance criteria\n- [ ] AC1: Fake\n### Verification scenarios\n- S1: Fake\n## Task stack\n- [ ] T01: Fake\n~~~\n";
        let document = format!("# Plan\n\n## Change summary\n\n{example}\n{VALID}");
        let report = check(&document);
        assert!(report.structured);
        assert!(report.issues.is_empty(), "{:?}", report.issues);
    }

    #[test]
    fn legacy_plan_and_fenced_structured_example_are_accepted() {
        let document = "# Plan\n\n## Acceptance criteria\n- [ ] AC1: Old format\n  - Validate: test\n\n~~~markdown\n### Verification scenarios\n- S1: Example\n~~~\n\n## Task stack\n- [ ] T01: Old task\n";
        let report = check(document);
        assert!(!report.structured);
        assert!(report.issues.is_empty());
    }

    #[test]
    fn validates_references_and_detects_duplicate_ids() {
        let document = VALID
            .replace("  - Decisions: D1", "  - Decisions: D9")
            .replace("  - Covers: AC1", "  - Covers: AC3")
            .replace("  - Check: test", "  - Check: test\n- S1: Duplicate");
        let report = check(&document);
        assert!(has_issue(&report, "references undefined D9"));
        assert!(has_issue(&report, "references undefined AC3"));
        assert!(has_issue(&report, "S1 is defined more than once"));
        assert!(has_issue(&report, "AC1 is not covered"));
    }

    #[test]
    fn requires_task_coverage_even_if_other_task_covers_every_ac() {
        let document = VALID.replace(
            "## Open questions",
            "- [ ] T02: Documentation (status:todo)\n  - Task ID: T02\n\n## Open questions",
        );
        let report = check(&document);
        assert!(has_issue(&report, "T02 is missing Covers:"));
    }

    #[test]
    fn rejects_empty_task_coverage() {
        let document = VALID.replace("  - Covers: AC1", "  - Covers:");
        assert!(has_issue(&check(&document), "T01 has empty Covers:"));
    }

    #[test]
    fn supports_inspection_only_plans() {
        let document = "# Plan\n\n## Acceptance criteria\n- [ ] AC1: Docs exist\n  - Validate: inspect\n  - Scenarios: none (inspection)\n\n## Task stack\n- [ ] T01: Docs\n  - Covers: AC1\n";
        let report = check(document);
        assert!(report.structured);
        assert!(report.issues.is_empty(), "{:?}", report.issues);
    }

    #[test]
    fn rejects_misplaced_fields_and_wrong_heading_context() {
        let document = VALID
            .replace("  - Rationale: Consistency", "  - Rationale: Consistency\n  - Check: misplaced")
            .replace("### Verification scenarios", "## Different section\n### Verification scenarios");
        let report = check(&document);
        assert!(has_issue(&report, "misplaced Check:"));
        assert!(has_issue(&report, "must be inside ## Acceptance criteria"));
    }

    #[test]
    fn ignores_validation_report_and_task_scope_prose() {
        let document = format!("{VALID}\n## Validation Report\n\n### Scenario verification\n- S1 (AC1): Verified\n");
        let report = check(&document);
        assert!(report.issues.is_empty(), "{:?}", report.issues);
    }
}
