use anyhow::{anyhow, Result};
use markdown::mdast::Node;
use markdown::{to_mdast, ParseOptions};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlanEntryKind {
    Decision,
    AcceptanceCriterion,
    Scenario,
    Task,
}

impl PlanEntryKind {
    pub fn of_id(id: &str) -> Self {
        if id.starts_with("AC") {
            Self::AcceptanceCriterion
        } else if id.starts_with('D') {
            Self::Decision
        } else if id.starts_with('S') {
            Self::Scenario
        } else {
            Self::Task
        }
    }

    fn of_section(heading: &PlanHeading) -> Option<Self> {
        match (heading.depth, heading.text.as_str()) {
            (2, "Design decisions") => Some(Self::Decision),
            (2, "Acceptance criteria") => Some(Self::AcceptanceCriterion),
            (3, "Verification scenarios") => Some(Self::Scenario),
            (2, "Task stack") => Some(Self::Task),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanHeading {
    pub depth: u8,
    pub text: String,
    pub line: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanField {
    pub key: String,
    pub value: String,
    pub line: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanEntry {
    pub id: String,
    pub kind: PlanEntryKind,
    pub checked: Option<bool>,
    pub line: usize,
    pub fields: Vec<PlanField>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutsideDefinition {
    pub id: String,
    pub line: usize,
    pub section: Option<PlanEntryKind>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Plan {
    pub headings: Vec<PlanHeading>,
    pub entries: Vec<PlanEntry>,
    pub outside_definitions: Vec<OutsideDefinition>,
    pub fenced_ids: Vec<String>,
}

impl Plan {
    pub fn entries_of(&self, kind: PlanEntryKind) -> impl Iterator<Item = &PlanEntry> {
        self.entries.iter().filter(move |entry| entry.kind == kind)
    }
}

pub fn extract(source: &str) -> Result<Plan> {
    let root = parse(source)?;
    let mut plan = Plan::default();
    let mut section = None;

    for node in root.children().into_iter().flatten() {
        match node {
            Node::Heading(heading) => {
                let heading = PlanHeading {
                    depth: heading.depth,
                    text: node.to_string(),
                    line: line_of(node),
                };
                section = PlanEntryKind::of_section(&heading);
                plan.headings.push(heading);
            }
            Node::List(_) => {
                for item in node.children().into_iter().flatten() {
                    match section.and_then(|kind| definition(source, item, kind)) {
                        Some(entry) => plan.entries.push(entry),
                        None => collect_outside_definitions(source, item, section, &mut plan),
                    }
                }
            }
            _ => collect_outside_definitions(source, node, None, &mut plan),
        }
    }

    Ok(plan)
}

fn parse(source: &str) -> Result<Node> {
    to_mdast(source, &ParseOptions::gfm())
        .map_err(|error| anyhow!("failed to parse Markdown: {error}"))
}

fn definition(source: &str, item: &Node, section: PlanEntryKind) -> Option<PlanEntry> {
    let Node::ListItem(list_item) = item else {
        return None;
    };
    let (id, _) = parse_id_token(lead_text(source, item)?)?;
    let kind = PlanEntryKind::of_id(id);
    if kind != section {
        return None;
    }

    let fields = item
        .children()
        .into_iter()
        .flatten()
        .filter(|child| matches!(child, Node::List(_)))
        .flat_map(|list| list.children().into_iter().flatten())
        .filter_map(|field_item| field(source, field_item))
        .collect();

    Some(PlanEntry {
        id: id.to_string(),
        kind,
        checked: list_item.checked,
        line: line_of(item),
        fields,
    })
}

fn field(source: &str, item: &Node) -> Option<PlanField> {
    let (key, value) = parse_field_key(lead_text(source, item)?)?;
    Some(PlanField {
        key: key.to_string(),
        value: value.to_string(),
        line: line_of(item),
    })
}

fn collect_outside_definitions(
    source: &str,
    node: &Node,
    section: Option<PlanEntryKind>,
    plan: &mut Plan,
) {
    match node {
        Node::Code(code) => collect_fenced_ids(&code.value, &mut plan.fenced_ids),
        Node::ListItem(_) => {
            if let Some((id, _)) = lead_text(source, node).and_then(parse_id_token) {
                plan.outside_definitions.push(OutsideDefinition {
                    id: id.to_string(),
                    line: line_of(node),
                    section,
                });
            }
            for child in node.children().into_iter().flatten() {
                collect_outside_definitions(source, child, None, plan);
            }
        }
        _ => {
            for child in node.children().into_iter().flatten() {
                collect_outside_definitions(source, child, None, plan);
            }
        }
    }
}

fn collect_fenced_ids(code: &str, ids: &mut Vec<String>) {
    let Ok(root) = parse(code) else {
        return;
    };
    let mut nested = Plan::default();
    collect_outside_definitions(code, &root, None, &mut nested);
    ids.extend(
        nested
            .outside_definitions
            .into_iter()
            .map(|outside| outside.id),
    );
    ids.extend(nested.fenced_ids);
}

fn lead_text<'a>(source: &'a str, item: &Node) -> Option<&'a str> {
    let paragraph = item.children()?.first()?;
    if !matches!(paragraph, Node::Paragraph(_)) {
        return None;
    }
    let position = paragraph.position()?;
    source.get(position.start.offset..position.end.offset)
}

fn line_of(node: &Node) -> usize {
    node.position().map_or(0, |position| position.start.line)
}

fn parse_id_token(text: &str) -> Option<(&str, &str)> {
    let (id, rest) = text.split_once(':')?;
    if !is_id(id) || !(rest.is_empty() || rest.starts_with(char::is_whitespace)) {
        return None;
    }
    Some((id, rest))
}

pub fn is_id(token: &str) -> bool {
    let (digits, minimum_digits) = if let Some(digits) = token.strip_prefix("AC") {
        (digits, 1)
    } else if let Some(digits) = token.strip_prefix('D').or_else(|| token.strip_prefix('S')) {
        (digits, 1)
    } else if let Some(digits) = token.strip_prefix('T') {
        return digits.len() >= 2 && digits.bytes().all(|byte| byte.is_ascii_digit());
    } else {
        return false;
    };
    digits.len() >= minimum_digits
        && digits.bytes().all(|byte| byte.is_ascii_digit())
        && !digits.starts_with('0')
}

fn parse_field_key(text: &str) -> Option<(&str, &str)> {
    let (key, rest) = text.split_once(':')?;
    let mut chars = key.chars();
    let starts_upper = chars.next().is_some_and(|first| first.is_ascii_uppercase());
    let key_chars_valid = chars.all(|ch| ch.is_ascii_alphabetic() || ch == ' ' || ch == '-');
    if !starts_upper || !key_chars_valid {
        return None;
    }
    if !(rest.is_empty() || rest.starts_with(char::is_whitespace)) {
        return None;
    }
    Some((key, rest.trim()))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::{extract, is_id, OutsideDefinition, Plan, PlanEntryKind};

    fn fixture(name: &str) -> Plan {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../config/pkl/renderers/fixtures/plan-structure")
            .join(name);
        let source = fs::read_to_string(&path).expect("fixture should be readable");
        extract(&source).expect("fixture should parse")
    }

    fn entry_lines(plan: &Plan) -> Vec<(&str, PlanEntryKind, usize)> {
        plan.entries
            .iter()
            .map(|entry| (entry.id.as_str(), entry.kind, entry.line))
            .collect()
    }

    #[test]
    fn fenced_example_plan_yields_only_out_of_fence_definitions() {
        let plan = fixture("fenced-example-plan.md");

        assert_eq!(
            entry_lines(&plan),
            vec![
                ("D1", PlanEntryKind::Decision, 37),
                ("AC1", PlanEntryKind::AcceptanceCriterion, 45),
                ("S1", PlanEntryKind::Scenario, 52),
                ("T01", PlanEntryKind::Task, 60),
            ]
        );
        assert_eq!(plan.fenced_ids, vec!["D1", "AC1", "S1", "T01"]);
        assert!(plan.outside_definitions.is_empty());
    }

    #[test]
    fn fenced_example_plan_headings_skip_fenced_headings() {
        let plan = fixture("fenced-example-plan.md");
        let headings: Vec<(u8, &str, usize)> = plan
            .headings
            .iter()
            .map(|heading| (heading.depth, heading.text.as_str(), heading.line))
            .collect();

        assert_eq!(
            headings,
            vec![
                (1, "Plan: fenced-example-plan", 1),
                (2, "Change summary", 3),
                (3, "Format at a glance", 7),
                (2, "Design decisions", 35),
                (2, "Acceptance criteria", 43),
                (3, "Verification scenarios", 50),
                (2, "Task stack", 58),
                (2, "Open questions", 69),
            ]
        );
    }

    #[test]
    fn definition_fields_are_sliced_from_source_with_lines() {
        let plan = fixture("fenced-example-plan.md");
        let task = plan
            .entries_of(PlanEntryKind::Task)
            .next()
            .expect("task should be extracted");
        let fields: Vec<(&str, &str, usize)> = task
            .fields
            .iter()
            .map(|field| (field.key.as_str(), field.value.as_str(), field.line))
            .collect();

        assert_eq!(task.checked, Some(false));
        assert_eq!(
            fields,
            vec![
                ("Task ID", "T01", 61),
                ("Covers", "AC1", 62),
                ("Scope", "In — route handler. Out — rotation policy.", 63),
                ("Dependencies", "none", 64),
                (
                    "Done when",
                    "The endpoint returns 401 for a revoked token.",
                    65
                ),
                ("Verify", "`pnpm test src/auth/refresh.test.ts`", 66),
                ("Context synchronization", "pending", 67),
            ]
        );

        let decision = plan
            .entries_of(PlanEntryKind::Decision)
            .next()
            .expect("decision should be extracted");
        assert_eq!(decision.checked, None);
    }

    #[test]
    fn misplaced_definition_yields_no_scenario_entry() {
        let plan = fixture("misplaced-definition.md");

        assert_eq!(plan.entries_of(PlanEntryKind::Scenario).count(), 0);
        assert_eq!(
            plan.outside_definitions,
            vec![OutsideDefinition {
                id: "S1".to_string(),
                line: 21,
                section: Some(PlanEntryKind::AcceptanceCriterion),
            }]
        );
        let criterion = plan
            .entries_of(PlanEntryKind::AcceptanceCriterion)
            .next()
            .expect("acceptance criterion should be extracted");
        let keys: Vec<&str> = criterion
            .fields
            .iter()
            .map(|field| field.key.as_str())
            .collect();
        assert_eq!(keys, vec!["Validate", "Decisions", "Scenarios"]);
    }

    #[test]
    fn unstructured_plan_with_fenced_structured_example_ignores_fenced_structure() {
        let plan = fixture("unstructured-plan-with-fenced-structured-example.md");

        assert_eq!(
            entry_lines(&plan),
            vec![
                ("AC1", PlanEntryKind::AcceptanceCriterion, 21),
                ("T01", PlanEntryKind::Task, 26),
            ]
        );
        assert!(!plan
            .headings
            .iter()
            .any(|heading| heading.text == "Verification scenarios"));
        assert!(plan.entries[1]
            .fields
            .iter()
            .all(|field| field.key != "Covers"));
        assert_eq!(plan.fenced_ids, vec!["S1", "T01"]);
    }

    #[test]
    fn id_tokens_follow_plan_id_grammar() {
        for valid in ["D1", "AC12", "S3", "T01", "T100"] {
            assert!(is_id(valid), "{valid} should be an ID");
        }
        for invalid in ["D0", "AC", "S01", "T1", "X1", "AC1a", ""] {
            assert!(!is_id(invalid), "{invalid} should not be an ID");
        }
    }
}
