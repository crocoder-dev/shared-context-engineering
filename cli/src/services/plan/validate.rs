use crate::services::plan::extract::{
    is_id, Plan, PlanEntry, PlanEntryKind, PlanField, PlanHeading,
};

const INSPECTION_ONLY: &str = "none (inspection)";
const VERIFICATION_SCENARIOS_HEADING: &str = "### Verification scenarios";
const ACCEPTANCE_CRITERIA_HEADING: &str = "## Acceptance criteria";
const REQUIRED_PLAN_HEADINGS: [&str; 2] = [ACCEPTANCE_CRITERIA_HEADING, "## Task stack"];
const ORDERED_PLAN_HEADINGS: [&str; 4] = [
    "## Design decisions",
    ACCEPTANCE_CRITERIA_HEADING,
    VERIFICATION_SCENARIOS_HEADING,
    "## Task stack",
];

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct PlanDiagnostic {
    pub line: Option<usize>,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanReport {
    pub diagnostics: Vec<PlanDiagnostic>,
}

impl PlanEntryKind {
    fn authoritative_section(self) -> &'static str {
        match self {
            Self::Decision => "## Design decisions",
            Self::AcceptanceCriterion => ACCEPTANCE_CRITERIA_HEADING,
            Self::Scenario => VERIFICATION_SCENARIOS_HEADING,
            Self::Task => "## Task stack",
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::Decision => "a design decision",
            Self::AcceptanceCriterion => "an acceptance criterion",
            Self::Scenario => "a verification scenario",
            Self::Task => "a task",
        }
    }

    fn required_fields(self) -> &'static [&'static str] {
        match self {
            Self::Decision => &["Boundary", "Rationale", "Rejected", "Trade-offs"],
            Self::AcceptanceCriterion => &["Validate", "Scenarios"],
            Self::Scenario => &["Given", "When", "Then", "Check"],
            Self::Task => &["Covers"],
        }
    }
}

fn field_owner(key: &str) -> Option<PlanEntryKind> {
    match key {
        "Boundary" | "Rationale" | "Rejected" | "Trade-offs" => Some(PlanEntryKind::Decision),
        "Validate" | "Decisions" | "Scenarios" => Some(PlanEntryKind::AcceptanceCriterion),
        "Given" | "When" | "Then" | "Check" => Some(PlanEntryKind::Scenario),
        "Covers" => Some(PlanEntryKind::Task),
        _ => None,
    }
}

fn reference_kind(key: &str) -> Option<PlanEntryKind> {
    match key {
        "Decisions" => Some(PlanEntryKind::Decision),
        "Scenarios" => Some(PlanEntryKind::Scenario),
        "Covers" => Some(PlanEntryKind::AcceptanceCriterion),
        _ => None,
    }
}

fn diagnostic(line: Option<usize>, message: String) -> PlanDiagnostic {
    PlanDiagnostic { line, message }
}

fn field_of<'a>(entry: &'a PlanEntry, key: &str) -> Option<&'a PlanField> {
    entry.fields.iter().find(|field| field.key == key)
}

fn listed_ids(value: &str) -> Option<Vec<&str>> {
    let ids: Vec<&str> = value.split(", ").collect();
    ids.iter().all(|id| is_id(id)).then_some(ids)
}

fn heading_text(heading: &PlanHeading) -> String {
    format!(
        "{} {}",
        "#".repeat(usize::from(heading.depth)),
        heading.text
    )
}

pub fn validate(plan: &Plan) -> PlanReport {
    let headings: Vec<(String, usize)> = plan
        .headings
        .iter()
        .map(|heading| (heading_text(heading), heading.line))
        .collect();
    let mut diagnostics = heading_diagnostics(&headings);
    diagnostics.extend(definition_diagnostics(plan));
    diagnostics.extend(reference_diagnostics(plan));
    diagnostics.sort();

    PlanReport { diagnostics }
}

fn first_line(headings: &[(String, usize)], text: &str) -> Option<usize> {
    headings
        .iter()
        .find(|(heading, _)| heading == text)
        .map(|(_, line)| *line)
}

fn first_index(headings: &[(String, usize)], text: &str) -> Option<usize> {
    headings.iter().position(|(heading, _)| heading == text)
}

fn heading_diagnostics(headings: &[(String, usize)]) -> Vec<PlanDiagnostic> {
    let mut diagnostics: Vec<PlanDiagnostic> = REQUIRED_PLAN_HEADINGS
        .iter()
        .filter(|required| first_index(headings, required).is_none())
        .map(|required| diagnostic(None, format!("heading `{required}` is missing")))
        .collect();

    for ordered in ORDERED_PLAN_HEADINGS {
        if let Some((_, line)) = headings
            .iter()
            .filter(|(heading, _)| heading == ordered)
            .nth(1)
        {
            diagnostics.push(diagnostic(
                Some(*line),
                format!("heading `{ordered}` appears more than once"),
            ));
        }
    }

    let present: Vec<&str> = ORDERED_PLAN_HEADINGS
        .into_iter()
        .filter(|ordered| first_index(headings, ordered).is_some())
        .collect();
    for pair in present.windows(2) {
        let (earlier, later) = (pair[0], pair[1]);
        if first_index(headings, earlier) > first_index(headings, later) {
            diagnostics.push(diagnostic(
                first_line(headings, later),
                format!("heading `{later}` must come after `{earlier}`"),
            ));
        }
    }

    if let Some(index) = first_index(headings, VERIFICATION_SCENARIOS_HEADING) {
        let parent = headings[..index]
            .iter()
            .rev()
            .find(|(heading, _)| heading.starts_with("# ") || heading.starts_with("## "));
        if parent.is_none_or(|(heading, _)| heading != ACCEPTANCE_CRITERIA_HEADING) {
            diagnostics.push(diagnostic(
                Some(headings[index].1),
                format!(
                    "heading `{VERIFICATION_SCENARIOS_HEADING}` is not inside `{ACCEPTANCE_CRITERIA_HEADING}`"
                ),
            ));
        }
    }

    diagnostics
}

