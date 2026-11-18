// SPDX-License-Identifier: AGPL-3.0-only

//! Command line and environment configuration.
//!
//! The options of the Python server (`finanze/args.py`) are accepted unchanged, so
//! existing compose files keep working, and are passed on to the Python process.
//! Options that only concern the Rust gateway start with `--backend` or `--python`.

use std::path::PathBuf;

use clap::{Parser, ValueEnum};

/// Log levels of the Python server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "UPPERCASE")]
pub enum LogLevel {
    None,
    Debug,
    Info,
    Warning,
    Error,
    Critical,
}

impl LogLevel {
    /// The argument value the Python server expects.
    pub const fn as_arg(self) -> &'static str {
        match self {
            LogLevel::None => "NONE",
            LogLevel::Debug => "DEBUG",
            LogLevel::Info => "INFO",
            LogLevel::Warning => "WARNING",
            LogLevel::Error => "ERROR",
            LogLevel::Critical => "CRITICAL",
        }
    }

    /// The equivalent `tracing` filter directive.
    pub const fn as_filter(self) -> &'static str {
        match self {
            LogLevel::None => "off",
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Warning => "warn",
            LogLevel::Error | LogLevel::Critical => "error",
        }
    }
}

/// Ferrofolio server: serves the ported API routes and forwards all others to the
/// Python backend, which it starts on the loopback interface.
#[derive(Debug, Clone, Parser)]
#[command(name = "ferrofolio", version, about)]
pub struct Cli {
    /// Directory to store database and configuration files (passed to the backend).
    #[arg(long)]
    pub data_dir: Option<PathBuf>,

    /// Host interface on which the server listens.
    #[arg(long, env = "FINANZE_HOST", default_value = "127.0.0.1")]
    pub host: String,

    /// Port on which the server listens. The backend builds redirect URLs with it,
    /// so it cannot be 0.
    #[arg(long, default_value_t = 7592, value_parser = clap::value_parser!(u16).range(1..))]
    pub port: u16,

    /// Console logging level (NONE disables console logging).
    #[arg(long, value_enum, default_value_t = LogLevel::Info, ignore_case = true)]
    pub log_level: LogLevel,

    /// Log directory of the backend.
    #[arg(long)]
    pub log_dir: Option<PathBuf>,

    /// File logging level of the backend.
    #[arg(long, value_enum, ignore_case = true)]
    pub log_file_level: Option<LogLevel>,

    /// Logging level for third-party libraries.
    #[arg(long, value_enum, default_value_t = LogLevel::Error, ignore_case = true)]
    pub third_party_log_level: LogLevel,

    /// Path to the llama.cpp server executable (passed to the backend).
    #[arg(long)]
    pub llama_server_path: Option<PathBuf>,

    /// Loopback port for the Python backend it starts; 0 picks a free port.
    #[arg(long, env = "FERROFOLIO_BACKEND_PORT", default_value_t = 0)]
    pub backend_port: u16,

    /// Python interpreter used to start the backend.
    #[arg(long, env = "FERROFOLIO_PYTHON", default_value = "python3")]
    pub python: String,

    /// Path of the Python backend package (the `finanze` directory).
    #[arg(long, env = "FERROFOLIO_BACKEND_APP", default_value = "finanze")]
    pub backend_app: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_the_python_server_arguments() {
        let cli = Cli::try_parse_from([
            "ferrofolio",
            "--port",
            "7592",
            "--data-dir",
            "/.storage",
            "--log-dir",
            "/.storage/logs",
            "--log-level",
            "DEBUG",
        ])
        .unwrap();
        assert_eq!(cli.port, 7592);
        assert_eq!(cli.log_level, LogLevel::Debug);
        assert_eq!(cli.data_dir, Some(PathBuf::from("/.storage")));
        assert_eq!(cli.third_party_log_level, LogLevel::Error);
    }

    #[test]
    fn the_public_port_cannot_be_zero() {
        assert!(Cli::try_parse_from(["ferrofolio", "--port", "0"]).is_err());
    }
}
