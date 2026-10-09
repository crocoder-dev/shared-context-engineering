use std::collections::HashMap;
use std::{fs, path::Path};

use crate::services::observability::otel_policy::OtelName;
use proc_macro2::{TokenStream, TokenTree};
use syn::visit::{self, Visit};
use syn::{
    Attribute, Expr, FnArg, ImplItemFn, ItemExternCrate, ItemFn, ItemMod, ItemUse, Macro, Meta,
    Pat, Stmt, TraitItemFn, Type, UseTree,
};

const TRUSTED_RELATIVE_PATH: &str = "services/observability/tracing_boundary.rs";
const TRACING_ROOT: &str = "tracing";
const EVENT_MACROS: [&str; 6] = ["error", "warn", "info", "debug", "trace", "event"];
const BOUNDARY_EVENT_MACROS: [&str; 4] = ["error", "warn", "info", "debug"];
const BOUNDARY_ALLOWED_USES: [&str; 4] = [
    "tracing::Level",
    "tracing::Dispatch",
    "tracing::instrument::Instrument",
    "tracing::instrument::WithSubscriber",
];
const TARGET_CONSTANT: &str = "SCE_TRACING_TARGET";
const SPAN_TARGET_TOKENS: &str = "target:OTEL_TARGET";
const SPAN_LEVEL_TOKENS: &str = "Level::INFO";
const SPAN_EMPTY_VALUE_TOKENS: &str = "tracing::field::Empty";
const SPAN_FIELDS: [&str; 4] = [
    "sce.command.name",
    "sce.outcome",
    "sce.error.category",
    "otel.status_code",
];
const APPROVED_CONSTANTS: [&str; 1] = ["CONTENTION_EXHAUSTED_CAUSE"];
const NUMERIC_TYPES: [&str; 2] = ["u32", "u64"];
const CLOSED_TYPES: [&str; 4] = ["LogLevel", "EventId", "OperationClass", "DbName"];
const ALLOWED_FIELDS: [&str; 13] = [
    "event_id",
    "log_level",
    "operation",
    "attempt",
    "max_attempts",
    "timeout_ms",
    "backoff_ms",
    "db_name",
    "attempts",
    "busy_timeout_ms",
    "contention_deadline_ms",
    "elapsed_ms",
    "cause",
];
const REQUIRED_EMITTERS: [&str; 3] = [
    "emit_logger_event",
    "emit_retry_event",
    "emit_contention_event",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Zone {
    Trusted,
    Untrusted,
}

#[derive(Debug, Default)]
struct Report {
    violations: Vec<String>,
    emitting_fns: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Binding {
    Numeric,
    ClosedValue,
    ClosedString,
    Unsafe,
}

impl Binding {
    fn is_exportable(self) -> bool {
        matches!(self, Self::Numeric | Self::ClosedString)
    }
}

struct FnScope {
    name: String,
    bindings: HashMap<String, Binding>,
}

struct Audit {
    zone: Zone,
    report: Report,
    scopes: Vec<FnScope>,
}

fn audit_source(source: &str, zone: Zone) -> Result<Report, syn::Error> {
    let file = syn::parse_file(source)?;
    let mut audit = Audit {
        zone,
        report: Report::default(),
        scopes: Vec::new(),
    };
    audit.visit_file(&file);
    audit.report.emitting_fns.sort();
    audit.report.emitting_fns.dedup();
    Ok(audit.report)
}

fn is_cfg_test(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| match &attr.meta {
        Meta::List(list) => list.path.is_ident("cfg") && list.tokens.to_string() == "test",
        _ => false,
    })
}

fn type_name(ty: &Type) -> Option<String> {
    match ty {
        Type::Path(path) => path
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string()),
        _ => None,
    }
}

fn single_ident(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Path(path) if path.qself.is_none() => path.path.get_ident().map(ToString::to_string),
        _ => None,
    }
}

