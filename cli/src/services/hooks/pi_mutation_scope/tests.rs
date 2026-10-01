use super::*;

fn tool_event_json(hook_event_name: &str, overrides: &[(&str, Value)]) -> String {
    let mut object = Map::new();
    object.insert(
        HOOK_EVENT_NAME_FIELD.to_string(),
        Value::String(hook_event_name.to_string()),
    );
    object.insert(
        SESSION_ID_FIELD.to_string(),
        Value::String("01a091f4-session".to_string()),
    );
    object.insert(
        TOOL_CALL_ID_FIELD.to_string(),
        Value::String("call_1|fc_1".to_string()),
    );
    object.insert(
        CWD_FIELD.to_string(),
        Value::String("/repo/checkout".to_string()),
    );
    object.insert(
        TOOL_NAME_FIELD.to_string(),
        Value::String("write".to_string()),
    );
    for (field, value) in overrides {
        object.insert((*field).to_string(), value.clone());
    }
    Value::Object(object).to_string()
}

fn key(session_id: &str, tool_call_id: &str) -> AttemptKey {
    AttemptKey {
        session_id: session_id.to_string(),
        tool_call_id: tool_call_id.to_string(),
    }
}

fn tool_call(payload: &str) -> PiToolCall {
    match parse_pi_hook_event(payload).expect("valid ToolCall parses") {
        PiHookEvent::Call(call) => call,
        other => panic!("expected ToolCall, got {other:?}"),
    }
}

mod ingress_conformance {
    use super::*;
    use crate::services::hooks::mutation_scope_ingress_conformance::{
        self as conformance, mutation_scope_ingress_conformance_tests,
        CheckoutResolutionConformance, IngressConformance,
    };

    struct PiIngressConformance;

    impl IngressConformance for PiIngressConformance {
        const ADAPTER: &'static str = "Pi";
        const EVENT_NAME_FIELD: &'static str = HOOK_EVENT_NAME_FIELD;
        const REQUIRED_START_FIELDS: &'static [&'static str] = &[
            SESSION_ID_FIELD,
            TOOL_CALL_ID_FIELD,
            CWD_FIELD,
            TOOL_NAME_FIELD,
        ];
        const OPTIONAL_START_FIELDS: &'static [(&'static str, &'static str)] =
            &[(MODEL_FIELD, "openai-codex/gpt-5.5")];
        const UNSUPPORTED_EVENT_NAMES: &'static [&'static str] =
            &["PreToolUse", "tool_call", "chat.params"];

        fn tracked_start() -> Map<String, Value> {
            serde_json::from_str(&tool_event_json(HOOK_EVENT_TOOL_CALL, &[]))
                .expect("the tracked ToolCall fixture is a JSON object")
        }

        fn parse(payload: &str) -> Result<()> {
            parse_pi_hook_event(payload).map(|_| ())
        }

        fn run(payload: &str) -> Result<String> {
            run_pi_mutation_scope_from_payload(payload, None)
        }

        fn run_with_seams(
            payload: &str,
            resolve_git_dir: conformance::GitDirResolver,
            seam: conformance::IngressSeam,
        ) -> Result<String> {
            run_pi_mutation_scope_from_payload_with_seams(payload, None, resolve_git_dir, seam)
        }
    }

    impl CheckoutResolutionConformance for PiIngressConformance {
        const CWD_FIELD: &'static str = CWD_FIELD;
        const FAIL_CLOSED_MESSAGE: &'static str = FAIL_CLOSED_MESSAGE;
    }

    mutation_scope_ingress_conformance_tests!(PiIngressConformance);
    mutation_scope_ingress_conformance_tests!(
        PiIngressConformance => tracked_start_fails_closed_when_its_checkout_cannot_be_resolved
    );
}

#[test]
fn tool_call_parses_identity_and_model() {
    let call = tool_call(&tool_event_json(
        HOOK_EVENT_TOOL_CALL,
        &[
            (TOOL_NAME_FIELD, Value::String("edit".to_string())),
            (
                MODEL_FIELD,
                Value::String("openai-codex/gpt-5.5".to_string()),
            ),
        ],
    ));
    assert_eq!(call.identity.session_id, "01a091f4-session");
    assert_eq!(call.identity.tool_call_id, "call_1|fc_1");
    assert_eq!(call.identity.tool_name, "edit");
    assert_eq!(call.model.as_deref(), Some("openai-codex/gpt-5.5"));
    assert_eq!(
        call.identity.classification(),
        ToolClassification::TrackedMutation
    );
}

#[test]
fn tool_call_model_is_optional() {
    let call = tool_call(&tool_event_json(HOOK_EVENT_TOOL_CALL, &[]));
    assert_eq!(call.model, None);
}

