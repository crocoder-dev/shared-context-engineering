use std::collections::BTreeSet;

use crate::services::error::CliError;
use crate::services::plan::extract::{Plan, PlanEntryKind};

pub const START_MARKER: &str = "<!-- sce:intent-review:start -->";
pub const END_MARKER: &str = "<!-- sce:intent-review:end -->";
pub const HEADING: &str = "## Intent review";
pub const PLANS_LABEL: &str = "**Plans:**";
pub const DECISIONS_HEADING: &str = "**Decisions requiring review**";
pub const NO_FLAGGED_DECISIONS: &str = "No decisions explicitly flagged for review.";
pub const DECISION_BRIEF_HEADING: &str = "## Decision Brief";
pub const REVIEW_REQUESTED_LABEL: &str = "**Review requested:**";
pub const PROPOSED_LABEL: &str = "**Proposed:**";
pub const REVIEW_LINK_SUFFIX: &str = "rationale, alternatives and trade-offs";
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BriefFormat {
    IntentReview,
    DecisionBrief,
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
    statement: &'a str,
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

#[cfg(test)]
pub fn render(arguments: &PrDescriptionArguments, plans: &[PlanInput]) -> Result<String, CliError> {
    render_format(arguments, plans, BriefFormat::IntentReview)
}

pub fn render_format(
    arguments: &PrDescriptionArguments,
    plans: &[PlanInput],
    format: BriefFormat,
) -> Result<String, CliError> {
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
    let plans_line = format!("{PLANS_LABEL} {}", plan_links.join(", "));

    match format {
        BriefFormat::IntentReview => Ok(render_intent_review(arguments, &plans_line, &questions)),
        BriefFormat::DecisionBrief => render_decision_brief(arguments, &plans_line, &questions),
    }
}

fn render_intent_review(
    arguments: &PrDescriptionArguments,
    plans_line: &str,
    questions: &[ReviewQuestion],
) -> String {
    let mut lines = vec![
        START_MARKER.to_string(),
        HEADING.to_string(),
        String::new(),
        plans_line.to_string(),
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

    lines.join("\n")
}

fn render_decision_brief(
    arguments: &PrDescriptionArguments,
    plans_line: &str,
    questions: &[ReviewQuestion],
) -> Result<String, CliError> {
    if let Some(question) = questions
        .iter()
        .find(|question| question.statement.is_empty())
    {
        return Err(CliError::validation(format!(
            "Decision {} in plan '{}' is marked 'Review: required' but has no proposed decision statement. Try: write the proposed decision after '- {}:' in {}.",
            question.decision_id,
            plan_label(question.path),
            question.decision_id,
            question.path
        )));
    }

    let count_noun = if questions.len() == 1 {
        "decision"
    } else {
        "decisions"
    };
    let mut blocks = vec![
        DECISION_BRIEF_HEADING.to_string(),
        format!("{REVIEW_REQUESTED_LABEL} {} {count_noun}", questions.len()),
        plans_line.to_string(),
    ];
    if questions.is_empty() {
        blocks.push(NO_FLAGGED_DECISIONS.to_string());
    }
    for (index, question) in questions.iter().enumerate() {
        blocks.push(format!(
            "### {}. {}",
            index + 1,
            escape_markdown(question.question)
        ));
        blocks.push(format!(
            "{PROPOSED_LABEL} {}",
            escape_markdown(question.statement)
        ));
        blocks.push(format!(
            "[Review {} {} — {REVIEW_LINK_SUFFIX}]({}#L{})",
            escape_markdown(plan_label(question.path)),
            question.decision_id,
            blob_url(arguments, question.path),
            question.line
        ));
    }
    Ok(format!(
        "{START_MARKER}\n{}\n\n{END_MARKER}\n",
        blocks.join("\n\n")
    ))
}

pub fn decision_brief_stdout_payload(mut brief: String) -> String {
    if brief.ends_with('\n') {
        brief.pop();
    }
    brief
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
                statement: &decision.statement,
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
