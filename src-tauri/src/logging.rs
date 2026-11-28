use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::prelude::*;
use tracing_subscriber::{EnvFilter, fmt};

/// Console + daily rolling file. Filter with `OVERLAI_LOG` (e.g. `OVERLAI_LOG=debug`).
/// Keep the returned guard alive for the whole program, it flushes the file writer.
pub fn init(log_dir: &Path) -> Option<WorkerGuard> {
    let filter = EnvFilter::try_from_env("OVERLAI_LOG")
        // rmcp logs every MCP request at info level.
        .unwrap_or_else(|_| EnvFilter::new("info,game_overlai=debug,rmcp=warn"));

    let file = std::fs::create_dir_all(log_dir).ok().and_then(|_| {
        RollingFileAppender::builder()
            .rotation(Rotation::DAILY)
            .filename_prefix("game-overlai")
            .filename_suffix("log")
            .max_log_files(7)
            .build(log_dir)
            .ok()
    });

    let console = fmt::layer().with_target(false);
    match file {
        Some(appender) => {
            let (writer, guard) = tracing_appender::non_blocking(appender);
            let file_layer = fmt::layer().with_ansi(false).with_target(false).with_writer(writer);
            tracing_subscriber::registry().with(filter).with(console).with(file_layer).init();
            Some(guard)
        }
        None => {
            tracing_subscriber::registry().with(filter).with(console).init();
            None
        }
    }
}
