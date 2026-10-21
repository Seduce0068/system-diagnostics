# ネットワーク不通時の切り分け手順（HR02 / enp4s0）

対象: 本 PC（Kubuntu）からのみインターネット接続が失われる事象。
他機器（スマートフォン等）は同時に通信できている前提とする。

## 1. 症状

- PC からのみ通信不可。他機器は同時に通信可。
- HR02 の LAN ランプは青のまま。セルフチェックは正常。
- REBOOT ボタン（1 秒押し）は効かないことが多い。
- ルーターの電源を 10 分以上切って再投入すると復旧する。

## 2. 判定したい仮説

| ID | 内容 | 復旧させる層 |
| --- | --- | --- |
| H1 | PC 側 NIC（r8168）の内部ハング | PC 側でリンク再交渉 |
| H2 | HR02 の PC 個別転送の破綻 | ルーターの再起動 |
| H3 | AdGuard CLI の透過プロキシ故障 | プロキシの停止または再起動 |

2026-10-20 の事象では、ICMP と DNS は通るのに新規 TCP だけが
失敗した。443 は AdGuard CLI の透過プロキシ経由なので、H3 を
最初に切り分ける。H3 は 2026-10-21 に確定した（6 章参照）。

## 3. 初動（ルーターに触る前に層を判定する）

### 3.1 プロキシ層とネットワーク層の判定

```bash
# 直接 TCP（ポート 53 は AdGuard の filtered_ports 範囲外）
nc -vz -w 5 1.1.1.1 53

# アプリ経路（443、AdGuard CLI の透過プロキシ経由）
curl -4 -sS -m 5 -o /dev/null -w '%{http_code}\n' https://www.google.com
```

- 直接 TCP 成功・443 失敗: **H3**。3.2 へ進む。
- 両方失敗: ネットワーク層。3.3 へ進む。

### 3.2 AdGuard CLI の再起動（H3 の復旧）

```bash
sudo systemctl restart adguard-cli
curl -4 -sS -m 5 -o /dev/null -w '%{http_code}\n' https://www.google.com
```

2026-10-21 の実測では、透過プロキシの待受キュー飽和と
`CLOSE-WAIT` の蓄積が原因で、再起動により即時復旧した。
再起動で直らない場合は、次の停止テストで切り分ける。

```bash
sudo systemctl stop adguard-cli
curl -4 -sS -m 5 -o /dev/null -w '%{http_code}\n' https://www.google.com
sudo systemctl start adguard-cli
```

443 が通るようになれば H3 確定。DNS フィルタは AdGuard Home が
継続するため、停止中も DNS レベルのブロックは維持される。

### 3.3 NIC の再交渉

```bash
sudo ip link set enp4s0 down
sleep 2
sudo ip link set enp4s0 up
sleep 10
```

疎通を確認する。

```bash
ping -c 3 192.168.128.1         # ゲートウェイ（L3 到達性）
ping -c 3 8.8.8.8               # 上位網（NAT 越え）
getent ahostsv4 www.google.com  # DNS 解決
```

- 復旧した場合: H1 確定。`5. 恒久対策` へ進む。
- 復旧しない場合: H2 の可能性。ルーターの電源を 10 分以上
  切って再投入する。

## 4. 証跡の自動記録

`watch` を常駐させると、状態変化の瞬間の `ip` / `nmcli` /
`journalctl` / sysfs 出力が JSONL に残る。手動操作の前後を
比較できるため、常時起動しておく。TCP は `tcp_ok`（443、
AdGuard 経由）と `direct_tcp_ok`（53、直接）の 2 系統で記録される。

```bash
cargo build --release
cargo run --release -- watch --interface enp4s0 --interval 5
```

systemd ユーザーユニットで常駐させる場合:

```bash
mkdir -p ~/.config/systemd/user
cp contrib/systemd/system-diag-watch.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now system-diag-watch.service
systemctl --user status system-diag-watch.service
```

