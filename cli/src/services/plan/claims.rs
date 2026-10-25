use anyhow::Context;
use serde::Serialize;

use crate::services::error::CliError;
use crate::services::plan::extract::{PlanEntry, PlanEntryKind};
use crate::services::plan::pr_description::{PlanInput, PrDescriptionArguments};

pub const SCHEMA_VERSION: &str = "sce.plan.claims.v1";
const CLAIM_KIND: &str = "acceptance_criterion";
const CLAIM_MODALITY: &str = "intended";
const MODE_SCENARIOS: &str = "scenarios";
const MODE_INSPECTION: &str = "inspection";
const INSPECTION_ONLY: &str = "none (inspection)";
const REFERENCE_SEPARATOR: &str = ", ";
const GITHUB_URL: &str = "https://github.com";
const REVIEW_KEY: &str = "Review";
const REVIEW_REQUIRED: &str = "required";
const REVIEW_QUESTION_KEY: &str = "Review question";
const DECISIONS_KEY: &str = "Decisions";
const SCENARIOS_KEY: &str = "Scenarios";
const COVERS_KEY: &str = "Covers";
const REVIEW_CLAIMS_COMMAND: &str = "sce plan review --claims";

#[derive(Debug, Serialize)]
struct Document<'a> {
    schema_version: &'static str,
    repository: Repository<'a>,
    plans: Vec<PlanLink<'a>>,
    claims: Vec<Claim<'a>>,
}

#[derive(Debug, Serialize)]
struct Repository<'a> {
    slug: &'a str,
    head_sha: &'a str,
}

#[derive(Debug, Serialize)]
struct PlanLink<'a> {
    path: &'a str,
    url: String,
}

#[derive(Debug, Serialize)]
struct Claim<'a> {
    id: String,
    kind: &'static str,
    statement: &'a str,
    modality: &'static str,
    author_checked: Option<bool>,
    source: Source<'a>,
    decisions: Vec<Decision<'a>>,
    verification: Verification<'a>,
    tasks: Vec<Task<'a>>,
}

#[derive(Debug, Serialize)]
struct Source<'a> {
    path: &'a str,
    entry_id: &'a str,
    line: usize,
    url: String,
}

#[derive(Debug, Serialize)]
struct Decision<'a> {
    id: &'a str,
    statement: &'a str,
    boundary: &'a str,
    rationale: &'a str,
    rejected: &'a str,
    trade_offs: &'a str,
    review: Review<'a>,
    source: Source<'a>,
}

#[derive(Debug, Serialize)]
struct Review<'a> {
    required: bool,
    question: Option<&'a str>,
}

#[derive(Debug, Serialize)]
struct Verification<'a> {
    mode: &'static str,
    validate: &'a str,
    scenarios: Vec<Scenario<'a>>,
}

#[derive(Debug, Serialize)]
struct Scenario<'a> {
    id: &'a str,
    statement: &'a str,
    given: &'a str,
    when: &'a str,
    then: &'a str,
    check: &'a str,
    source: Source<'a>,
}

#[derive(Debug, Serialize)]
struct Task<'a> {
    id: &'a str,
    statement: &'a str,
    author_checked: Option<bool>,
    source: Source<'a>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ClaimsDiagnostic {
    line: usize,
    message: String,
}

struct PlanContext<'a> {
    arguments: &'a PrDescriptionArguments,
    input: &'a PlanInput,
}

pub fn render(arguments: &PrDescriptionArguments, plans: &[PlanInput]) -> Result<String, CliError> {
    let mut sorted: Vec<&PlanInput> = plans.iter().collect();
    sorted.sort_by(|left, right| left.path.cmp(&right.path));

    for input in &sorted {
        let diagnostics = plan_diagnostics(input);
        if !diagnostics.is_empty() {
            return Err(CliError::validation(render_diagnostics(
                &input.path,
                &diagnostics,
            )));
        }
    }

    let document = Document {
        schema_version: SCHEMA_VERSION,
        repository: Repository {
            slug: &arguments.repo,
            head_sha: &arguments.head,
        },
        plans: sorted
            .iter()
            .map(|input| PlanLink {
                path: &input.path,
                url: blob_url(arguments, &input.path),
            })
            .collect(),
        claims: sorted
            .iter()
            .flat_map(|&input| {
                let context = PlanContext { arguments, input };
                input
                    .plan
                    .entries_of(PlanEntryKind::AcceptanceCriterion)
                    .map(move |criterion| context.claim(criterion))
            })
            .collect(),
    };

    serde_json::to_string_pretty(&document)
        .context("Failed to serialize the plan claims document")
        .map_err(CliError::runtime)
}

