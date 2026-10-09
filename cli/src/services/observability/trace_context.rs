use std::collections::HashMap;

use opentelemetry::propagation::TextMapPropagator;
use opentelemetry::trace::TraceContextExt;
use opentelemetry::Context;
use opentelemetry_sdk::propagation::TraceContextPropagator;

pub const TRACEPARENT_ENV: &str = "TRACEPARENT";
pub const TRACESTATE_ENV: &str = "TRACESTATE";

pub fn extract_remote_parent(env: &impl Fn(&str) -> Option<String>) -> Option<Context> {
    let mut carrier = HashMap::new();
    carrier.insert("traceparent".to_string(), env(TRACEPARENT_ENV)?);
    if let Some(state) = env(TRACESTATE_ENV) {
        carrier.insert("tracestate".to_string(), state);
    }
    let context = TraceContextPropagator::new().extract(&carrier);
    let span_context = context.span().span_context().clone();
    (span_context.is_valid() && span_context.is_remote()).then_some(context)
}

#[cfg(test)]
mod tests;
