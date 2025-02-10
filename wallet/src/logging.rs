use anyhow::Result;
use std::panic;
use tracing::error;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

/// Initialize tracing to save logs into the logs/ folder
pub fn setup_tracing() -> Result<()> {
    let file_appender = RollingFileAppender::new(Rotation::DAILY, "logs", "wallet.log");

    tracing_subscriber::registry()
        .with(fmt::layer().with_writer(file_appender))
        .with(EnvFilter::from_default_env().add_directive(tracing::Level::TRACE.into()))
        .init();

    Ok(())
}

/// Make sure tracing is able to log panics occurring in the wallet
pub fn setup_panic_hook() {
    panic::set_hook(Box::new(|panic_info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        error!("Application panicked!");
        error!("Panic info: {:?}", panic_info);
        error!("Backtrace: {:?}", backtrace);
    }));
}
