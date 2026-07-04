use anyhow::Result;
use clap::{Parser, Subcommand};
use std::fs;
use std::path::PathBuf;
use crate::diag::Diagnostic;

mod diag;

#[derive(Parser)]
#[command(author, version, about = "System Diagnostic Tool")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Run,
    Save { path: PathBuf },
    Compare { baseline: PathBuf },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let diagnostics: Vec<Box<dyn Diagnostic>> = vec![
        Box::new(diag::system::UptimeLoadAvgDiagnostic),
        Box::new(diag::cpu::CpuFrequencyDiagnostic),
        Box::new(diag::cpu_vuln::CpuVulnerabilityDiagnostic),
        Box::new(diag::net::LocalGatewayPingDiagnostic),
        Box::new(diag::conn::ActiveConnectionDiagnostic),
        Box::new(diag::user::ActiveUsersSshDiagnostic),
        Box::new(diag::mtu::NetworkMTUDiagnostic),
        Box::new(diag::time::NtpTimeSyncDiagnostic),
        Box::new(diag::dns::DnsLatencyDiagnostic),
    ];

    match cli.command {
        Commands::Run => {
            for d in diagnostics {
                println!("{}: {}", d.name(), d.run()?);
            }
        }
        Commands::Save { path } => {
            let mut snapshot = serde_json::Map::new();
            for d in diagnostics {
                snapshot.insert(d.name().to_string(), d.report());
            }
            fs::write(path, serde_json::to_string_pretty(&snapshot)?)?;
            println!("Snapshot saved.");
        }
        Commands::Compare { baseline } => {
            let baseline_content = fs::read_to_string(baseline)?;
            let baseline_json: serde_json::Value = serde_json::from_str(&baseline_content)?;
            let mut current = serde_json::Map::new();
            for d in diagnostics {
                current.insert(d.name().to_string(), d.report());
            }
            
            // Simple comparison logic
            if serde_json::Value::Object(current) == baseline_json {
                println!("No drift detected.");
            } else {
                println!("Drift detected!");
                // Here we could implement more detailed diffing
            }
        }
    }
    Ok(())
}