fn classify_param(ty: &Type) -> Binding {
    match type_name(ty) {
        Some(name) if NUMERIC_TYPES.contains(&name.as_str()) => Binding::Numeric,
        Some(name) if CLOSED_TYPES.contains(&name.as_str()) => Binding::ClosedValue,
        _ => Binding::Unsafe,
    }
}

fn classify_initializer(expr: &Expr, bindings: &HashMap<String, Binding>) -> Binding {
    match expr {
        Expr::MethodCall(call) if call.method == "as_str" && call.args.is_empty() => {
            let closed_receiver = match &*call.receiver {
                Expr::Path(path) if path.qself.is_none() => {
                    let segments: Vec<_> = path.path.segments.iter().collect();
                    match segments.as_slice() {
                        [only] => {
                            bindings.get(&only.ident.to_string()) == Some(&Binding::ClosedValue)
                        }
                        [owner, _] => CLOSED_TYPES.contains(&owner.ident.to_string().as_str()),
                        _ => false,
                    }
                }
                _ => false,
            };
            if closed_receiver {
                Binding::ClosedString
            } else {
                Binding::Unsafe
            }
        }
        other => match single_ident(other) {
            Some(name) if APPROVED_CONSTANTS.contains(&name.as_str()) => Binding::ClosedString,
            _ => Binding::Unsafe,
        },
    }
}

fn build_scope(name: &str, inputs: Vec<&FnArg>, block: Option<&syn::Block>) -> FnScope {
    let mut bindings = HashMap::new();
    for input in inputs {
        if let FnArg::Typed(typed) = input {
            if let Pat::Ident(ident) = &*typed.pat {
                bindings.insert(ident.ident.to_string(), classify_param(&typed.ty));
            }
        }
    }
    if let Some(block) = block {
        for statement in &block.stmts {
            if let Stmt::Local(local) = statement {
                if let (Pat::Ident(ident), Some(init)) = (&local.pat, &local.init) {
                    let binding = classify_initializer(&init.expr, &bindings);
                    bindings.insert(ident.ident.to_string(), binding);
                }
            }
        }
    }
    FnScope {
        name: name.to_string(),
        bindings,
    }
}

fn flatten_use(tree: &UseTree, prefix: &str, out: &mut Vec<String>) {
    match tree {
        UseTree::Path(path) => {
            let next = format!("{prefix}{}::", path.ident);
            flatten_use(&path.tree, &next, out);
        }
        UseTree::Name(name) => out.push(format!("{prefix}{}", name.ident)),
        UseTree::Rename(rename) => {
            out.push(format!("{prefix}{} as {}", rename.ident, rename.rename));
        }
        UseTree::Glob(_) => out.push(format!("{prefix}*")),
        UseTree::Group(group) => {
            for item in &group.items {
                flatten_use(item, prefix, out);
            }
        }
    }
}

fn mentions_tracing_path(tokens: &TokenStream) -> bool {
    let trees: Vec<TokenTree> = tokens.clone().into_iter().collect();
    trees.iter().enumerate().any(|(index, tree)| match tree {
        TokenTree::Ident(ident) if *ident == TRACING_ROOT => matches!(
            trees.get(index + 1),
            Some(TokenTree::Punct(punct)) if punct.as_char() == ':'
        ),
        TokenTree::Group(group) => mentions_tracing_path(&group.stream()),
        _ => false,
    })
}

fn split_arguments(tokens: &TokenStream) -> Vec<Vec<TokenTree>> {
    let mut segments = vec![Vec::new()];
    for tree in tokens.clone() {
        match &tree {
            TokenTree::Punct(punct) if punct.as_char() == ',' => segments.push(Vec::new()),
            _ => segments
                .last_mut()
                .expect("segments is never empty")
                .push(tree),
        }
    }
    if segments.last().is_some_and(Vec::is_empty) {
        segments.pop();
    }
    segments
}