ログ: `$XDG_STATE_HOME/system-diag/net-watch.jsonl`
（未設定時は `~/.local/state/system-diag/net-watch.jsonl`）

## 5. 恒久対策

### 5.1 H1 確定後

out-of-tree の `r8168` を撤去し、カーネル同梱の `r8169` に戻す。

```bash
sudo apt purge r8168-dkms
sudo reboot
ethtool -i enp4s0    # driver: r8169 を確認
```

ロールバックする場合は `sudo apt install r8168-dkms`。

### 5.2 H3 確定後（2026-10-21）

AdGuard CLI v1.4.13 の透過プロキシは `CLOSE-WAIT` を解放せず、
待受キューが飽和すると新規 TCP を落とす。次のいずれかで回避する。

- 監視 watchdog: 待受キューの `Recv-Q` または `CLOSE-WAIT` 件数が
  閾値を超えたら `systemctl restart adguard-cli` を実行する。
- 定期再起動: 1 日 1 回など、低頻度で再起動する。
- 上流修正: AdGuard CLI へバグ報告する。v1.4.13 が最新のため、
  アップデート待ちでは解決しない。

再起動でリダイレクト先ポートは変わる。watchdog は
`iptables -t nat -S AGCLI` から現在のポートを取得する。

## 6. 参考（2026-10 時点の実測）

- NIC: `10ec:8168` rev 15、ドライバ `r8168` 8.055.00（DKMS）。
- スリープ復帰のたびに `Device reseting!` とリンク再交渉が発生。
  5 日間で carrier 接続イベント 30 回。
- MTU は NM プロファイルに 1280 固定（実測: payload 1252 成功 /
  1472 失敗）。

### 2026-10-20/21 の事象

- 22:51–23:34 に 67 件の `tcp-failed`。すべて
  `gateway_reachable=true`、`internet_reachable=true`、
  `dns_ok=true` で、TCP だけが失敗した。
- 23:33:00 の `ip link set dev enp4s0 down` / `up` 後、
  23:34:26 に一時的に全疎通 OK。ただし 23:34:33 に再度
  TCP 失敗。23:34:57 にリンク断（ルーター電源断）。
- 00:11 にルーター再投入。00:11:50 に DHCP 失敗
  （`ip-config-unavailable`）、00:12:29 に成功し復旧。
- 実測: `1.1.1.1:443` への接続は `adguard-cli` 所有、
  `1.1.1.1:53` への接続は呼び出しプロセス所有（直接）。
- 解釈: NIC の再交渉では直らず、長時間のリンク断で復旧した。
  H1 は否定寄りで、H3（AdGuard CLI のプロキシ故障）が最有力。

### 2026-10-21 の確定（H3）

- 19:12–19:54 の `tcp-failed` はすべて `tcp=false`、
  `direct_tcp_ok=true`。直接 TCP（53）は正常で、AdGuard 経由の
  443 だけが失敗した。
- 障害時の実測:
  - 透過プロキシの待受 `127.0.0.1:42667` が
    `Recv-Q=129 / Send-Q=128`（上限 128）で飽和。
  - `CLOSE-WAIT` が 176〜177 件。
  - `TcpExtListenOverflows` / `TcpExtListenDrops` が
    54,715 → 55,386 と増加。
- 19:56:02 に `sudo systemctl restart adguard-cli` を実行。
  リダイレクト先は 34349 に変わり、待受キュー 0/128、
  `CLOSE-WAIT` 0、HTTPS は 200/204 で復旧した。
- 再起動後、watch は 19:54:38 の `ok` を最後に、20:12 時点まで
  新規レコードなし（状態変化なし）。`TcpExtListenOverflows` は
  55,393 で、再起動後の増加は 7 件のみ。直接 TCP は全期間で成功。
- 結論: AdGuard CLI の透過プロキシが `CLOSE-WAIT` を解放せず、
  accept queue が飽和して新規 SYN を落とす。ルーターの NAT
  枯渇や NIC ハングではない。
