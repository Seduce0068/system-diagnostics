use anyhow::Result;
use clap::{Parser, Subcommand};
use std::fs;
use std::path::PathBuf;

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

    match cli.command {
        Commands::Run => {
            for d in diag::get_all_diagnostics() {
                match d.run() {
                    Ok(data) => println!("{}: {}", d.name(), data.readable),
                    Err(e) => eprintln!("{}: 診断失敗: {}", d.name(), e),
                }
            }
        }
        Commands::Save { path } => {
            let mut snapshot = serde_json::Map::new();
            for d in diag::get_all_diagnostics() {
                if let Ok(data) = d.run() {
                    snapshot.insert(d.name().to_string(), data.raw);
                }
            }
            fs::write(path, serde_json::to_string_pretty(&snapshot)?)?;
            println!("Snapshot saved.");
        }
        Commands::Compare { baseline } => {
            let baseline_content = fs::read_to_string(baseline)?;
            let baseline_json: serde_json::Value = serde_json::from_str(&baseline_content)?;
            let mut current = serde_json::Map::new();
            for d in diag::get_all_diagnostics() {
                if let Ok(data) = d.run() {
                    current.insert(d.name().to_string(), data.raw);
                }
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
