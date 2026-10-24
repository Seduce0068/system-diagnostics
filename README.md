# システム診断 & ベースライン比較ツール

## 1. 概要

本ツールは、Linux システムの包括的な診断および状態変化（ドリフト）検知を行う Rust 製のツールです。CPU、メモリ、ネットワーク、DNS、システム設定などの網羅的なチェック機能を備え、現在の状態をスナップショットとして保存・比較することが可能です。

## 2. 前提条件

- OS: Linux
- 権限: 一部の診断項目に `root` 権限（または `CAP_NET_RAW` 等の機能権限）が必要です。

## 3. 実行方法

```bash
# 全ての診断を実行
sudo cargo run -- run

# 現在の状態をベースラインとして保存
sudo cargo run -- save baseline.json

# 保存したベースラインと現在の状態を比較
sudo cargo run -- compare baseline.json

# リンク/疎通の変化を監視し、変化時に証跡を記録
cargo run -- watch --interface enp4s0 --interval 5
```

`watch` は carrier、IPv4、既定経路、ゲートウェイ到達性、上位網到達性、
DNS 解決、TCP 接続を定期測定し、状態が変化した時点の
コマンド出力一式（`ip`、`nmcli`、`journalctl` 等）を JSONL で追記する。
TCP は 2 経路を別々に記録する。

- `tcp_ok`: `1.1.1.1:443`（AdGuard CLI の透過プロキシ経由）
- `direct_tcp_ok`: `1.1.1.1:53`（プロキシ範囲外の直接接続）

両者の差から、プロキシ層とネットワーク層のどちらが壊れたかを
事後に判定できる。
既定の保存先は `$XDG_STATE_HOME/system-diag/net-watch.jsonl`
（未設定時は `~/.local/state/system-diag/net-watch.jsonl`）。
JSONL は状態変化時のみ追記する。監視ループの生存確認は、
同じディレクトリの `net-watch.heartbeat` を毎周期上書きして行う。
常駐時はこの heartbeat の mtime を監視すること。

`watch` は外部コマンドに上限時間を設け、失敗を次のように分類する。

- `ok` / `degraded` / `down`: 測定が完了したうえでの判定。
- `unknown`: `ip`、`ping`、`getent` の起動失敗やタイムアウトで
  判定できない状態。ネットワーク障害とは区別する。

各レコードは `schema_version: 2` と `event`
（`started` / `changed` / `once`）を持ち、`evidence` は
`stdout`、`stderr`、`exit_code`、`timed_out` を個別に保持する。

不通発生時の切り分け手順と常駐方法は [RUNBOOK.md](RUNBOOK.md) を
参照してください。

## 4. 診断内容

- **システム:** 稼働時間、ロードアベレージ
- **CPU:** 周波数、熱制限（スロットリング）検知、脆弱性ステータス
- **ネットワーク:** デフォルトゲートウェイのMTU診断、確立済み接続数の集計
- **ユーザー/認証:** アクティブユーザー数、SSH認証失敗数の監視
- **DNS/Time:** DNSレイテンシ、NTP同期ステータス

## 5. 出力例

```bash
稼働時間: 1日 17時間 15分 | 負荷平均: 2.8 [5分], 2.34 [15分] | CPUステータス: 正常 (1分平均: 2.45)
...
Snapshot saved.
```