impl Audit {
    fn context(&self) -> String {
        self.scopes
            .last()
            .map_or_else(|| "<module>".to_string(), |scope| scope.name.clone())
    }

    fn violation(&mut self, message: &str) {
        let entry = format!("{}: {message}", self.context());
        self.report.violations.push(entry);
    }

    fn with_scope(&mut self, scope: FnScope, walk: impl FnOnce(&mut Self)) {
        self.scopes.push(scope);
        walk(self);
        self.scopes.pop();
    }

    fn check_use_leaf(&mut self, leaf: &str) {
        let rooted = leaf == TRACING_ROOT
            || leaf.starts_with("tracing::")
            || leaf.starts_with("tracing as ");
        if !rooted {
            return;
        }
        let allowed = self.zone == Zone::Trusted && BOUNDARY_ALLOWED_USES.contains(&leaf);
        if !allowed {
            self.violation(&format!("import `{leaf}` is not permitted here"));
        }
    }

    fn audit_boundary_macro(&mut self, segments: &[String], last: &str, tokens: &TokenStream) {
        let rooted = segments.first().is_some_and(|first| first == TRACING_ROOT);
        if !rooted && !EVENT_MACROS.contains(&last) {
            if mentions_tracing_path(tokens) {
                self.violation(&format!("macro `{last}!` smuggles a `tracing` path"));
            }
            return;
        }
        if !(rooted && segments.len() == 2) {
            self.violation(&format!(
                "`{}!` must be invoked as `tracing::{last}!`",
                segments.join("::")
            ));
            return;
        }
        match last {
            "enabled" => {}
            "span" => self.check_boundary_span(tokens),
            name if BOUNDARY_EVENT_MACROS.contains(&name) => {
                self.check_boundary_event(name, tokens);
            }
            _ => self.violation(&format!("`tracing::{last}!` is not permitted")),
        }
    }

    fn check_boundary_span(&mut self, tokens: &TokenStream) {
        if self.scopes.last().is_none() {
            self.violation("`span!` outside a function");
            return;
        }
        let segments = split_arguments(tokens);
        let render =
            |trees: &[TokenTree]| trees.iter().map(ToString::to_string).collect::<String>();
        let name_is_enumerated = segments.get(2).is_some_and(|name| {
            syn::parse2::<syn::LitStr>(name.iter().cloned().collect())
                .is_ok_and(|literal| OtelName::parse(&literal.value()).is_some())
        });
        let head_is_typed = segments.len() >= 3
            && render(&segments[0]) == SPAN_TARGET_TOKENS
            && render(&segments[1]) == SPAN_LEVEL_TOKENS
            && name_is_enumerated;
        if !head_is_typed {
            self.violation("`tracing::span!` must use the `OTEL_TARGET`, `Level::INFO` and an enumerated span name");
            return;
        }
        for field in &segments[3..] {
            let rendered = render(field);
            let allowed = rendered
                .strip_suffix(SPAN_EMPTY_VALUE_TOKENS)
                .and_then(|key| key.strip_suffix('='))
                .map(|key| key.trim_matches('"'))
                .is_some_and(|key| SPAN_FIELDS.contains(&key));
            if !allowed {
                self.violation(&format!("`tracing::span!` field `{rendered}` must be an allowlisted key recorded as `Empty`"));
            }
        }
    }

    fn check_boundary_event(&mut self, macro_name: &str, tokens: &TokenStream) {
        if let Some(scope) = self.scopes.last() {
            let name = scope.name.clone();
            self.report.emitting_fns.push(name);
        } else {
            self.violation(&format!("`{macro_name}!` outside a function"));
            return;
        }

        let segments = split_arguments(tokens);
        let Some((message, fields)) = segments.split_last() else {
            self.violation(&format!("`{macro_name}!` has no message"));
            return;
        };

        let static_message = match message.as_slice() {
            [tree @ TokenTree::Literal(_)] => {
                syn::parse2::<syn::LitStr>(TokenStream::from(tree.clone())).is_ok_and(|literal| {
                    let value = literal.value();
                    !value.contains('{') && !value.contains('}')
                })
            }
            _ => false,
        };
        if !static_message {
            self.violation(&format!(
                "`{macro_name}!` message must be one static string literal without interpolation"
            ));
        }

        for field in fields {
            self.check_boundary_field(macro_name, field);
        }
    }

