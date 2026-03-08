pub fn dependency_contract_snapshot() -> (
    anyhow::Result<()>,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
) {
    (
        Ok(()),
        // Note: dirs and reqwest crates are verified via Cargo.toml; dirs has no public types
        // Note: serde is verified via serde_json dependency (serde_json depends on serde)
        std::any::type_name::<hmac::Hmac<sha2::Sha256>>(),
        std::any::type_name::<inquire::ui::RenderConfig>(),
        std::any::type_name::<lexopt::Parser>(),
        std::any::type_name::<opentelemetry::Context>(),
        std::any::type_name::<opentelemetry_otlp::SpanExporter>(),
        std::any::type_name::<opentelemetry_sdk::trace::SdkTracerProvider>(),
        std::any::type_name::<reqwest::Client>(),
        std::any::type_name::<serde_json::Value>(), // serde verified via serde_json dependency
        std::any::type_name::<sha2::Sha256>(),
        std::any::type_name::<tokio::runtime::Runtime>(),
        std::any::type_name::<tracing::Level>(),
        std::any::type_name::<
            tracing_opentelemetry::OpenTelemetryLayer<
                tracing_subscriber::Registry,
                opentelemetry_sdk::trace::Tracer,
            >,
        >(),
        std::any::type_name::<tracing_subscriber::Registry>(),
        std::any::type_name::<turso::Builder>(),
    )
}

#[cfg(test)]
mod tests {
    use super::dependency_contract_snapshot;

    #[test]
    fn dependency_contract_snapshot_references_agreed_crates() {
        let (
            result,
            hmac_ty,
            inquire_ty,
            lexopt_ty,
            opentelemetry_ty,
            opentelemetry_otlp_ty,
            opentelemetry_sdk_ty,
            reqwest_ty,
            serde_json_ty,
            sha2_ty,
            tokio_ty,
            tracing_ty,
            tracing_opentelemetry_ty,
            tracing_subscriber_ty,
            turso_ty,
        ) = dependency_contract_snapshot();
        assert!(result.is_ok());
        assert!(hmac_ty.contains("hmac::"));
        assert!(inquire_ty.contains("inquire::"));
        assert!(lexopt_ty.contains("lexopt::"));
        assert!(opentelemetry_ty.contains("opentelemetry::"));
        assert!(opentelemetry_otlp_ty.contains("opentelemetry_otlp::"));
        assert!(opentelemetry_sdk_ty.contains("opentelemetry_sdk::"));
        assert!(reqwest_ty.contains("reqwest::"));
        assert!(serde_json_ty.contains("serde_json::"));
        assert!(sha2_ty.contains("sha2::"));
        assert!(tokio_ty.contains("tokio::"));
        assert!(tracing_ty.contains("tracing"));
        assert!(tracing_opentelemetry_ty.contains("tracing_opentelemetry::"));
        assert!(tracing_subscriber_ty.contains("tracing_subscriber::"));
        assert!(turso_ty.contains("turso::"));
    }
}
