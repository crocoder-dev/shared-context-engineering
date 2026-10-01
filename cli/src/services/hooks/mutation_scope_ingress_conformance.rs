use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{anyhow, Result};
use serde_json::{json, Map, Value};

use crate::services::observability::traits::Logger;

pub(crate) type GitDirResolver<'a> = &'a dyn Fn(&str) -> Result<PathBuf>;

pub(crate) type IngressSeam<'a> = &'a dyn Fn(&Path, &str, Option<&dyn Logger>) -> Result<String>;

static NEXT_CONFORMANCE_GIT_DIR_ID: AtomicU64 = AtomicU64::new(0);

pub(crate) trait IngressConformance {
    const ADAPTER: &'static str;
    const EVENT_NAME_FIELD: &'static str;
    const REQUIRED_START_FIELDS: &'static [&'static str];
    const OPTIONAL_START_FIELDS: &'static [(&'static str, &'static str)];
    const UNSUPPORTED_EVENT_NAMES: &'static [&'static str];

    fn tracked_start() -> Map<String, Value>;

    fn parse(payload: &str) -> Result<()>;

    fn run(payload: &str) -> Result<String>;

    fn run_with_seams(
        payload: &str,
        resolve_git_dir: GitDirResolver,
        seam: IngressSeam,
    ) -> Result<String>;
}

pub(crate) trait CheckoutResolutionConformance: IngressConformance {
    const CWD_FIELD: &'static str;
    const FAIL_CLOSED_MESSAGE: &'static str;
}

struct Rejection {
    case: String,
    payload: String,
    detail: String,
}

fn rejection(case: impl Into<String>, payload: impl Into<String>, detail: &str) -> Rejection {
    Rejection {
        case: case.into(),
        payload: payload.into(),
        detail: detail.to_string(),
    }
}

fn start_with<A: IngressConformance>(field: &str, value: Value) -> String {
    let mut object = A::tracked_start();
    object.insert(field.to_string(), value);
    Value::Object(object).to_string()
}

fn start_without<A: IngressConformance>(fields: &[&str]) -> String {
    let mut object = A::tracked_start();
    for field in fields {
        object.remove(*field);
    }
    Value::Object(object).to_string()
}

fn required_fields<A: IngressConformance>() -> Vec<&'static str> {
    let mut fields = vec![A::EVENT_NAME_FIELD];
    fields.extend_from_slice(A::REQUIRED_START_FIELDS);
    fields
}

fn malformed_json_rejections() -> Vec<Rejection> {
    [
        ("", "got an empty payload"),
        ("   ", "got an empty payload"),
        ("{", "expected valid JSON"),
        ("{]", "expected valid JSON"),
        ("{not json", "expected valid JSON"),
    ]
    .into_iter()
    .map(|(payload, detail)| rejection(format!("malformed payload {payload:?}"), payload, detail))
    .collect()
}

fn non_object_json_rejections() -> Vec<Rejection> {
    ["[]", "\"string\"", "5", "true", "null"]
        .into_iter()
        .map(|payload| {
            rejection(
                format!("non-object payload {payload}"),
                payload,
                "expected a JSON object",
            )
        })
        .collect()
}

fn missing_required_rejections<A: IngressConformance>() -> Vec<Rejection> {
    required_fields::<A>()
        .into_iter()
        .map(|field| {
            rejection(
                format!("missing {field}"),
                start_without::<A>(&[field]),
                &format!("missing required field '{field}'"),
            )
        })
        .collect()
}

fn blank_required_rejections<A: IngressConformance>() -> Vec<Rejection> {
    let mut rejections = Vec::new();
    for field in required_fields::<A>() {
        for blank in ["", "   "] {
            rejections.push(rejection(
                format!("blank {field} {blank:?}"),
                start_with::<A>(field, json!(blank)),
                &format!("field '{field}' must be a non-blank string"),
            ));
        }
    }
    rejections
}

fn wrong_typed_required_rejections<A: IngressConformance>() -> Vec<Rejection> {
    let mut rejections = Vec::new();
    for field in required_fields::<A>() {
        for value in [json!(7), json!(false), json!([]), json!({}), Value::Null] {
            rejections.push(rejection(
                format!("wrong-typed {field} {value}"),
                start_with::<A>(field, value),
                &format!("field '{field}' must be a string"),
            ));
        }
    }
    rejections
}

