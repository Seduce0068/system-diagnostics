# MTU 診断ツール 技術設計書

## 1. 概要

本ツールは、Raw Socket を直接操作することで、ネットワーク上の MTU (Maximum Transmission Unit) を断片化させずに通過可能な最大値として特定するRust製のCLIツールである。

## 2. 設計方針

- **環境非依存:** 外部コマンド（ping等）への依存を排除し、Rustのネイティブライブラリのみで完結させる。
- **堅牢性:** ネットワークの状態に左右されず、ゲートウェイを含めた段階的な診断を行う。
- **正確性:** DFビット制御による断片化テストと、動的なIP/ICMPヘッダー解析を行う。

## 3. コンポーネント構成

- **`get_default_interface`**: nix API を使用し、有効なネットワークインターフェースを自動選定。
- **`get_default_gateway`**: `/proc/net/route` を解析し、デフォルトゲートウェイを特定。
- **`build_icmp_echo_request`**: ICMP Echo Request パケットをバイト列として構築（チェックサム計算を含む）。
- **`perform_ping`**: Raw Socket を作成し、バインド、送信、受信タイムアウト処理を実行。
- **`detect_optimal_mtu`**: 576〜1500の間で二分探索を実施。

## 4. セキュリティ

- Raw Socket を使用するため、実行には `CAP_NET_RAW` 権限（通常は root）が必要。
