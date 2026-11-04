# システム診断ツール テスト仕様書

## 1. テスト目的

本ツールの各診断モジュールのロジックおよび、ベースライン管理機能の信頼性を担保する。ユニットテストではロジックの分離と異常系（エラーハンドリング）を検証し、堅牢性を向上させる。

## 2. テストケース

| ID | 機能 | 内容 | 期待結果 |
| :--- | :--- | :--- | :--- |
| TC-01 | Diagnostic トレイト | 各モジュールの `run()` | 正常終了し `DiagnosticData` を返す |
| TC-02 | ロジック関数 (Unit) | 各モジュールの診断ロジック | 入力データに対する正しい解析結果(正常系)が返されること |
| TC-03 | ロジック関数 (Unit) | 各モジュールのエラーハンドリング | 不正な入力データに対する適切なErrが返されること |
| TC-04 | CLI 解析 | `save` コマンド | 正しく JSON ファイルが作成されること |
| TC-05 | ベースライン比較 | `compare` コマンド | 同一環境で変更なしと判定されること |
| TC-06 | ベースライン比較 | `compare` コマンド | 差異がある場合に「Drift detected」となること |
| TC-07 | Health 分類 | carrier down | `down` / `carrier-down` |
| TC-08 | Health 分類 | IPv4 なし | `down` / `no-ipv4` |
| TC-09 | Health 分類 | gateway のみ不通 | `degraded` |
| TC-10 | Health 分類 | 上流のみ不通 | `degraded` |
| TC-11 | Health 分類 | DNS のみ失敗 | `degraded` |
| TC-12 | Health 分類 | プロキシ TCP のみ失敗 | `degraded` |
| TC-13 | Health 分類 | 直接 TCP のみ失敗 | `degraded` |
| TC-14 | Health 分類 | 測定失敗 | `unknown`（`down` と区別） |
| TC-15 | Health 分類 | interface 不在 | `unknown` / `interface_present=false` |
| TC-16 | 引数検証 | interval が 0 または非数値 | エラー |
| TC-17 | 引数検証 | 存在しない interface | 起動時にエラー |
| TC-18 | コマンド実行 | タイムアウト | 期限で打ち切り Err |
| TC-19 | コマンド実行 | 正常終了 | status と出力を返す |
| TC-20 | コマンド実行 | spawn 失敗 | Err |
| TC-21 | コマンド実行 | 孫が pipe を保持 | 500 ms 以内に回収 |
| TC-22 | コマンド実行 | 大量出力 | デッドロックしない |
| TC-23 | heartbeat | 書き出し | 必須キーを持つ JSON |
| TC-24 | パス解決 | 既定の log / heartbeat | 期待するパス |
| TC-25 | 時刻整形 | RFC 3339（正負） | 期待形式 |
| TC-26 | 文字列 | UTF-8 境界の切り詰め | 壊れない |
| TC-27 | パース | gateway / IPv4 / offset | 期待値 |

## 3. テスト実行環境

- 環境: Linux (root/CAP_NET_RAW権限が必要な場合あり)
- 手順: `cargo test` (ユニットテスト) および `cargo run` を用いた統合テスト。

## 4. watch の手動確認

常駐監視はユニットテストに加え、実機で次を確認する。

- `systemctl --user show -p WatchdogUSec` が `1min` を返す。
- `~/.local/state/system-diag/net-watch.heartbeat` の mtime が
  更新され続ける。
- 状態が変わらない間、`net-watch.jsonl` に行が増えない。
- リンク断の発生時に `carrier-down` が記録され、`down` と
  `unknown` が混同されない。
