# MTU 診断ツール 利用ガイド

## 1. 概要

本ツールは、ローカルネットワークおよびインターネット上の MTU を診断する Rust スクリプトです。

## 2. 前提条件

- OS: Linux
- 権限: `root` 権限 (CAP_NET_RAW) が必要です。

## 3. 実行方法

```bash
sudo cargo run --manifest-path mtu_diag.rs
```

## 4. 診断内容

1. デフォルトインターフェースの自動検知
2. デフォルトゲートウェイの検知と MTU 診断
3. 外部ターゲット (8.8.8.8, 1.1.1.1) の MTU 診断

## 5. 出力例

``` markdown

=== 堅牢なMTU 診断ツール ===
診断対象インターフェース: eth0
ゲートウェイ検出: 192.168.1.1
Testing target: 192.168.1.1
推奨MTU (via eth0 - 192.168.1.1): 1500
Testing target: 8.8.8.8
推奨MTU (via eth0 - 8.8.8.8): 1492
```
