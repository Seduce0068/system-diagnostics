# 診断ツール修正計画

以下のファイルに対して、DiagnosticData構造体からnameフィールドを削除し、関連する未使用のインポートやコードをクリーンアップします。

## 対象ファイル

- src/diag/conn.rs
- src/diag/cpu.rs
- src/diag/cpu_vuln.rs
- src/diag/dns.rs
- src/diag/mtu.rs
- src/diag/net.rs
- src/diag/system.rs
- src/diag/time.rs
- src/diag/user.rs
- src/diag/mod.rs (構造体は修正済み。トレイトのfn nameメソッドを削除)

## 修正内容

1. DiagnosticData構造体の初期化箇所から `name: ...` を削除。
2. テストコードから `data.name` へのアクセスを削除。
3. `Diagnostic` トレイトから `fn name(&self) -> &str;` を削除し、各実装から `fn name` メソッドを削除。
4. 未使用のインポートを削除。
5. src/main.rs の修正。
6. `cargo check` による検証。

この手順に従い、各ファイルを修正してください。
