// a2a (Agent to Agent) - v0.1.0
// https://github.com/OpenBST/a2a
//
// Author: OpenBST
// License: MIT OR Apache-2.0

use anyhow::Result;
use clap::Parser;

#[derive(Parser, Debug)]
pub struct StatusArgs {}

pub fn run(_args: StatusArgs) -> Result<()> {
    crate::runner::doctor::print_status()
}
