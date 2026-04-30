// a2a (Agent to Agent) - v0.1.0
// https://github.com/OpenBST/a2a
//
// Author: OpenBST
// License: MIT OR Apache-2.0

use std::process::ExitCode;

fn main() -> ExitCode {
    match a2a::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            // Soft user errors (`UserError`): expected outcomes such as
            // "profile name taken" or "API key already stored". Print
            // to stdout, exit 0 — shells don't render a stack trace.
            if let Some(uerr) = err.downcast_ref::<a2a::UserError>() {
                println!("{}", uerr.0);
                ExitCode::SUCCESS
            } else if let Some(bf) = err.downcast_ref::<a2a::BusinessFailure>() {
                // Upstream / external failure (TLS, model quota,
                // network). Exit non-zero so CI can branch on it,
                // but write to stdout — the per-model failure
                // messages were already streamed during the run, and
                // hitting stderr here would make PowerShell wrap the
                // exit with a `NativeCommandError` stack trace
                // pointing inside its own temporary script wrapper,
                // which is pure noise to the user.
                println!("{}", bf.0);
                ExitCode::from(1)
            } else {
                // Genuine a2a system / programming errors: stderr +
                // exit 1. Stack-trace-style PS wrapping is fine here
                // because the user genuinely needs to file an issue.
                eprintln!("Error: {err:#}");
                ExitCode::from(1)
            }
        }
    }
}
