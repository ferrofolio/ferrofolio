// SPDX-License-Identifier: AGPL-3.0-only

use std::io::Write;
use std::process::ExitCode;

use clap::Parser;
use server::config::Cli;

fn main() -> ExitCode {
    let cli = Cli::parse();
    if let Err(error) = server::init_logging(&cli) {
        // Logging is not available, so the error goes to the console directly; if
        // that fails too, the exit code is all that is left.
        let _ = writeln!(std::io::stderr(), "could not configure logging: {error}");
        return ExitCode::FAILURE;
    }
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            tracing::error!(%error, "could not start the async runtime");
            return ExitCode::FAILURE;
        }
    };
    let exit_code = runtime.block_on(server::run(cli));
    // Dropping the runtime would wait without limit for calculations still running
    // on blocking threads.
    runtime.shutdown_timeout(server::RUNTIME_SHUTDOWN_GRACE);
    exit_code
}