fn unsupported_event_rejections<A: IngressConformance>() -> Vec<Rejection> {
    A::UNSUPPORTED_EVENT_NAMES
        .iter()
        .map(|name| {
            rejection(
                format!("unsupported event {name:?}"),
                start_with::<A>(A::EVENT_NAME_FIELD, json!(name)),
                &format!("unsupported {} '{name}'", A::EVENT_NAME_FIELD),
            )
        })
        .collect()
}

fn invalid_optional_rejections<A: IngressConformance>() -> Vec<Rejection> {
    let mut rejections = Vec::new();
    for (field, _) in A::OPTIONAL_START_FIELDS {
        for value in [
            json!(7),
            json!(false),
            json!([]),
            json!({}),
            json!(""),
            json!("   "),
        ] {
            rejections.push(rejection(
                format!("invalid optional {field} {value}"),
                start_with::<A>(field, value),
                &format!("field '{field}' must be null, absent, or a non-blank string"),
            ));
        }
    }
    rejections
}

fn assert_validation_error<A: IngressConformance, T>(
    layer: &str,
    rejection: &Rejection,
    result: Result<T>,
) {
    let Err(error) = result else {
        panic!(
            "{} {layer} accepted {}: {}",
            A::ADAPTER,
            rejection.case,
            rejection.payload
        );
    };
    let error = error.to_string();
    let prefix = format!("Invalid {} hook event payload from STDIN: ", A::ADAPTER);
    assert!(
        error.starts_with(&prefix) && error.contains(&rejection.detail),
        "{} {layer} rejected {} with an unexpected diagnostic: {error:?}",
        A::ADAPTER,
        rejection.case
    );
}

fn assert_parser_rejects<A: IngressConformance>(rejections: &[Rejection]) {
    assert!(!rejections.is_empty());
    for rejection in rejections {
        assert_validation_error::<A, _>("parser", rejection, A::parse(&rejection.payload));
    }
}

fn unresolved_git_dir<A: IngressConformance>(contract: &str) -> PathBuf {
    let id = NEXT_CONFORMANCE_GIT_DIR_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "sce-{}-mutation-scope-ingress-conformance-{contract}-{}-{id}",
        A::ADAPTER,
        std::process::id()
    ))
}

fn assert_fails_closed_before_dispatch<A: IngressConformance>(rejections: &[Rejection]) {
    assert!(!rejections.is_empty());
    for rejection in rejections {
        assert_validation_error::<A, _>(
            "production entrypoint",
            rejection,
            A::run(&rejection.payload),
        );

        let git_dir = unresolved_git_dir::<A>("fails-closed");
        let resolver_calls = Cell::new(0_usize);
        let seam_calls = Cell::new(0_usize);
        let resolver = |_cwd: &str| {
            resolver_calls.set(resolver_calls.get() + 1);
            Ok(git_dir.clone())
        };
        let seam = |_root: &Path, _payload: &str, _logger: Option<&dyn Logger>| {
            seam_calls.set(seam_calls.get() + 1);
            Ok(String::new())
        };

        let result = A::run_with_seams(&rejection.payload, &resolver, &seam);
        let fabricated_state = git_dir.exists();
        let _ = std::fs::remove_dir_all(&git_dir);

        assert_validation_error::<A, _>("seamed entrypoint", rejection, result);
        assert_eq!(
            resolver_calls.get(),
            0,
            "{} resolved a checkout for {}",
            A::ADAPTER,
            rejection.case
        );
        assert_eq!(
            seam_calls.get(),
            0,
            "{} invoked the mutation seam for {}",
            A::ADAPTER,
            rejection.case
        );
        assert!(
            !fabricated_state,
            "{} fabricated mutation-scope state for {}",
            A::ADAPTER,
            rejection.case
        );
    }
}

pub(crate) fn malformed_json_is_rejected<A: IngressConformance>() {
    assert_parser_rejects::<A>(&malformed_json_rejections());
}

pub(crate) fn non_object_json_is_rejected<A: IngressConformance>() {
    assert_parser_rejects::<A>(&non_object_json_rejections());
}

pub(crate) fn missing_required_identity_is_rejected_without_fabrication<A: IngressConformance>() {
    let rejections = missing_required_rejections::<A>();
    assert_parser_rejects::<A>(&rejections);
    assert_fails_closed_before_dispatch::<A>(&rejections);
}

pub(crate) fn blank_required_identifiers_are_rejected<A: IngressConformance>() {
    assert_parser_rejects::<A>(&blank_required_rejections::<A>());
}

