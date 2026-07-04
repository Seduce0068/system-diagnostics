#!/usr/bin/env rust-script
//! ```cargo
//! [dependencies]
//! anyhow = "1.0"
//! pnet = "0.35"
//! reqwest = { version = "0.11", features = ["json", "stream"] }
//! clap = { version = "4.0", features = ["derive"] }
//! tokio = { version = "1.30", features = ["full"] }
//! futures = "0.3"
//! ```

use std::time::{Duration, Instant};
use pnet::datalink;
use anyhow::{Result, Context, anyhow};
use clap::Parser;
use tokio::time::timeout;
use futures::future::join_all;
use futures::StreamExt;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// タイムアウト時間（秒）
    #[arg(short, long, default_value_t = 10)]
    timeout: u64,

    /// 速度テスト用URL (デフォルトは 10MB のデータ)
    #[arg(short, long, default_value = "https://speed.cloudflare.com/__down?bytes=10485760")]
    url: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let timeout_duration = Duration::from_secs(args.timeout);

    println!("========================================");
    println!("   🌐 堅牢版ネットワーク診断ツール (Async/Rust)   ");
    println!("========================================\n");

    check_local_interfaces();

    println!("\n[2] インターネット疎通確認 (並列検証):");
    match check_internet_connectivity(timeout_duration).await {
        Ok(_) => println!("  ✅ インターネット接続: 正常"),
        Err(e) => println!("  ❌ インターネット接続: 異常 ({})", e),
    }
    
    // 速度テスト
    println!("\n[3] ダウンロード速度テスト (Target: {}):", args.url);
    match measure_download_speed(&args.url, timeout_duration).await {
        Ok(speed) => println!("  ✅ 推測速度: {:.2} Mbps", speed),
        Err(e) => println!("  ❌ 速度計測失敗: {}", e),
    }

    println!("\n----------------------------------------");
    println!("✅ 診断完了");

    Ok(())
}

async fn measure_download_speed(url: &str, timeout_duration: Duration) -> Result<f64> {
    let client = reqwest::Client::builder()
        .timeout(timeout_duration)
        .build()?;
    
    let start = Instant::now();
    let response = client.get(url).send().await.context("リクエスト送信失敗")?;
    
    if !response.status().is_success() {
        return Err(anyhow!("HTTPエラー: {}", response.status()));
    }
    
    let mut total_bytes = 0;
    let mut stream = response.bytes_stream();
    
    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result.context("レスポンス取得失敗")?;
        total_bytes += chunk.len();
    }
    
    let duration = start.elapsed().as_secs_f64();
    if duration <= 0.0001 { return Ok(0.0); } // ゼロ除算・極小値による無限大の防止
    
    let mbps = ((total_bytes * 8) as f64 / 1_000_000.0) / duration;
    Ok(mbps)
}

fn check_local_interfaces() {
    println!("[1] ローカルインターフェース:");
    for interface in datalink::interfaces() {
        if interface.is_up() && !interface.is_loopback() && !interface.ips.is_empty() {
            let ips: Vec<String> = interface.ips.iter().map(|ip| ip.to_string()).collect();
            println!("  ✅ {} (UP) - IP: {}", interface.name, ips.join(", "));
        }
    }
}

async fn check_internet_connectivity(timeout_duration: Duration) -> Result<()> {
    let targets = ["8.8.8.8:443", "1.1.1.1:443"];
    let mut tasks = Vec::new();
    
    for target in targets {
        let target_str = target.to_string();
        tasks.push(tokio::spawn(async move {
            let start = Instant::now();
            let result = timeout(timeout_duration, tokio::net::TcpStream::connect(&target_str)).await;
            (target_str, result, start.elapsed())
        }));
    }
    
    let results = join_all(tasks).await;
    let mut successful = false;
    
    for res in results {
        match res {
            Ok((target, result, duration)) => {
                match result {
                    Ok(Ok(_)) => {
                        println!("  ✅ {}: 接続成功 ({:?})", target, duration);
                        successful = true;
                    }
                    Ok(Err(e)) => {
                        println!("  ⚠️ {}: 接続拒否/失敗 ({:?}) - {}", target, duration, e);
                    }
                    Err(_) => {
                        println!("  ⚠️ {}: タイムアウト ({:?})", target, duration);
                    }
                }
            }
            Err(e) => {
                println!("  ⚠️ 疎通タスク実行エラー (パニックなど): {}", e);
            }
        }
    }
    
    if successful {
        Ok(())
    } else {
        Err(anyhow!("全てのターゲットへの接続に失敗しました"))
    }
}
