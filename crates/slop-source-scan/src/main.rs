// SPDX-License-Identifier: AGPL-3.0-or-later
//! `slop-source-check <dir>`: SLAPorSLOP's source check on a checked-out repository, for a
//! maker's GitHub runner (the Slop Build recipe's `source-checks` job). Reads the tree the way
//! the server reads a repository archive (`.git` left out, symbolic links skipped, nothing run)
//! and prints, as one line of JSON on stdout:
//!
//! ```text
//! {"raw": <RawScan>, "tool": "slop-source-check <version>"}
//! ```
//!
//! The store page's licence and visibility are applied by the server (`finalize`).

use std::{path::Path, process::ExitCode};

use slop_source_scan::{SourceCheck, scan_dir, tool};

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let (Some(dir), None) = (args.next(), args.next()) else {
        eprintln!("usage: slop-source-check <dir>");
        return ExitCode::from(2);
    };
    let raw = match scan_dir(Path::new(&dir)) {
        Ok(raw) => raw,
        Err(err) => {
            eprintln!("slop-source-check: {err:#}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(why) = raw.validate() {
        // SLAPorSLOP will refuse it; printed anyway, so the maker can see what it found.
        eprintln!("slop-source-check: SLAPorSLOP won't accept this result: {why}");
    }
    match serde_json::to_string(&SourceCheck { raw, tool: tool() }) {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("slop-source-check: {err}");
            ExitCode::FAILURE
        }
    }
}
