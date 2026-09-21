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

#[test]
fn empty_payload_is_rejected() {
    let error = parse_pi_hook_event("   ").unwrap_err().to_string();
    assert_eq!(
        error,
        "Invalid Pi hook event payload from STDIN: expected a JSON object, got an empty payload."
    );
}

#[test]
fn non_object_json_is_rejected() {
    for payload in ["[]", "\"ToolCall\"", "42", "null"] {
        let error = parse_pi_hook_event(payload).unwrap_err().to_string();
        assert!(
            error.contains("expected a JSON object"),
            "payload {payload:?} produced {error:?}"
        );
    }
}

#[test]
fn invalid_json_is_rejected() {
    let error = parse_pi_hook_event("{not json").unwrap_err().to_string();
    assert!(
        error.contains("Invalid Pi hook event payload from STDIN: expected valid JSON"),
        "{error:?}"
    );
}

#[test]
fn unsupported_hook_event_name_is_rejected() {
    for name in ["PreToolUse", "tool_call", "chat.params", ""] {
        let payload = tool_event_json(name, &[]);
        let error = parse_pi_hook_event(&payload).unwrap_err().to_string();
        assert!(
            error.contains("hook_event_name"),
            "name {name:?} produced {error:?}"
        );
    }
}

#[test]
fn missing_required_fields_are_rejected_without_fabricating_identity() {
    for field in [
        SESSION_ID_FIELD,
        TOOL_CALL_ID_FIELD,
        CWD_FIELD,
        TOOL_NAME_FIELD,
    ] {
        let mut object: Map<String, Value> =
            serde_json::from_str(&tool_event_json(HOOK_EVENT_TOOL_CALL, &[])).unwrap();
        object.remove(field);
        let payload = Value::Object(object).to_string();

        let error = parse_pi_hook_event(&payload).unwrap_err().to_string();
        assert!(
            error.contains(&format!("'{field}'")),
            "missing {field} produced {error:?}"
        );
    }
}

#[test]
fn blank_required_fields_are_rejected() {
    for field in [
        SESSION_ID_FIELD,
        TOOL_CALL_ID_FIELD,
        CWD_FIELD,
        TOOL_NAME_FIELD,
    ] {
        let payload = tool_event_json(
            HOOK_EVENT_TOOL_CALL,
            &[(field, Value::String("   ".to_string()))],
        );
        let error = parse_pi_hook_event(&payload).unwrap_err().to_string();
        assert!(
            error.contains(&format!("field '{field}' must be a non-blank string")),
            "blank {field} produced {error:?}"
        );
    }
}

#[test]
fn wrong_typed_fields_are_rejected() {
    let payload = tool_event_json(
        HOOK_EVENT_TOOL_CALL,
        &[(TOOL_CALL_ID_FIELD, Value::Bool(true))],
    );
    let error = parse_pi_hook_event(&payload).unwrap_err().to_string();
    assert!(
        error.contains("field 'tool_call_id' must be a string"),
        "{error:?}"
    );
}

#[test]
fn wrong_typed_optional_model_is_rejected() {
    let payload = tool_event_json(HOOK_EVENT_TOOL_CALL, &[(MODEL_FIELD, Value::Bool(false))]);
    let error = parse_pi_hook_event(&payload).unwrap_err().to_string();
    assert!(
        error.contains("field 'model' must be null, absent, or a non-blank string"),
        "{error:?}"
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
fn run_from_payload_fails_closed_when_a_tracked_start_cannot_resolve_its_checkout() {
    let payload = tool_event_json(
        HOOK_EVENT_TOOL_CALL,
        &[(
            CWD_FIELD,
            Value::String("/nonexistent/sce/pi/checkout".to_string()),
        )],
    );
    let error = run_pi_mutation_scope_from_payload(&payload, None)
        .expect_err("a tracked Start that cannot resolve its checkout must fail closed");
    assert!(error.to_string().contains(FAIL_CLOSED_MESSAGE), "{error:?}");
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
fn run_from_payload_surfaces_malformed_input() {
    let error = run_pi_mutation_scope_from_payload("{bad", None)
        .unwrap_err()
        .to_string();
    assert!(error.contains("expected valid JSON"), "{error:?}");
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
