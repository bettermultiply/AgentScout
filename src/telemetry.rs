//! Optional, best-effort OTLP tracing. Only explicit metadata belongs here.
use opentelemetry::trace::{Status, TraceContextExt, TracerProvider};
use opentelemetry::{Context, KeyValue, propagation::TextMapPropagator};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
    Resource, error::OTelSdkResult, propagation::TraceContextPropagator, trace::*,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering::Relaxed},
    },
    time::Duration,
};
use tracing_opentelemetry::OpenTelemetrySpanExt;
use tracing_subscriber::prelude::*;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Carrier {
    traceparent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tracestate: Option<String>,
}

impl Carrier {
    pub fn context(&self) -> Context {
        if self.traceparent.len() != 55 || self.tracestate.as_ref().is_some_and(|s| s.len() > 512) {
            return Context::new();
        }
        let mut headers = HashMap::from([("traceparent".to_owned(), self.traceparent.clone())]);
        if let Some(state) = &self.tracestate {
            headers.insert("tracestate".into(), state.clone());
        }
        TraceContextPropagator::new().extract(&headers)
    }

    pub fn from_context(cx: &Context) -> Option<Self> {
        let mut headers = HashMap::new();
        TraceContextPropagator::new().inject_context(cx, &mut headers);
        Some(Self {
            traceparent: headers.remove("traceparent")?,
            tracestate: headers.remove("tracestate"),
        })
    }

    pub fn current() -> Option<Self> {
        Self::from_context(&current())
    }

    pub fn inject_env(&self, command: &mut std::process::Command) {
        command.env("TRACEPARENT", &self.traceparent);
        if let Some(state) = &self.tracestate {
            command.env("TRACESTATE", state);
        } else {
            command.env_remove("TRACESTATE");
        }
    }
}

pub fn current() -> Context {
    let cx = tracing::Span::current().context();
    if cx.has_active_span() {
        cx
    } else {
        Context::current()
    }
}

pub fn parent(span: &tracing::Span, cx: Context) {
    if cx.has_active_span() {
        let _ = span.set_parent(cx);
    }
}

pub fn finish<T, E>(span: &tracing::Span, result: &Result<T, E>) {
    if result.is_err() {
        span.set_status(Status::error("operation failed"));
    }
}

#[derive(Debug, Default)]
struct Counts {
    ended: AtomicU64,
    exported: AtomicU64,
    failed: AtomicU64,
}

#[derive(Debug)]
struct Exporter(opentelemetry_otlp::SpanExporter, Arc<Counts>);
impl SpanExporter for Exporter {
    async fn export(&self, batch: Vec<SpanData>) -> OTelSdkResult {
        let count = batch.len() as u64;
        let result = self.0.export(batch).await;
        let counter = if result.is_ok() {
            &self.1.exported
        } else {
            &self.1.failed
        };
        counter.fetch_add(count, Relaxed);
        result
    }
    fn set_resource(&mut self, resource: &Resource) {
        self.0.set_resource(resource);
    }
}

#[derive(Debug)]
struct Processor(BatchSpanProcessor, Arc<Counts>);
impl SpanProcessor for Processor {
    fn on_start(&self, span: &mut Span, cx: &Context) {
        self.0.on_start(span, cx);
    }
    fn on_end(&self, span: SpanData) {
        self.1.ended.fetch_add(1, Relaxed);
        self.0.on_end(span);
    }
    fn force_flush(&self) -> OTelSdkResult {
        self.0.force_flush()
    }
    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.0.shutdown_with_timeout(timeout)
    }
    fn set_resource(&mut self, resource: &Resource) {
        self.0.set_resource(resource);
    }
}

pub struct Guard(SdkTracerProvider, Arc<Counts>, &'static str);

impl Drop for Guard {
    fn drop(&mut self) {
        let timeout = self
            .0
            .shutdown_with_timeout(Duration::from_millis(1500))
            .is_err();
        let ended = self.1.ended.load(Relaxed);
        let exported = self.1.exported.load(Relaxed);
        let failed = self.1.failed.load(Relaxed);
        let dropped_or_pending = ended.saturating_sub(exported + failed);
        if timeout || failed != 0 || dropped_or_pending != 0 {
            eprintln!(
                "{}: OTLP ended={ended} exported={exported} failed={failed} dropped_or_pending={dropped_or_pending} shutdown_failed={timeout}",
                self.2
            );
        }
    }
}

pub fn init(service: &'static str) -> Option<Guard> {
    if std::env::var("OTEL_SDK_DISABLED").is_ok_and(|v| v.eq_ignore_ascii_case("true"))
        || ![
            "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
            "OTEL_EXPORTER_OTLP_ENDPOINT",
        ]
        .iter()
        .any(|key| std::env::var(key).is_ok_and(|v| !v.trim().is_empty()))
    {
        return None;
    }
    let exporter = match opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .with_protocol(opentelemetry_otlp::Protocol::HttpBinary)
        .with_timeout(Duration::from_millis(750))
        .build()
    {
        Ok(exporter) => exporter,
        Err(_) => {
            eprintln!("{service}: OTLP exporter configuration failed");
            return None;
        }
    };
    let counts = Arc::new(Counts::default());
    let batch = BatchSpanProcessor::builder(Exporter(exporter, counts.clone()))
        .with_batch_config(
            BatchConfigBuilder::default()
                .with_max_queue_size(1024)
                .with_max_export_batch_size(128)
                .with_scheduled_delay(Duration::from_millis(300))
                .build(),
        )
        .build();
    let provider = SdkTracerProvider::builder()
        .with_resource(
            Resource::builder_empty()
                .with_attribute(KeyValue::new("service.name", service))
                .build(),
        )
        .with_span_processor(Processor(batch, counts.clone()))
        .build();
    let layer = tracing_opentelemetry::layer()
        .with_tracer(provider.tracer("ascout"))
        .with_threads(false)
        .with_location(false)
        .with_filter(tracing_subscriber::filter::filter_fn(|m| {
            m.target() == "ascout_otel"
        }));
    tracing_subscriber::registry().with(layer).try_init().ok()?;
    Some(Guard(provider, counts, service))
}