pub(crate) fn wrong_typed_required_fields_are_rejected<A: IngressConformance>() {
    assert_parser_rejects::<A>(&wrong_typed_required_rejections::<A>());
}

pub(crate) fn unsupported_event_name_is_rejected<A: IngressConformance>() {
    assert_parser_rejects::<A>(&unsupported_event_rejections::<A>());
}

pub(crate) fn optional_fields_are_accepted_when_absent_null_or_string<A: IngressConformance>() {
    assert!(!A::OPTIONAL_START_FIELDS.is_empty());
    let optional_fields: Vec<&str> = A::OPTIONAL_START_FIELDS
        .iter()
        .map(|(field, _)| *field)
        .collect();

    let absent = start_without::<A>(&optional_fields);
    if let Err(error) = A::parse(&absent) {
        panic!(
            "{} rejected a tracked start without optional fields: {error:#}",
            A::ADAPTER
        );
    }

    for (field, sample) in A::OPTIONAL_START_FIELDS {
        for value in [Value::Null, json!(sample)] {
            let payload = start_with::<A>(field, value.clone());
            if let Err(error) = A::parse(&payload) {
                panic!("{} rejected {field} = {value}: {error:#}", A::ADAPTER);
            }
        }
    }
}

pub(crate) fn wrong_typed_or_blank_optional_fields_are_rejected<A: IngressConformance>() {
    assert_parser_rejects::<A>(&invalid_optional_rejections::<A>());
}

pub(crate) fn malformed_runtime_input_fails_closed_before_dispatch<A: IngressConformance>() {
    let mut rejections = malformed_json_rejections();
    rejections.extend(non_object_json_rejections());
    rejections.extend(blank_required_rejections::<A>());
    rejections.extend(wrong_typed_required_rejections::<A>());
    rejections.extend(unsupported_event_rejections::<A>());
    rejections.extend(invalid_optional_rejections::<A>());
    assert_fails_closed_before_dispatch::<A>(&rejections);
}

pub(crate) fn tracked_start_fails_closed_when_its_checkout_cannot_be_resolved<
    A: CheckoutResolutionConformance,
>() {
    let unresolvable = start_with::<A>(
        A::CWD_FIELD,
        json!(format!("/nonexistent/sce/{}/checkout", A::ADAPTER)),
    );
    let error = A::run(&unresolvable)
        .expect_err("a tracked start that cannot resolve its checkout must fail closed");
    assert!(
        error.to_string().contains(A::FAIL_CLOSED_MESSAGE),
        "{error:?}"
    );

    let resolver_calls = Cell::new(0_usize);
    let seam_calls = Cell::new(0_usize);
    let resolver = |_cwd: &str| {
        resolver_calls.set(resolver_calls.get() + 1);
        Err(anyhow!("checkout resolution failure injected by test"))
    };
    let seam = |_root: &Path, _payload: &str, _logger: Option<&dyn Logger>| {
        seam_calls.set(seam_calls.get() + 1);
        Ok(String::new())
    };

    let payload = Value::Object(A::tracked_start()).to_string();
    let error = A::run_with_seams(&payload, &resolver, &seam)
        .expect_err("a failed checkout resolution must surface as a hard failure");
    assert!(
        error.to_string().contains(A::FAIL_CLOSED_MESSAGE),
        "{error:?}"
    );
    assert_eq!(resolver_calls.get(), 1);
    assert_eq!(seam_calls.get(), 0);
}

macro_rules! mutation_scope_ingress_conformance_tests {
    ($adapter:ty) => {
        $crate::services::hooks::mutation_scope_ingress_conformance::mutation_scope_ingress_conformance_tests! {
            $adapter =>
            malformed_json_is_rejected,
            non_object_json_is_rejected,
            missing_required_identity_is_rejected_without_fabrication,
            blank_required_identifiers_are_rejected,
            wrong_typed_required_fields_are_rejected,
            unsupported_event_name_is_rejected,
            optional_fields_are_accepted_when_absent_null_or_string,
            wrong_typed_or_blank_optional_fields_are_rejected,
            malformed_runtime_input_fails_closed_before_dispatch,
        }
    };
    ($adapter:ty => $($contract:ident),+ $(,)?) => {
        $(
            #[test]
            fn $contract() {
                $crate::services::hooks::mutation_scope_ingress_conformance::$contract::<$adapter>();
            }
        )+
    };
}

pub(crate) use mutation_scope_ingress_conformance_tests;