    fn check_boundary_field(&mut self, macro_name: &str, field: &[TokenTree]) {
        match field {
            [TokenTree::Ident(key), TokenTree::Punct(colon), TokenTree::Ident(value)]
                if *key == "target" && colon.as_char() == ':' =>
            {
                if *value != TARGET_CONSTANT {
                    self.violation(&format!(
                        "`{macro_name}!` target must be `{TARGET_CONSTANT}`"
                    ));
                }
            }
            [TokenTree::Ident(name)] => {
                let name = name.to_string();
                self.check_field_value(macro_name, &name, &name);
            }
            [TokenTree::Ident(name), TokenTree::Punct(equals), TokenTree::Ident(value)]
                if equals.as_char() == '=' =>
            {
                self.check_field_value(macro_name, &name.to_string(), &value.to_string());
            }
            _ => self.violation(&format!(
                "`{macro_name}!` field must be a plain `name` or `name = binding`"
            )),
        }
    }

    fn check_field_value(&mut self, macro_name: &str, name: &str, value: &str) {
        if !ALLOWED_FIELDS.contains(&name) {
            self.violation(&format!("`{macro_name}!` field `{name}` is not classified"));
            return;
        }
        if APPROVED_CONSTANTS.contains(&value) {
            return;
        }
        let binding = if value == name {
            self.scopes
                .last()
                .and_then(|scope| scope.bindings.get(value))
                .copied()
        } else {
            None
        };
        if !binding.is_some_and(Binding::is_exportable) {
            self.violation(&format!(
                "`{macro_name}!` field `{name}` value `{value}` is not a closed-enum string or numeric parameter bound to the same name"
            ));
        }
    }
}

impl<'ast> Visit<'ast> for Audit {
    fn visit_item_mod(&mut self, node: &'ast ItemMod) {
        if self.zone == Zone::Trusted && is_cfg_test(&node.attrs) {
            return;
        }
        visit::visit_item_mod(self, node);
    }

    fn visit_item_fn(&mut self, node: &'ast ItemFn) {
        if self.zone == Zone::Trusted && is_cfg_test(&node.attrs) {
            return;
        }
        let scope = build_scope(
            &node.sig.ident.to_string(),
            node.sig.inputs.iter().collect(),
            Some(&node.block),
        );
        self.with_scope(scope, |audit| visit::visit_item_fn(audit, node));
    }

    fn visit_impl_item_fn(&mut self, node: &'ast ImplItemFn) {
        let scope = build_scope(
            &node.sig.ident.to_string(),
            node.sig.inputs.iter().collect(),
            Some(&node.block),
        );
        self.with_scope(scope, |audit| visit::visit_impl_item_fn(audit, node));
    }

    fn visit_trait_item_fn(&mut self, node: &'ast TraitItemFn) {
        let scope = build_scope(
            &node.sig.ident.to_string(),
            node.sig.inputs.iter().collect(),
            node.default.as_ref(),
        );
        self.with_scope(scope, |audit| visit::visit_trait_item_fn(audit, node));
    }

    fn visit_item_use(&mut self, node: &'ast ItemUse) {
        if self.zone == Zone::Trusted && is_cfg_test(&node.attrs) {
            return;
        }
        let mut leaves = Vec::new();
        flatten_use(&node.tree, "", &mut leaves);
        for leaf in leaves {
            self.check_use_leaf(&leaf);
        }
    }

