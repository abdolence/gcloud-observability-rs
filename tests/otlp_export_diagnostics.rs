#![cfg(feature = "otlp")]

use gcloud_sdk::tonic::transport::server::TcpIncoming;
use gcloud_sdk::tonic::transport::Server;
use gcloud_sdk::tonic::{async_trait, Request, Response, Status};
use opentelemetry::trace::{
    SpanContext, SpanId, SpanKind, Status as SpanStatus, TraceFlags, TraceId, TraceState,
};
use opentelemetry::InstrumentationScope;
use opentelemetry_otlp::{SpanExporter, WithExportConfig};
use opentelemetry_proto::tonic::collector::trace::v1::trace_service_server::{
    TraceService, TraceServiceServer,
};
use opentelemetry_proto::tonic::collector::trace::v1::{
    ExportTracePartialSuccess, ExportTraceServiceRequest, ExportTraceServiceResponse,
};
use opentelemetry_sdk::trace::{SpanData, SpanEvents, SpanExporter as _, SpanLinks};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::Layer;

const REJECTION_REASON: &str = "span names must not be empty";

/// Accepts every export and reports one span of it as rejected.
struct RejectingCollector;

#[async_trait]
impl TraceService for RejectingCollector {
    async fn export(
        &self,
        _request: Request<ExportTraceServiceRequest>,
    ) -> Result<Response<ExportTraceServiceResponse>, Status> {
        Ok(Response::new(ExportTraceServiceResponse {
            partial_success: Some(ExportTracePartialSuccess {
                rejected_spans: 1,
                error_message: REJECTION_REASON.to_string(),
            }),
        }))
    }
}

struct CapturedEvent {
    name: &'static str,
    level: Level,
    target: String,
    fields: String,
}

#[derive(Clone, Default)]
struct CapturingLayer(Arc<Mutex<Vec<CapturedEvent>>>);

struct FieldsText<'a>(&'a mut String);

impl Visit for FieldsText<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0.push_str(&format!("{}={value:?} ", field.name()));
    }
}

impl<S: Subscriber> Layer<S> for CapturingLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut fields = String::new();
        event.record(&mut FieldsText(&mut fields));
        self.0.lock().unwrap().push(CapturedEvent {
            name: event.metadata().name(),
            level: *event.metadata().level(),
            target: event.metadata().target().to_string(),
            fields,
        });
    }
}

fn finished_span() -> SpanData {
    SpanData {
        span_context: SpanContext::new(
            TraceId::from(1),
            SpanId::from(2),
            TraceFlags::SAMPLED,
            false,
            TraceState::default(),
        ),
        parent_span_id: SpanId::INVALID,
        parent_span_is_remote: false,
        span_kind: SpanKind::Internal,
        name: "work".into(),
        start_time: SystemTime::now(),
        end_time: SystemTime::now(),
        attributes: Vec::new(),
        dropped_attributes_count: 0,
        events: SpanEvents::default(),
        links: SpanLinks::default(),
        status: SpanStatus::Unset,
        instrumentation_scope: InstrumentationScope::builder("test").build(),
    }
}

// The exporter is built here, over plain HTTP/2 to a local collector, because the crate's own
// builder accepts `https` endpoints only. What is under test is that the `opentelemetry-otlp`
// build the crate selects keeps the exporter's own warnings.
#[tokio::test]
async fn partially_rejected_export_is_logged_as_a_warning() {
    let incoming = TcpIncoming::bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let endpoint = format!("http://{}", incoming.local_addr().unwrap());
    tokio::spawn(
        Server::builder()
            .add_service(TraceServiceServer::new(RejectingCollector))
            .serve_with_incoming(incoming),
    );
    let exporter = SpanExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .with_timeout(Duration::from_secs(5))
        .build()
        .unwrap();

    let captured = CapturingLayer::default();
    let _subscriber =
        tracing::subscriber::set_default(tracing_subscriber::registry().with(captured.clone()));
    exporter
        .export(vec![finished_span()])
        .await
        .expect("a partially rejected export is still a successful one");

    let events = captured.0.lock().unwrap();
    let rejection = events
        .iter()
        .find(|event| event.name == "TonicTracesClient.PartialSuccess")
        .expect("the rejection is logged");
    assert_eq!(rejection.level, Level::WARN);
    assert_eq!(rejection.target, "opentelemetry-otlp");
    assert!(rejection.fields.contains(REJECTION_REASON));
}
