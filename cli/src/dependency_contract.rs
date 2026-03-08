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
        std::any::type_name::<clap::builder::Command>(), // clap derive feature
        std::any::type_name::<clap_complete::Shell>(),   // clap_complete
        std::any::type_name::<hmac::Hmac<sha2::Sha256>>(),
        std::any::type_name::<inquire::ui::RenderConfig>(),
        std::any::type_name::<opentelemetry::Context>(),
        std::any::type_name::<opentelemetry_otlp::SpanExporter>(),
        std::any::type_name::<opentelemetry_sdk::trace::SdkTracerProvider>(),
        std::any::type_name::<serde_json::Value>(),
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