    fn visit_item_extern_crate(&mut self, node: &'ast ItemExternCrate) {
        if node.ident == TRACING_ROOT {
            self.violation("`extern crate tracing` is not permitted");
        }
    }

    fn visit_path(&mut self, node: &'ast syn::Path) {
        if self.zone == Zone::Untrusted
            && node
                .segments
                .first()
                .is_some_and(|segment| segment.ident == TRACING_ROOT)
        {
            self.violation("direct `tracing` path outside the typed boundary");
        }
        visit::visit_path(self, node);
    }

    fn visit_macro(&mut self, node: &'ast Macro) {
        let segments: Vec<String> = node
            .path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect();
        let Some(last) = segments.last().cloned() else {
            return;
        };
        let rooted = segments.first().is_some_and(|first| first == TRACING_ROOT);
        let event_named = EVENT_MACROS.contains(&last.as_str());

        match self.zone {
            Zone::Untrusted => {
                if rooted || event_named {
                    self.violation(&format!(
                        "`{}!` emits or reaches tracing outside the typed boundary",
                        segments.join("::")
                    ));
                } else if mentions_tracing_path(&node.tokens) {
                    self.violation(&format!(
                        "macro `{last}!` references `tracing` outside the typed boundary"
                    ));
                }
            }
            Zone::Trusted => self.audit_boundary_macro(&segments, &last, &node.tokens),
        }
    }
}

fn relative_source_files(src: &Path) -> Vec<(String, std::path::PathBuf)> {
    fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in fs::read_dir(dir).expect("read src dir") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
    }

    let mut files = Vec::new();
    walk(src, &mut files);
    files.sort();
    files
        .into_iter()
        .map(|path| {
            let relative = path
                .strip_prefix(src)
                .expect("relative path")
                .to_string_lossy()
                .replace('\\', "/");
            (relative, path)
        })
        .collect()
}

fn violations(source: &str, zone: Zone) -> Vec<String> {
    audit_source(source, zone)
        .expect("fixture parses")
        .violations
}

#[test]
fn tracing_boundary_a_every_tracing_macro_site_records_only_classified_fields() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let files = relative_source_files(&src);
    assert!(
        files
            .iter()
            .any(|(relative, _)| relative == TRUSTED_RELATIVE_PATH),
        "trusted boundary file must exist"
    );

    let mut failures = Vec::new();
    let mut trusted_report = None;
    for (relative, path) in &files {
        let source = fs::read_to_string(path).expect("read source");
        let zone = if relative == TRUSTED_RELATIVE_PATH {
            Zone::Trusted
        } else {
            Zone::Untrusted
        };
        let report = audit_source(&source, zone)
            .unwrap_or_else(|error| panic!("{relative} must parse: {error}"));
        for violation in &report.violations {
            failures.push(format!("{relative}: {violation}"));
        }
        if zone == Zone::Trusted {
            trusted_report = Some(report);
        }
    }

    assert!(
        failures.is_empty(),
        "tracing boundary bypasses:\n{}",
        failures.join("\n")
    );
    let trusted = trusted_report.expect("trusted boundary audited");
    for emitter in REQUIRED_EMITTERS {
        assert!(
            trusted.emitting_fns.iter().any(|name| name == emitter),
            "audit must observe emitter {emitter}"
        );
    }
}

#[test]
fn tracing_boundary_a_audit_rejects_interpolated_message_capture() {
    let source = r#"
        fn leak(secret: &str) {
            tracing::warn!("token={secret}");
        }
    "#;
    assert!(!violations(source, Zone::Untrusted).is_empty());
    let trusted = violations(source, Zone::Trusted);
    assert!(
        trusted.iter().any(|v| v.contains("static string literal")),
        "{trusted:?}"
    );
}

