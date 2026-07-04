use anyhow::Result;
use clap::{Parser, Subcommand};
use std::fs;
use std::path::PathBuf;
use system_diag_tool::diag;

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

fn print_diff(val1: &serde_json::Value, val2: &serde_json::Value, path: String) {
    if val1 == val2 {
        return;
    }
    match (val1, val2) {
        (serde_json::Value::Object(map1), serde_json::Value::Object(map2)) => {
            for key in map1.keys().chain(map2.keys()).collect::<std::collections::HashSet<_>>() {
                let p = if path.is_empty() { key.to_string() } else { format!("{}.{}", path, key) };
                match (map1.get(key), map2.get(key)) {
                    (Some(v1), Some(v2)) => print_diff(v1, v2, p),
                    (Some(_), None) => println!("Removed: {}", p),
                    (None, Some(_)) => println!("Added: {}", p),
                    (None, None) => unreachable!(),
                }
            }
        }
        (v1, v2) => {
            println!("Changed: {} ({} -> {})", path, v1, v2);
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Run => {
            for d in diag::get_all_diagnostics() {
                match d.run() {
                    Ok(data) => println!("{}", data.readable),
                    Err(e) => eprintln!("診断失敗: {}", e),
                }
            }
        }
        Commands::Save { path } => {
            let mut snapshot = serde_json::Map::new();
            for d in diag::get_all_diagnostics() {
                match d.run() {
                    Ok(data) => {
                        snapshot.insert(d.id().to_string(), data.raw);
                    }
                    Err(e) => {
                        eprintln!("⚠️ 警告: 診断 '{}' が失敗したため、スナップショットに保存されませんでした: {}", d.id(), e);
                    }
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
                match d.run() {
                    Ok(data) => {
                        current.insert(d.id().to_string(), data.raw);
                    }
                    Err(e) => {
                        eprintln!("⚠️ 警告: 診断 '{}' が失敗したため、比較から除外されました: {}", d.id(), e);
                    }
                }
            }
            
            let current_json = serde_json::Value::Object(current);
            if current_json == baseline_json {
                println!("No drift detected.");
            } else {
                println!("Drift detected!");
                print_diff(&baseline_json, &current_json, "".to_string());
            }
        }
    }
    Ok(())
}