#[test]
fn tool_result_and_tool_execution_end_parse_minimal_identity() {
    for name in [
        HOOK_EVENT_TOOL_RESULT,
        HOOK_EVENT_TOOL_EXECUTION_END,
        HOOK_EVENT_TOOL_EXECUTION_ABANDON,
    ] {
        let event = parse_pi_hook_event(&tool_event_json(name, &[])).unwrap();
        let identity = match event {
            PiHookEvent::Executed(identity)
            | PiHookEvent::ExecutionEnd(identity)
            | PiHookEvent::ExecutionAbandon(identity) => identity,
            other => panic!("expected a minimal-identity event, got {other:?}"),
        };
        assert_eq!(
            identity.attempt_key(),
            key("01a091f4-session", "call_1|fc_1")
        );
    }
}

#[test]
fn tool_execution_start_parses_and_is_never_evidence() {
    let event =
        parse_pi_hook_event(&tool_event_json(HOOK_EVENT_TOOL_EXECUTION_START, &[])).unwrap();
    let PiHookEvent::ExecutionStart(identity) = event else {
        panic!("expected ToolExecutionStart");
    };
    assert_eq!(identity.tool_call_id, "call_1|fc_1");
}

#[test]
fn classification_table() {
    let cases: &[(&str, ToolClassification)] = &[
        ("bash", ToolClassification::TrackedMutation),
        ("edit", ToolClassification::TrackedMutation),
        ("write", ToolClassification::TrackedMutation),
        ("read", ToolClassification::Untracked),
        ("grep", ToolClassification::Untracked),
        ("find", ToolClassification::Untracked),
        ("ls", ToolClassification::Untracked),
        ("probe_mutate", ToolClassification::Untracked),
        ("Bash", ToolClassification::Untracked),
        ("some_future_pi_builtin", ToolClassification::Untracked),
        ("", ToolClassification::Untracked),
    ];
    for (tool_name, expected) in cases {
        assert_eq!(
            classify_tool(tool_name),
            *expected,
            "classify_tool({tool_name:?})"
        );
    }
}

#[test]
fn scope_id_embeds_attempt_seq_and_is_length_prefixed() {
    let k = key("01a091f4-session", "call_1|fc_1");
    let scope_id = format_pi_scope_id(&k, 1);
    assert_eq!(
        scope_id,
        "pi-tool-v1|n=1|s=16:01a091f4-session|c=11:call_1|fc_1"
    );
    assert_ne!(format_pi_scope_id(&k, 1), format_pi_scope_id(&k, 2));
    assert_eq!(
        pi_scope_start_event_id(&scope_id),
        format!("{scope_id}|start")
    );
    assert_eq!(
        pi_scope_close_event_id(&scope_id),
        format!("{scope_id}|close")
    );
    assert_ne!(
        pi_scope_start_event_id(&scope_id),
        pi_scope_close_event_id(&scope_id)
    );
}

#[test]
fn length_prefix_disambiguates_delimiter_collisions() {
    let a = key("s|c=1:x", "y");
    let b = key("s", "1:x|y");
    assert_ne!(format_pi_scope_id(&a, 1), format_pi_scope_id(&b, 1));
}

#[test]
fn provenance_canonicalizes_the_session_and_normalizes_the_model() {
    let provenance = pi_scope_provenance("01a091f4-session", Some("openai-codex/gpt-5.5"));
    assert_eq!(provenance.session_id, "pi_01a091f4-session");
    assert_eq!(provenance.model_id.as_deref(), Some("openai-codex/gpt-5.5"));
}

#[test]
fn provenance_keeps_an_already_prefixed_session_id() {
    let provenance = pi_scope_provenance("pi_01a091f4-session", None);
    assert_eq!(provenance.session_id, "pi_01a091f4-session");
}

#[test]
fn provenance_without_model_evidence_is_null() {
    for model in [None, Some(""), Some("   ")] {
        let provenance = pi_scope_provenance("01a091f4-session", model);
        assert_eq!(provenance.model_id, None, "model {model:?}");
    }
}

#[test]
fn run_from_payload_is_neutral_for_untracked_events() {
    for tool_name in ["read", "grep", "find", "ls", "probe_mutate"] {
        let payload = tool_event_json(
            HOOK_EVENT_TOOL_CALL,
            &[(TOOL_NAME_FIELD, Value::String(tool_name.to_string()))],
        );
        assert_eq!(
            run_pi_mutation_scope_from_payload(&payload, None).unwrap(),
            String::new()
        );
    }
}

#[test]
fn tool_execution_start_is_always_a_no_op_regardless_of_classification() {
    for tool_name in ["bash", "read"] {
        let payload = tool_event_json(
            HOOK_EVENT_TOOL_EXECUTION_START,
            &[(TOOL_NAME_FIELD, Value::String(tool_name.to_string()))],
        );
        assert_eq!(
            run_pi_mutation_scope_from_payload(&payload, None).unwrap(),
            String::new()
        );
    }
}