impl<'a> PlanContext<'a> {
    fn claim(&self, criterion: &'a PlanEntry) -> Claim<'a> {
        Claim {
            id: format!("{}#{}", self.input.path, criterion.id),
            kind: CLAIM_KIND,
            statement: &criterion.statement,
            modality: CLAIM_MODALITY,
            author_checked: criterion.checked,
            source: self.source(criterion),
            decisions: field_value(criterion, DECISIONS_KEY)
                .map(|value| {
                    self.resolve(value, PlanEntryKind::Decision)
                        .map(|decision| self.decision(decision))
                        .collect()
                })
                .unwrap_or_default(),
            verification: self.verification(criterion),
            tasks: self
                .input
                .plan
                .entries_of(PlanEntryKind::Task)
                .filter(|task| {
                    field_value(task, COVERS_KEY)
                        .is_some_and(|value| listed_ids(value).any(|id| id == criterion.id))
                })
                .map(|task| Task {
                    id: &task.id,
                    statement: &task.statement,
                    author_checked: task.checked,
                    source: self.source(task),
                })
                .collect(),
        }
    }

    fn decision(&self, decision: &'a PlanEntry) -> Decision<'a> {
        let required = field_value(decision, REVIEW_KEY) == Some(REVIEW_REQUIRED);
        Decision {
            id: &decision.id,
            statement: &decision.statement,
            boundary: field_text(decision, "Boundary"),
            rationale: field_text(decision, "Rationale"),
            rejected: field_text(decision, "Rejected"),
            trade_offs: field_text(decision, "Trade-offs"),
            review: Review {
                required,
                question: required
                    .then(|| field_value(decision, REVIEW_QUESTION_KEY))
                    .flatten(),
            },
            source: self.source(decision),
        }
    }

    fn verification(&self, criterion: &'a PlanEntry) -> Verification<'a> {
        let validate = field_text(criterion, "Validate");
        match field_value(criterion, SCENARIOS_KEY) {
            Some(INSPECTION_ONLY) | None => Verification {
                mode: MODE_INSPECTION,
                validate,
                scenarios: Vec::new(),
            },
            Some(value) => Verification {
                mode: MODE_SCENARIOS,
                validate,
                scenarios: self
                    .resolve(value, PlanEntryKind::Scenario)
                    .map(|scenario| Scenario {
                        id: &scenario.id,
                        statement: &scenario.statement,
                        given: field_text(scenario, "Given"),
                        when: field_text(scenario, "When"),
                        then: field_text(scenario, "Then"),
                        check: field_text(scenario, "Check"),
                        source: self.source(scenario),
                    })
                    .collect(),
            },
        }
    }

    fn resolve(
        &self,
        value: &'a str,
        kind: PlanEntryKind,
    ) -> impl Iterator<Item = &'a PlanEntry> + 'a {
        let plan = &self.input.plan;
        listed_ids(value).filter_map(move |id| plan.entries_of(kind).find(|entry| entry.id == id))
    }

    fn source(&self, entry: &'a PlanEntry) -> Source<'a> {
        Source {
            path: &self.input.path,
            entry_id: &entry.id,
            line: entry.line,
            url: format!(
                "{}#L{}",
                blob_url(self.arguments, &self.input.path),
                entry.line
            ),
        }
    }
}

fn consumed_keys(kind: PlanEntryKind) -> &'static [&'static str] {
    match kind {
        PlanEntryKind::Decision => &[
            "Boundary",
            "Rationale",
            "Rejected",
            "Trade-offs",
            REVIEW_KEY,
            REVIEW_QUESTION_KEY,
        ],
        PlanEntryKind::AcceptanceCriterion => &["Validate", DECISIONS_KEY, SCENARIOS_KEY],
        PlanEntryKind::Scenario => &["Given", "When", "Then", "Check"],
        PlanEntryKind::Task => &[COVERS_KEY],
    }
}

fn plan_diagnostics(input: &PlanInput) -> Vec<ClaimsDiagnostic> {
    let mut diagnostics = Vec::new();

    for entry in &input.plan.entries {
        for key in consumed_keys(entry.kind) {
            if let Some(repeated) = entry.fields.iter().filter(|field| field.key == *key).nth(1) {
                diagnostics.push(ClaimsDiagnostic {
                    line: repeated.line,
                    message: format!("`{key}:` appears more than once on `{}`", entry.id),
                });
            }
        }
    }

    diagnostics.sort();
    diagnostics
}

fn render_diagnostics(path: &str, diagnostics: &[ClaimsDiagnostic]) -> String {
    let mut lines = vec![format!("Plan claims check failed for '{path}':")];
    lines.extend(
        diagnostics
            .iter()
            .map(|diagnostic| format!("{path}:{}: {}", diagnostic.line, diagnostic.message)),
    );
    lines.push(format!(
        "Try: merge each repeated field into one line, then rerun '{REVIEW_CLAIMS_COMMAND}'."
    ));
    lines.join("\n")
}

fn field_value<'a>(entry: &'a PlanEntry, key: &str) -> Option<&'a str> {
    entry
        .fields
        .iter()
        .find(|field| field.key == key)
        .map(|field| field.value.as_str())
}

fn field_text<'a>(entry: &'a PlanEntry, key: &str) -> &'a str {
    field_value(entry, key).unwrap_or_default()
}

fn listed_ids(value: &str) -> impl Iterator<Item = &str> {
    value.split(REFERENCE_SEPARATOR)
}

fn blob_url(arguments: &PrDescriptionArguments, path: &str) -> String {
    format!(
        "{GITHUB_URL}/{}/blob/{}/{path}",
        arguments.repo, arguments.head
    )
}
