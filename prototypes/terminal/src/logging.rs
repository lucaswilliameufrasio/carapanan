use std::{env, path::PathBuf};

use tracing_appender::{non_blocking::WorkerGuard, rolling};
use tracing_subscriber::{
    EnvFilter, Layer, filter::filter_fn, fmt, layer::SubscriberExt, util::SubscriberInitExt,
};

/// Installs opt-in local JSON logging when `CARAPANA_LOG_DIR` is set.
/// Files rotate daily and retain at most seven generations. Logging failures
/// do not prevent the mock-only prototype from starting.
pub(super) fn install() -> Option<WorkerGuard> {
    let directory = env::var_os("CARAPANA_LOG_DIR").filter(|value| !value.is_empty())?;
    let directory = PathBuf::from(directory);
    install_in(&directory)
        .map_err(|_| {
            eprintln!("Carapanã: logging unavailable; continuing without file logs.");
        })
        .ok()
}

fn install_in(
    directory: &std::path::Path,
) -> Result<WorkerGuard, Box<dyn std::error::Error + Send + Sync>> {
    let appender = rolling::Builder::new()
        .rotation(rolling::Rotation::DAILY)
        .filename_prefix("carapana")
        .max_log_files(7)
        .build(directory)?;
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("carapana=info"));

    tracing_subscriber::registry()
        .with(filter)
        .with(
            fmt::layer()
                .json()
                .with_target(false)
                .with_writer(writer)
                .with_filter(filter_fn(|metadata| metadata.target() == "carapana::trace")),
        )
        .try_init()?;

    Ok(guard)
}

#[cfg(test)]
mod tests {
    use super::install_in;
    use carapana_core::tracing::{
        LocalTraceSink, TraceEvent, TraceLevel, TraceRecord, TraceResult, TraceSink,
    };
    use std::{fs, time::SystemTime};

    #[test]
    fn should_write_only_structured_trace_record_fields_to_rotating_file() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("carapana-logs-{unique}"));
        fs::create_dir(&directory).unwrap();

        let guard = install_in(&directory).unwrap();
        LocalTraceSink.record(TraceRecord::new(
            42,
            TraceEvent::AuthorizationChecked,
            TraceLevel::Warn,
            TraceResult::Denied,
            Some(8),
        ));
        tracing::info!(target: "carapana::other", secret = "must not be logged");
        drop(guard);

        let entries = fs::read_dir(&directory)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(entries.len(), 1);
        let line = fs::read_to_string(entries[0].path()).unwrap();
        let record: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(record["fields"]["correlation"], 42);
        assert_eq!(record["fields"]["event"], "authorization_checked");
        assert_eq!(record["fields"]["result"], "denied");
        assert_eq!(record["fields"]["elapsed_millis"], 8);
        assert!(
            record["fields"]["elapsed_millis_present"]
                .as_bool()
                .unwrap()
        );
        assert!(!line.contains("message_id"));
        assert!(!line.contains("secret"));

        fs::remove_dir_all(directory).unwrap();
    }
}
