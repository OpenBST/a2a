// a2a (Agent to Agent) - v0.1.0
// https://github.com/OpenBST/a2a
//
// Author: OpenBST
// License: MIT OR Apache-2.0

//! `a2a doctor`: environment health check. SPEC §3.

use anyhow::Result;
use clap::Parser;

#[derive(Parser, Debug)]
pub struct DoctorArgs {}

pub fn run(_args: DoctorArgs) -> Result<()> {
    crate::runner::doctor::run_doctor()
}
