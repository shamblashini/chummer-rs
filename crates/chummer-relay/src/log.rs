//! Logging set-up for the relay binary.

use std::fmt::Debug;

use tracing::field::{Field, Visit};
use tracing::{Event, Metadata};
use tracing_subscriber::layer::{Context, Filter};

const HTTP_SERVER: &str = "iroh_relay::server::http_server";

/// Drops iroh-relay's "failed to handle connection" error for a connection
/// that was not upgraded to the relay protocol within 30 seconds. Behind a
/// reverse proxy every ordinary request (the apps' `/ping` latency probe,
/// health checks) leaves such a connection open for reuse, and the relay
/// logged an ERROR for each one although the request had been answered.
/// Port scanners stalling a handshake cause the same message without a
/// proxy. Every other error of that module is kept.
#[derive(Clone, Copy, Debug, Default)]
pub struct QuietIdleConnections;

impl<S> Filter<S> for QuietIdleConnections {
    fn enabled(&self, _: &Metadata<'_>, _: &Context<'_, S>) -> bool {
        true
    }

    fn event_enabled(&self, event: &Event<'_>, _: &Context<'_, S>) -> bool {
        if event.metadata().target() != HTTP_SERVER {
            return true;
        }
        let mut found = IdleTimeout(false);
        event.record(&mut found);
        !found.0
    }
}

struct IdleTimeout(bool);

impl Visit for IdleTimeout {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "error" {
            let text = format!("{value:?}");
            if text.contains("EstablishTimeout") || text.contains("did not reach established state") {
                self.0 = true;
            }
        }
    }
}

/// Starts logging to stderr: `RUST_LOG`, or `default` when it is unset.
pub fn init(default: &str) {
    use tracing_subscriber::prelude::*;
    let env = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| default.into());
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_filter(env).with_filter(QuietIdleConnections))
        .init();
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use tracing_subscriber::prelude::*;

    use super::*;

    #[derive(Debug)]
    struct EstablishTimeout;

    impl std::fmt::Display for EstablishTimeout {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("Connection did not reach established state within timeout")
        }
    }

    #[derive(Clone, Default)]
    struct Lines(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for Lines {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn idle_proxy_connections_are_not_logged_as_errors() {
        let out = Lines::default();
        let w = out.clone();
        let layer = tracing_subscriber::fmt::layer().with_writer(move || w.clone()).with_ansi(false).with_filter(QuietIdleConnections);
        tracing::subscriber::with_default(tracing_subscriber::registry().with(layer), || {
            let error = EstablishTimeout;
            tracing::error!(target: "iroh_relay::server::http_server", ?error, "failed to handle connection");
            let error = std::io::Error::other("bad request");
            tracing::error!(target: "iroh_relay::server::http_server", ?error, "failed to handle connection");
            let error = EstablishTimeout;
            tracing::error!(target: "chummer_relay", ?error, "elsewhere");
        });
        let text = String::from_utf8(out.0.lock().unwrap().clone()).unwrap();
        assert_eq!(text.lines().count(), 2, "{text}");
        assert!(text.contains("bad request"));
        assert!(text.contains("elsewhere"));
    }
}
