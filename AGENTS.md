---
type: Playbook
title: "AGENTS.md for system-diag-tool"
description: "システム診断ツール固有の制約とグローバル規約との差分。"
tags:
  - agents
  - system-diag
  - rust
generated:
  by: opencode
  at: "2026-10-31T00:27:37+09:00"
---

## プロジェクト概要

Linux のシステム診断とベースライン比較を行う Rust 製 CLI。CPU、メモリ、
ネットワーク、DNS、NTP などの状態を収集し、`baseline.json` と比較する。
一部の診断は root 権限または `CAP_NET_RAW` を必要とする。

## グローバル設定との関係

- 共通規約はグローバル `~/.config/opencode/AGENTS.md` を継承する。
- 本ファイルは本プロジェクト固有の差分だけを記述する。グローバルと
  重複する規約は追加せず、差分がある場合にのみ上書きする。
- グローバル TODO は `~/.config/opencode/todo.md`。複数プロジェクト
  横断のタスクはそこに置き、本プロジェクト固有のタスクは本リポジトリの
  issue やドキュメントで管理する。
- 本ファイルとグローバルが矛盾する場合は、本ファイルの記述を優先する。

## プロジェクト固有の制約

- 診断の実行は読み取り専用を基本とし、`save` / `compare` で
  `baseline.json` を上書きする前に差分を確認する。
- root 権限が必要な診断を、権限のない環境で成功したものとして扱わない。

## 検証

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

CI は `onoderakeita/workflows/rust-gates.yml@v1` に委譲している。