#[test]
fn tracing_boundary_a_audit_rejects_untrusted_field_value() {
    let source = r#"
        fn leak(untrusted_value: &str) {
            tracing::warn!(
                operation = untrusted_value,
                "retry"
            );
        }
    "#;
    assert!(!violations(source, Zone::Untrusted).is_empty());
    let trusted = violations(source, Zone::Trusted);
    assert!(
        trusted.iter().any(|v| v.contains("untrusted_value")),
        "{trusted:?}"
    );
}

#[test]
fn tracing_boundary_a_audit_rejects_same_named_binding_of_unclassified_type() {
    let source = r#"
        fn leak(operation: &str) {
            tracing::warn!(operation, "retry");
        }
        fn leak_string(operation: &str) {
            let operation = operation.to_string();
            tracing::warn!(operation, "retry");
        }
        fn leak_as_str(raw: &str) {
            let operation = raw.as_str();
            tracing::warn!(operation, "retry");
        }
    "#;
    assert_eq!(violations(source, Zone::Trusted).len(), 3);
}

#[test]
fn tracing_boundary_a_audit_rejects_display_capture_and_unclassified_field_names() {
    let source = r#"
        fn leak(secret: &str) {
            tracing::error!(
                password = %secret,
                "authentication failed"
            );
            tracing::error!(secret = ?secret, "authentication failed");
            tracing::error!(error = secret, "authentication failed");
        }
    "#;
    assert!(!violations(source, Zone::Untrusted).is_empty());
    assert_eq!(violations(source, Zone::Trusted).len(), 3);
}

#[test]
fn tracing_boundary_a_audit_rejects_imported_macro_aliases() {
    let unqualified = r#"
        use tracing::warn;

        fn leak(secret: &str) {
            warn!("secret={secret}");
        }
    "#;
    let untrusted = violations(unqualified, Zone::Untrusted);
    assert!(
        untrusted.iter().any(|v| v.contains("import")),
        "{untrusted:?}"
    );
    assert!(
        untrusted.iter().any(|v| v.contains("warn!")),
        "{untrusted:?}"
    );
    let trusted = violations(unqualified, Zone::Trusted);
    assert!(trusted.iter().any(|v| v.contains("import")), "{trusted:?}");
    assert!(
        trusted.iter().any(|v| v.contains("must be invoked")),
        "{trusted:?}"
    );

    for source in [
        "use tracing::warn as w; fn f() { w!(\"x\"); }",
        "use tracing::*; fn f() { warn!(\"x\"); }",
        "use tracing::{warn, info}; fn f() { info!(\"x\"); }",
        "use ::tracing::warn; fn f() { warn!(\"x\"); }",
        "use tracing as t; fn f() { t::warn!(\"x\"); }",
        "use tracing; fn f() { tracing::warn!(\"x\"); }",
        "extern crate tracing as t; fn f() { t::warn!(\"x\"); }",
        "fn f() { ::tracing::warn!(\"x\"); }",
        "fn f() { tracing::event!(tracing::Level::WARN, \"x\"); }",
        "fn f() { tracing::trace!(\"x\"); }",
        "fn f() { let _ = tracing::Level::WARN; }",
        "#[tracing::instrument] fn f() {}",
        "macro_rules! emit { () => { tracing::warn!(\"x\") }; }",
        "fn f() { wrap!(tracing::warn!(\"x\")); }",
    ] {
        assert!(
            !violations(source, Zone::Untrusted).is_empty(),
            "untrusted zone must reject {source}"
        );
    }

    for source in [
        "use tracing::warn as w; fn f() { w!(\"x\"); }",
        "use tracing::*;",
        "use tracing::{Level, warn};",
        "fn f() { tracing::event!(tracing::Level::WARN, \"x\"); }",
        "fn f() { tracing::trace!(\"x\"); }",
        "extern crate tracing as t;",
        "macro_rules! emit { () => { tracing::warn!(\"x\") }; }",
        "fn f() { wrap!(tracing::warn!(\"x\")); }",
    ] {
        assert!(
            !violations(source, Zone::Trusted).is_empty(),
            "trusted zone must reject {source}"
        );
    }
}

