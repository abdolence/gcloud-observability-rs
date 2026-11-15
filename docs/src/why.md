# Why not plain OpenTelemetry

`tracing`, `tracing-opentelemetry`, `opentelemetry-otlp`, `opentelemetry-appender-tracing` and the JSON formatter of `tracing-subscriber` can send spans and logs to Google Cloud without this library.
They know nothing about Google Cloud, so each of the points below is left to the application.

## Authentication

The Telemetry API needs Google credentials on every request, and `opentelemetry-otlp` has no hook for Google tokens other than a tonic `Interceptor`, a callback that edits each outgoing request.
An interceptor runs synchronously while Google tokens are refreshed asynchronously.

- `GcpAuthInterceptor` refreshes the credentials in a background task and publishes the headers through a watch channel, so each request copies the latest ones without waiting;
- a failed refresh keeps the last headers in use and logs a warning when it starts failing and another when it recovers;
- the exporter builders set the endpoint, TLS roots, the connect, TLS handshake and request timeouts of the channel, and the `gcp.project_id` resource attribute that the Telemetry API reads;
- the same interceptor authenticates an `opentelemetry-otlp` metrics exporter, see [Metrics over OTLP](./metrics.md).

See [Authentication](./authentication.md) and [OTLP exporter](./traces/otlp.md).

## JSON lines on stdout

GKE, Cloud Run and hosts with the Cloud Logging agent turn a JSON line on stdout into a log entry, and read special keys from it: `severity`, `logging.googleapis.com/trace`, `logging.googleapis.com/spanId`, `logging.googleapis.com/trace_sampled`, `logging.googleapis.com/sourceLocation`, `logging.googleapis.com/labels`, `logging.googleapis.com/insertId`, `logging.googleapis.com/operation` and `httpRequest`.
See Google's [structured logging](https://docs.cloud.google.com/logging/docs/structured-logging) reference.

- The JSON output of `tracing-subscriber` writes none of them: it writes the level as `level` rather than `severity`, and it has no trace or span keys, so the entry is not linked to its trace;
- the OpenTelemetry crates have no sink that writes this format;
- the [JSON lines sink](./logging/json-lines.md) writes these keys, and takes the trace and span from the OpenTelemetry span the event happened in.

## Log entries over OTLP

The stock bridge, `opentelemetry-appender-tracing`, sends a `tracing` event as a text payload with every field as a string label, and a log name made per call site.

- The [OTLP sink](./logging/otlp.md) sends the fields as a typed payload, fills `httpRequest`, `sourceLocation`, `operation` and `insertId`, and writes every entry to the log named by its `LogId`;
- [Comparison with the OpenTelemetry tracing bridge](./logging/otlp-bridge.md) shows the same event through both.

## Error Reporting

Error Reporting groups only entries in its own format, and no plain crate writes it.
The [logging layer](./errors/error-reporting.md) does, for every sink:

- the `@type` of a `ReportedErrorEvent`, the `serviceContext` and the `reportLocation` of the event;
- the `source()` chain of an error passed as `&dyn Error`, on the first line of the message;
- a [backtrace](./errors/backtraces.md) from `Backtrace::capture`, which costs nothing while `RUST_BACKTRACE` is unset;
- a [panic hook](./errors/panic-hook.md) that reports the file and line where the panic happened.

## Other details

- The layer skips events of the crates an export goes through (`h2`, `hyper`, `tonic`, `rustls`, `opentelemetry` and others) when its sink sends over the network, so that a log entry about an export is not itself exported, which at DEBUG or TRACE level would make one more export for each;
- events inside a span are not attached to the span as span events by default, which Trace Explorer would show beside the log entry, see [Span events and annotations](./traces/span-events.md);
- entries sent over OTLP store the bare trace ID, so they are filtered with `trace:"TRACE_ID"`, see the [OTLP sink](./logging/otlp.md).

## When plain OpenTelemetry is enough

An application that sends only spans over OTLP and already handles Google authentication can use `opentelemetry-otlp` alone.
