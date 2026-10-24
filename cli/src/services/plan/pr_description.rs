use std::collections::BTreeSet;

use crate::services::error::CliError;
use crate::services::plan::extract::{Plan, PlanEntryKind};

pub const START_MARKER: &str = "<!-- sce:intent-review:start -->";
pub const END_MARKER: &str = "<!-- sce:intent-review:end -->";
pub const HEADING: &str = "## Intent review";
pub const PLANS_LABEL: &str = "**Plans:**";
pub const DECISIONS_HEADING: &str = "**Decisions requiring review**";
pub const NO_FLAGGED_DECISIONS: &str = "No decisions explicitly flagged for review.";
pub const MAX_REVIEW_QUESTIONS: usize = 3;
pub const SHA_LENGTH: usize = 40;
pub const PLAN_PATH_PREFIX: &str = "context/plans/";
pub const PLAN_PATH_SUFFIX: &str = ".md";
const GITHUB_URL: &str = "https://github.com";
const REVIEW_KEY: &str = "Review";
const REVIEW_REQUIRED: &str = "required";
const REVIEW_QUESTION_KEY: &str = "Review question";
const ESCAPED_CHARACTERS: [char; 12] =
    ['\\', '`', '*', '_', '[', ']', '<', '>', '!', '|', '~', '&'];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrDescriptionArguments {
    pub repo: String,
    pub head: String,
    pub paths: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanInput {
    pub path: String,
    pub plan: Plan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ReviewQuestion<'a> {
    path: &'a str,
    decision_id: &'a str,
    line: usize,
    question: &'a str,
}

pub fn parse_arguments(
    repo: &str,
    head: &str,
    paths: &[String],
) -> Result<PrDescriptionArguments, CliError> {
    if !is_repository(repo) {
        return Err(CliError::validation(format!(
            "Invalid --repo value '{repo}'. Try: pass <OWNER>/<NAME>, where OWNER uses [A-Za-z0-9-] and NAME uses [A-Za-z0-9._-] and is not '.' or '..'."
        )));
    }
    if !is_full_sha(head) {
        return Err(CliError::validation(format!(
            "Invalid --head value '{head}'. Try: pass the full {SHA_LENGTH}-character lowercase hexadecimal commit SHA, for example from 'git rev-parse HEAD'."
        )));
    }

    let mut seen = BTreeSet::new();
    for path in paths {
        if !is_plan_path(path) {
            return Err(CliError::validation(format!(
                "Invalid plan path '{path}'. Try: pass a path relative to the repository root as {PLAN_PATH_PREFIX}<FILE>{PLAN_PATH_SUFFIX}, where FILE uses [A-Za-z0-9._-] and is not '.' or '..'."
            )));
        }
        if !seen.insert(path.as_str()) {
            return Err(CliError::validation(format!(
                "Plan path '{path}' is given more than once. Try: pass each plan path once."
            )));
        }
    }

    Ok(PrDescriptionArguments {
        repo: repo.to_string(),
        head: head.to_string(),
        paths: paths.to_vec(),
    })
}

fn is_repository(value: &str) -> bool {
    let Some((owner, name)) = value.split_once('/') else {
        return false;
    };
    !owner.is_empty()
        && owner
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        && is_safe_segment(name)
}

fn is_full_sha(value: &str) -> bool {
    value.len() == SHA_LENGTH
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_plan_path(value: &str) -> bool {
    value
        .strip_prefix(PLAN_PATH_PREFIX)
        .and_then(|file| file.strip_suffix(PLAN_PATH_SUFFIX))
        .is_some_and(is_safe_segment)
}

fn is_safe_segment(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

pub fn render(arguments: &PrDescriptionArguments, plans: &[PlanInput]) -> Result<String, CliError> {
    let mut sorted: Vec<&PlanInput> = plans.iter().collect();
    sorted.sort_by(|left, right| left.path.cmp(&right.path));

    let questions: Vec<ReviewQuestion> = sorted
        .iter()
        .flat_map(|input| review_questions(input))
        .collect();
    if questions.len() > MAX_REVIEW_QUESTIONS {
        let marked: Vec<String> = questions
            .iter()
            .map(|question| format!("{} {}", plan_label(question.path), question.decision_id))
            .collect();
        return Err(CliError::validation(format!(
            "Found {} decisions marked 'Review: required'; at most {MAX_REVIEW_QUESTIONS} are allowed: {}. Try: remove 'Review:' and 'Review question:' from the decisions that do not need senior review, or split the plans into separate pull requests.",
            questions.len(),
            marked.join(", ")
        )));
    }

    let plan_links: Vec<String> = sorted
        .iter()
        .map(|input| {
            format!(
                "[{}]({})",
                escape_markdown(plan_label(&input.path)),
                blob_url(arguments, &input.path)
            )
        })
        .collect();

    let mut lines = vec![
        START_MARKER.to_string(),
        HEADING.to_string(),
        String::new(),
        format!("{PLANS_LABEL} {}", plan_links.join(", ")),
        String::new(),
    ];
    if questions.is_empty() {
        lines.push(NO_FLAGGED_DECISIONS.to_string());
    } else {
        lines.push(DECISIONS_HEADING.to_string());
        for (index, question) in questions.iter().enumerate() {
            lines.push(String::new());
            lines.push(format!(
                "{}. {} — [{} {}]({}#L{})",
                index + 1,
                escape_markdown(question.question),
                escape_markdown(plan_label(question.path)),
                question.decision_id,
                blob_url(arguments, question.path),
                question.line
            ));
        }
    }
    lines.push(String::new());
    lines.push(END_MARKER.to_string());

    Ok(lines.join("\n"))
}

fn review_questions(input: &PlanInput) -> Vec<ReviewQuestion<'_>> {
    input
        .plan
        .entries_of(PlanEntryKind::Decision)
        .filter_map(|decision| {
            let marked = decision
                .fields
                .iter()
                .any(|field| field.key == REVIEW_KEY && field.value == REVIEW_REQUIRED);
            let question = decision
                .fields
                .iter()
                .find(|field| field.key == REVIEW_QUESTION_KEY)?;
            marked.then_some(ReviewQuestion {
                path: &input.path,
                decision_id: &decision.id,
                line: decision.line,
                question: &question.value,
            })
        })
        .collect()
}

fn plan_label(path: &str) -> &str {
    path.strip_prefix(PLAN_PATH_PREFIX)
        .and_then(|file| file.strip_suffix(PLAN_PATH_SUFFIX))
        .unwrap_or(path)
}

fn blob_url(arguments: &PrDescriptionArguments, path: &str) -> String {
    format!(
        "{GITHUB_URL}/{}/blob/{}/{path}",
        arguments.repo, arguments.head
    )
}

pub fn escape_markdown(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        if ESCAPED_CHARACTERS.contains(&character) {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}
