//! Tracing-subscriber init matching the bridge-craftwork service contract:
//! JSON logs to stdout in production, pretty logs locally. `format` is "json"
//! (prod) or "pretty" (local dev).

use tracing_subscriber::{fmt, prelude::*, EnvFilter};

pub fn init(level: &str, format: &str) {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("{level},tower_http=info")));

    let reg = tracing_subscriber::registry().with(filter);

    match format {
        "pretty" => reg.with(fmt::layer().pretty()).init(),
        _ => reg
            .with(
                fmt::layer()
                    .json()
                    .with_current_span(false)
                    .with_span_list(false)
                    .with_target(false),
            )
            .init(),
    }
}