#[test]
fn tracing_boundary_a_audit_ignores_comments_and_string_literals() {
    let source = r##"
        //! tracing::warn!("doc comment {secret}");
        /// tracing::warn!("doc {secret}");
        fn quiet() {
            // tracing::warn!("line comment {secret}");
            /* tracing::warn!("block comment {secret}"); */
            let plain = "tracing::warn!(\"token={secret}\")";
            let raw = r#"tracing::warn!("token={secret}"); use tracing::warn;"#;
            let _ = (plain, raw);
        }
    "##;
    assert!(violations(source, Zone::Untrusted).is_empty());
    assert!(violations(source, Zone::Trusted).is_empty());
}

#[test]
fn tracing_boundary_a_audit_accepts_conforming_boundary_emitter() {
    let source = r#"
        use tracing::Level;

        pub fn emit_example(operation: OperationClass, attempt: u32, level: LogLevel) {
            let operation = operation.as_str();
            let log_level = level.as_str();
            let cause = CONTENTION_EXHAUSTED_CAUSE;
            tracing::warn!(
                target: SCE_TRACING_TARGET,
                operation,
                attempt,
                log_level,
                cause,
                "Static message"
            );
            let _ = tracing::enabled!(target: SCE_TRACING_TARGET, Level::WARN);
        }

        #[cfg(test)]
        mod tests {
            fn capture() {
                tracing::subscriber::with_default(1, || tracing::warn!("{x}"));
            }
        }
    "#;
    let report = audit_source(source, Zone::Trusted).expect("parses");
    assert!(report.violations.is_empty(), "{:?}", report.violations);
    assert_eq!(report.emitting_fns, vec!["emit_example".to_string()]);
}

#[test]
fn tracing_boundary_a_audit_rejects_non_constant_target_and_missing_static_message() {
    let source = r#"
        pub fn emit(operation: OperationClass) {
            let operation = operation.as_str();
            tracing::warn!(target: "other", operation, "message");
            tracing::warn!(target: SCE_TRACING_TARGET, operation, concat!("a", "b"));
            tracing::warn!(target: SCE_TRACING_TARGET, operation, "a {}", operation);
            tracing::warn!(target: SCE_TRACING_TARGET, operation);
        }
        const FN_FREE: () = { tracing::warn!("x"); };
    "#;
    assert!(violations(source, Zone::Trusted).len() >= 5);
}

#[test]
fn tracing_boundary_a_audit_accepts_only_typed_otel_span_in_the_boundary() {
    let conforming = r#"
        pub fn start() {
            let _ = tracing::span!(
                target: OTEL_TARGET,
                Level::INFO,
                "sce.command",
                "sce.outcome" = tracing::field::Empty,
                "otel.status_code" = tracing::field::Empty
            );
        }
    "#;
    assert!(violations(conforming, Zone::Trusted).is_empty());
    assert!(!violations(conforming, Zone::Untrusted).is_empty());

    for rejected in [
        r#"pub fn s(v: &str) { let _ = tracing::span!(target: OTEL_TARGET, Level::INFO, "sce.command", "sce.outcome" = v); }"#,
        r#"pub fn s() { let _ = tracing::span!(target: "sce", Level::INFO, "sce.command"); }"#,
        r#"pub fn s() { let _ = tracing::span!(target: OTEL_TARGET, Level::INFO, "sce.arbitrary"); }"#,
        r#"pub fn s() { let _ = tracing::span!(target: OTEL_TARGET, Level::INFO, "sce.command", "password" = tracing::field::Empty); }"#,
        r#"pub fn s() { let _ = tracing::span!(target: OTEL_TARGET, Level::DEBUG, "sce.command"); }"#,
    ] {
        assert!(
            !violations(rejected, Zone::Trusted).is_empty(),
            "{rejected}"
        );
    }
}
