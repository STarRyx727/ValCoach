use std::path::PathBuf;

use valcoach_vrf_probe::{probe_file, write_probe_artifacts};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let replay = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: valcoach-vrf-probe <replay.vrf> [output-directory]")?;
    let report = probe_file(&replay)?;
    if let Some(output) = std::env::args_os().nth(2).map(PathBuf::from) {
        write_probe_artifacts(&report, &output)?;
    } else {
        let mut public_report = report;
        public_report.server_events.clear();
        println!("{}", serde_json::to_string_pretty(&public_report)?);
    }
    Ok(())
}