fn definition_diagnostics(plan: &Plan) -> Vec<PlanDiagnostic> {
    let mut diagnostics = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    let mut reported: Vec<&str> = Vec::new();

    for entry in &plan.entries {
        if seen.contains(&entry.id.as_str()) {
            if !reported.contains(&entry.id.as_str()) {
                reported.push(&entry.id);
                diagnostics.push(diagnostic(
                    Some(entry.line),
                    format!("`{}` is defined more than once", entry.id),
                ));
            }
        } else {
            seen.push(&entry.id);
        }

        for field in &entry.fields {
            if let Some(owner) = field_owner(&field.key).filter(|owner| *owner != entry.kind) {
                diagnostics.push(diagnostic(
                    Some(field.line),
                    format!(
                        "`{}:` on `{}` is misplaced; it belongs to {}",
                        field.key,
                        entry.id,
                        owner.display_name()
                    ),
                ));
            }
        }

        for key in entry.kind.required_fields() {
            if field_of(entry, key).is_none() {
                diagnostics.push(diagnostic(
                    Some(entry.line),
                    format!("`{}` is missing `{key}:`", entry.id),
                ));
            }
        }
    }

    for outside in &plan.outside_definitions {
        if let Some(section) = outside.section {
            diagnostics.push(diagnostic(
                Some(outside.line),
                format!(
                    "`{}` is defined under `{}`; it belongs under `{}`",
                    outside.id,
                    section.authoritative_section(),
                    PlanEntryKind::of_id(&outside.id).authoritative_section()
                ),
            ));
        }
    }

    diagnostics
}

fn undefined_hint(plan: &Plan, id: &str) -> String {
    if plan
        .outside_definitions
        .iter()
        .any(|outside| outside.id == id)
    {
        format!(
            "; it appears only outside `{}`",
            PlanEntryKind::of_id(id).authoritative_section()
        )
    } else if plan.fenced_ids.iter().any(|other| other == id) {
        "; it appears only inside a fenced code block".to_string()
    } else {
        String::new()
    }
}

fn reference_diagnostics(plan: &Plan) -> Vec<PlanDiagnostic> {
    let references: Vec<(&PlanEntry, &PlanField, PlanEntryKind)> = plan
        .entries
        .iter()
        .flat_map(|entry| {
            entry.fields.iter().filter_map(move |field| {
                let kind = reference_kind(&field.key)?;
                (field_owner(&field.key) == Some(entry.kind)).then_some((entry, field, kind))
            })
        })
        .collect();
    let listed_by = |key: &str| -> Vec<&str> {
        references
            .iter()
            .filter(|(_, field, _)| field.key == key)
            .filter_map(|(_, field, _)| listed_ids(&field.value))
            .flatten()
            .collect()
    };
    let covered_ids = listed_by("Covers");
    let scenario_ids = listed_by("Scenarios");
    let mut diagnostics = Vec::new();

    for (owner, field, kind) in &references {
        if field.key == "Scenarios" && field.value == INSPECTION_ONLY {
            continue;
        }
        let Some(ids) = listed_ids(&field.value) else {
            diagnostics.push(diagnostic(
                Some(field.line),
                format!(
                    "`{}:` on `{}` is not a comma-separated ID list",
                    field.key, owner.id
                ),
            ));
            continue;
        };
        for id in ids {
            if PlanEntryKind::of_id(id) != *kind {
                diagnostics.push(diagnostic(
                    Some(field.line),
                    format!(
                        "`{}:` on `{}` names `{id}`, which is not {}",
                        field.key,
                        owner.id,
                        kind.display_name()
                    ),
                ));
            } else if !plan.entries.iter().any(|entry| entry.id == id) {
                diagnostics.push(diagnostic(
                    Some(field.line),
                    format!(
                        "`{}:` on `{}` references undefined `{id}`{}",
                        field.key,
                        owner.id,
                        undefined_hint(plan, id)
                    ),
                ));
            }
        }
    }

    for entry in plan.entries_of(PlanEntryKind::AcceptanceCriterion) {
        if !covered_ids.contains(&entry.id.as_str()) {
            diagnostics.push(diagnostic(
                Some(entry.line),
                format!("`{}` is not covered by any task's `Covers:`", entry.id),
            ));
        }
    }
    for entry in plan.entries_of(PlanEntryKind::Scenario) {
        if !scenario_ids.contains(&entry.id.as_str()) {
            diagnostics.push(diagnostic(
                Some(entry.line),
                format!(
                    "`{}` is not referenced by any acceptance criterion's `Scenarios:`",
                    entry.id
                ),
            ));
        }
    }

    diagnostics
}
