# ネットワーク不通時の切り分け手順（<router> / <lan-iface>）

対象: 本 PC（Kubuntu）からのみインターネット接続が失われる事象。
他機器（スマートフォン等）は同時に通信できている前提とする。

## 1. 症状

- PC からのみ通信不可。他機器は同時に通信可。
- <router> の LAN ランプは青のまま。セルフチェックは正常。
- REBOOT ボタン（1 秒押し）は効かないことが多い。
- ルーターの電源を 10 分以上切って再投入すると復旧する。

## 2. 判定したい仮説

| ID | 内容 | 復旧させる層 |
| --- | --- | --- |
| H1 | PC 側 NIC（<vendor-driver>）の内部ハング | PC 側でリンク再交渉 |
| H2 | <router> のハング（LAN ポート停止を含む） | ルーターの電源再投入 |
| H3 | AdGuard CLI の透過プロキシ故障 | プロキシの停止または再起動 |

初回の事象では、ICMP と DNS は通るのに新規 TCP だけが
失敗した。443 は AdGuard CLI の透過プロキシ経由なので、H3 を
最初に切り分ける。H3 は翌日に確定した（6 章参照）。

直近の事象は H2 で、PC 側のリンク断はその症状だった。
その間にも同型の約 40 分停止があり、H2 の再発と判断する。
H1 は直近の事象時点で未確定（6 章参照）。

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

H3 確定時の実測では、透過プロキシの待受キュー飽和と
`CLOSE-WAIT` の蓄積が原因で、再起動により即時復旧した。
再起動で直らない場合は、次の停止テストで切り分ける。

```bash
sudo systemctl stop adguard-cli
curl -4 -sS -m 5 -o /dev/null -w '%{http_code}\n' https://www.google.com
sudo systemctl start adguard-cli
```

443 が通るようになれば H3 確定。DNS フィルタは AdGuard Home が
継続するため、停止中も DNS レベルのブロックは維持される。

### 3.3 リンク層の確認と NIC の再交渉

まず L1（キャリア）を確認する。

```bash
ip -s link show <lan-iface>    # NO-CARRIER / operstate
ethtool <lan-iface> | grep -E 'Link detected|Speed|Duplex'
```

- `Link detected: no`（`NO-CARRIER`）: PC とルーター間の物理リンクが
  落ちている。再投入の前に、下の NIC 再交渉を一度だけ試す。
  復旧すれば H1、復旧しなければ H2 の可能性が高い。
  ケーブルとルーターの LAN ポートを確認し、ルーターの電源を
  再投入する（直近の事象では電源再投入で復旧した）。
- `Link detected: yes` で L3 が不通: 次の中継を行う。

```bash
sudo ip link set <lan-iface> down
sleep 2
sudo ip link set <lan-iface> up
sleep 10
```

疎通を確認する。

```bash
ping -c 3 192.0.2.1         # ゲートウェイ（L3 到達性）
ping -c 3 8.8.8.8               # 上位網（NAT 越え）
getent ahostsv4 www.google.com  # DNS 解決
```

- 復旧した場合: H1 の可能性が高い。`5.1` の判定条件を確認する。
- 復旧しない場合: H2 の可能性。ルーターの電源を 10 分以上
  切って再投入する。

## 4. 証跡の自動記録

`watch` を常駐させると、状態変化の瞬間の `ip` / `nmcli` /
`journalctl` / sysfs 出力が JSONL に残る。手動操作の前後を
比較できるため、常時起動しておく。TCP は `tcp_ok`（443、
AdGuard 経由）と `direct_tcp_ok`（53、直接）の 2 系統で記録される。

```bash
cargo build --release
cargo run --release -- watch --interface <lan-iface> --interval 5
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

heartbeat: 同じディレクトリの `net-watch.heartbeat`
（毎周期上書き。監視の生存確認はこの mtime で行う）

`watch` は `WatchdogSec=60` で systemd に keep-alive を送る。
`systemctl --user show -p WatchdogUSec -p NotifyAccess` で
`60000000` と `main` を確認できる。

JSONL は状態変化時のみ追記される。ログの mtime が止まっていても、
状態が変わっていなければ異常ではない。停止の疑いがある場合は
heartbeat の mtime と `systemctl --user status` を確認する。

## 5. 恒久対策

### 5.1 H1 の扱い（直近の事象時点で未確定）

`<vendor-driver>` の `Device reseting!` は、これまでサスペンド復帰時
（`PM: suspend exit` と同時）にのみ発生している。直近の
リンク断はルーターの電源再投入で復旧し、rx/tx エラーも 0 だった
ため H1 ではない。

H1 と判断する条件:

- ルーターの電源再投入でも復旧せず、PC 側のリンク再交渉や
  ドライバ再読み込みで復旧する。
- `Device reseting!` がサスペンドと無関係に発生する。
- rx_errors / rx_dropped / PCIe/AER エラーが増加する。

H1 と判断した場合のみ、out-of-tree の `<vendor-driver>` を撤去し、
カーネル同梱の `<in-tree-driver>` に戻す。

```bash
sudo apt purge <vendor-driver>-dkms
sudo reboot
ethtool -i <lan-iface>    # driver: <in-tree-driver> を確認
```

ロールバックする場合は `sudo apt install <vendor-driver>-dkms`。
移行後は `watch` で `carrier-down` と `Device reseting!` を観察する。

### 5.2 H3 確定後

AdGuard CLI v1.4.13 の透過プロキシは `CLOSE-WAIT` を解放せず、
待受キューが飽和すると新規 TCP を落とす。次のいずれかで回避する。

- 監視 watchdog（実装済み）: `contrib/adguard-watchdog/`。
  待受キューの `Recv-Q` または `CLOSE-WAIT` 件数が閾値を
  超えたら `systemctl restart adguard-cli` を実行する。
- 定期再起動: 1 日 1 回など、低頻度で再起動する。
- 上流修正: AdGuard CLI へバグ報告する。stable は v1.4.13 だが、
  v1.5.0-rc.2（2026-10-08）が公開済み。v1.5.0-nightly.10
  （2026-10-21）は #125 を修正済みと release notes に記載。
  #136（defunct root helper）は 2026-08-01 に「次の nightly で
  修正」とコメントされ、closed。v1.5.0-rc.1 以降に含まれる
  可能性が高い。ただし CLOSE_WAIT 漏れの修正は未確認。

導入手順としきい値は `contrib/adguard-watchdog/README.md` を参照。
未導入なら `sudo ./contrib/adguard-watchdog/install.sh` を実行する。

再起動でリダイレクト先ポートは変わる。watchdog は
`iptables -t nat -S AGCLI` から現在のポートを取得する。

### 5.3 上流・コミュニティ調査（2026-10-21）

（2026-10-24 追記: v1.5.0 stable が公開済み。判断は 5.4 を参照）

今回の `CLOSE-WAIT` 蓄積と accept queue 飽和について、
公開情報を「公式」「コミュニティ」「推測」に分けて整理した。

公式:

- stable は v1.4.13（2026-05-28）。v1.5.0 は rc.2
  （2026-10-08）で、stable は未リリース。
- v1.5.0-nightly.10（2026-10-21）の release notes は
  #125「Fedora stops recognizing internet access」を
  修正済みと記載。ただし #125 は open のまま
  （v1.6 ラベル、Code Review）。
- #125 は NetworkManager の port 80 検査が auto proxy で
  `SYN_SENT` のまま受理されない事象。今回の 443 の
  `CLOSE-WAIT` 蓄積とは同一ではない。
- #141（SOCKS5 の route loop）は v1.5 で修正済み。
  #136（defunct root helper）は 2026-08-01 に
  「次の nightly で修正」とコメントされ、closed。
  v1.5.0-rc.1 以降に含まれる可能性が高い。
- `CLOSE-WAIT` 蓄積を明記した AdGuard CLI の公開 issue は
  未発見。CoreLibs #2058 は Windows/Firefox の同種事象で
  修正済みだが、CLI/Linux の証拠ではない。

コミュニティ:

- Reddit 2024-11-06: `localhost:38521` に `CLOSE_WAIT` が
  数百件、時間とともに増加。コメント・解決策なし。
  本文に「VPN runs」とあり、AdGuard VPN CLI の可能性がある。
- Reddit 2026-09-01: Kubuntu + auto mode で接続が freeze。
  protection を無効化すると復帰。標準の `filtered_ports`。
- Reddit 2026-06-15/27: Fedora 等で auto mode にすると
  internet が完全に不通。manual では system-wide filtering が
  無効。解決策なし。
- Reddit 2026-08-23: NetworkManager の limited connectivity に
  対する port 80 / NetworkManager bypass の回避策。ただし
  #125 のコメントでは Ubuntu/CachyOS で効かないとの報告。
- AdGuard forum は 2023-10-13 に閉鎖（コミュニティ投稿による）。
  現在の公式コミュニティは Reddit / Telegram / GitHub。
  日本語圏の具体報告は今回の調査では未発見。AUR は 1.4.13-1 で、
  `CLOSE_WAIT` の報告はなし。

推測:

- 今回の事象は再起動で復旧するため表面化しにくく、公開報告が
  少ない。根本原因は未修正の可能性が高い。
- nightly.10 の #125 修正は NetworkManager 経路の修正であり、
  443 の `CLOSE-WAIT` 漏れを修正する証拠はない。
- 443 を `filtered_ports` から除外すれば 443 の透過経路は
  回避できるが、port 80 等の透過経路は残るため、根本修正では
  ない。

出典:

- AdGuardCLI releases:
  <https://github.com/AdguardTeam/AdGuardCLI/releases>
- #125: <https://github.com/AdguardTeam/AdGuardCLI/issues/125>
- #136: <https://github.com/AdguardTeam/AdGuardCLI/issues/136>
- #141: <https://github.com/AdguardTeam/AdGuardCLI/issues/141>
- CoreLibs #2058:
  <https://github.com/AdguardTeam/CoreLibs/issues/2058>
- Reddit CLOSE_WAIT:
  <https://www.reddit.com/r/Adguard/comments/1gl8e6w/issue_open_tcp_connections_with_adguardcli_linux/>
- Reddit Kubuntu:
  <https://www.reddit.com/r/Adguard/comments/1w457ep/help_with_adguardcli_on_linux_kubuntu/>
- Reddit Fedora:
  <https://www.reddit.com/r/Fedora/comments/1ugtefs/adguard_cli_for_linux_help/>
- Reddit limited connectivity:
  <https://www.reddit.com/r/Adguard/comments/1vwc9jw/fix_limited_connectivity_connected_site_only_in/>
- Official community: <https://adguard.com/discuss.html>

### 5.4 決定: v1.5.0 へ更新（2026-10-24）

2026-10-24 15:50 時点の `adguard-cli --version` は v1.4.13 だが、
CLI が `v1.5.0 is now available` を表示した。stable が公開された
ため、rc ではなく v1.5.0 stable へ更新する。

同時刻の実機証跡:

- `<root-helper>` が 2 日以上 `Z`（defunct）のまま残存。
  親の `adguard-cli`（PID <pid>）が reap していない。
- `journalctl -u adguard-cli` に
  `Failed to protect socket: Failed to send command to root helper`。
- これは #136（defunct root helper）と同じ機構で、v1.5.0 に
  修正が含まれる可能性が高い。

手順:

```bash
# 設定の退避
cp -a ~/.local/share/adguard-cli \
  ~/.local/share/adguard-cli.bak-$(date +%Y%m%d)
# 更新（root が必要）
sudo adguard-cli update
# watchdog のタイムアウトを 150 秒へ反映
sudo ./contrib/adguard-watchdog/install.sh
```

注意: `sudo adguard-cli update` は installer が `/opt/adguard-cli` の
所有者変更を `/dev/tty` で確認するため、非対話環境では失敗する。
`/opt/adguard-cli` がサービスユーザー `<user>` の所有であることを
確認し、`sudo` を付けずに `adguard-cli update` を実行する。
SUID helper は installer が直接上書きせず、`.new` として配置され、
再起動時に本体へ置き換わる。

更新後の確認:

```bash
adguard-cli --version
systemctl status adguard-cli --no-pager
MAIN=$(systemctl show -p MainPID --value adguard-cli)
ps -o pid,ppid,stat,comm --ppid "$MAIN"
journalctl -u adguard-cli --since '-10 min' | grep -i 'root helper'
```

判定:

- helper が `S`（sleep）で `Z` でないこと。
- `Failed to send command to root helper` が再出現しないこと。
- 1 時間ほど `watch` のログと
  `ss -tan state close-wait` を確認する。

再発した場合の次の手段:

1. `proxy.yaml` の `filtered_ports` から 443 を外す。
   例: `80:442,444:5221,5300:49151`（構文は実機で確認する）。
   HTTPS フィルタは無効になる。
2. `proxy_mode` を `manual` に落とす。system-wide の透過は無効。

実施結果（2026-10-24 16:10）:

- v1.5.0 へ更新。`/opt/adguard-cli` の `adguard-cli`、
  `adguard_root_helper`、`adguard_cli_nm`、`certutil` は
  ed25519 署名を検証済み。
- 再起動時に `adguard_root_helper.new` が本体へ置き換わり、
  `root:root` と SUID を維持。
- 再起動後、`Failed to send command to root helper` は 0 件、
  defunct helper もなし。
- `proxy.yaml` の変更は `detect_outbound_interface: true` の追加のみ。
  `filtered_ports` と `https_filtering` は不変。
- 実測: HTTPS 200、DNS 解決、TCP 443 すべて成功。
- 継続監視: `CLOSE_WAIT` 件数と `Route loop detected`
  （更新前から存在する 127.0.0.1:8000 宛ての警告）。

### 5.5 <router> の再発防止（2026-11-05）

直近 3 回の長いリンク断は、いずれも火曜の夜〜水曜未明に
発生した（第 1 回・第 2 回は火曜夜、第 3 回は水曜夜。各回の実時刻は記載しない）。
7 日周期で、直近の火曜夜は停止していない。
第 1 回と第 3 回はルーターの再起動で復旧し、第 2 回もリンク復帰後の
DHCP 失敗という同じ経過を示す。同時刻に NIC のエラー、PCIe/AER、
サスペンド復帰はない。

2026-11-05 時点の確認:

- ソフトウェアは <firmware>（公式の最新版）。
- 自動更新は有効。
- ファームウェア未更新は原因ではない。
- `newSoftware = 1` は、更新画面で
  「新しいバージョンはありません」なら古い表示として無視する。

<firmware> でも再発しているため、残る候補を次の順で疑う。

1. ルーター本体のソフトウェア不具合
   - 当時の証言: 以前は毎日固定時刻の自動再起動を設定して
     いたが、毎日ハングしたため無効にした。無効化後は週次の
     ハングに変わった。
   - 再起動処理そのものがハングの引き金で、自動再起動は使わない。
   - ランプは全青、ファンで冷却済み。熱・高温は除外する。
2. LAN ポートとケーブル
   - PC は 1G なので、2.5G ポートなら 1G ポートへ切替える。
   - CAT5e 以上の既知の良品ケーブルに交換する。
   - ハブを介さず直結する（回線事業者によってはハブ経由を保証しない）。
3. PC 側 NIC（<vendor-driver>）
   - 次回のリンク断時に `ip link set <lan-iface> down` / `up` を試す。
   - 復旧すれば H1。`<vendor-driver>-dkms` を外して `<in-tree-driver>` を試す。
4. ルーター本体の故障
   - 上記で改善しなければ 113 に連絡し、交換を依頼する。
   - `net-watch.jsonl` の時刻とランプの色を添える。

自動再起動は使わない:

- 過去に毎日固定時刻で有効化したところ、毎日ハングした。
- 再起動処理がハングを誘発する可能性がある。
- 停止を早めるだけで、原因対策にならない。

次回のリンク断では、電源を触る前に次を記録する。

- 設定ツールのホーム → 接続状態 → 接続時間。
  ルーターが自力で再起動していれば、停止時刻付近にリセットされる。
- 5 つのランプの色。全青なら高温ではない。
- スマートフォンの Wi-Fi が同時に使えるか。
  Wi-Fi も不通ならルーター全体、Wi-Fi が使えるなら LAN 側だけ。
- ルーターの管理画面が開くか。開かなければ本体がハングしている。
- セルフチェック → 診断履歴。過去の診断結果が残る。

設定ツールのログインパスワードが初期値のままなら変更する。

出典:

- ルーターソフトウェアアップデート情報（メーカー公式サポートページ）。
- ルーター取扱説明書（メーカー公開 PDF。型番を汎化したため URL は記載しない）。
- 回線事業者の公式お知らせ（日付・型番を汎化したため URL は記載しない）。

## 6. 参考（2026-10〜11 の実測）

- NIC: `<pci-id>` rev `<rev>`、ドライバ `<vendor-driver>` `<version>`（DKMS）。
- スリープ復帰のたびに `Device reseting!` とリンク再交渉が発生。
  5 日間で carrier 接続イベント 30 回。
- MTU は NM プロファイルに 1280 固定（実測: payload 1252 成功 /
  1472 失敗）。

### 初回の事象（2 日間にわたる TCP 失敗）

- 火曜夜に 67 件の `tcp-failed`。すべて
  `gateway_reachable=true`、`internet_reachable=true`、
  `dns_ok=true` で、TCP だけが失敗した。
- 夜半の `ip link set dev <lan-iface> down` / `up` 後、
  一時的に全疎通 OK。ただし直後に再度
  TCP 失敗。その後リンク断（ルーター電源断）。
- 約 40 分後にルーター再投入。再投入直後に DHCP 失敗
  （`ip-config-unavailable`）、その後成功し復旧。
- 実測: `1.1.1.1:443` への接続は `adguard-cli` 所有、
  `1.1.1.1:53` への接続は呼び出しプロセス所有（直接）。
- 解釈: NIC の再交渉では直らず、長時間のリンク断で復旧した。
  H1 は否定寄りで、H3（AdGuard CLI のプロキシ故障）が最有力。

### 翌日の確定（H3）

- 夕方の `tcp-failed` はすべて `tcp=false`、
  `direct_tcp_ok=true`。直接 TCP（53）は正常で、AdGuard 経由の
  443 だけが失敗した。
- 障害時の実測:
  - 透過プロキシの待受 `127.0.0.1:<listen-port>` が
    `Recv-Q=129 / Send-Q=128`（上限 128）で飽和。
  - `CLOSE-WAIT` が 176〜177 件。
  - `TcpExtListenOverflows` / `TcpExtListenDrops` が
    54,715 → 55,386 と増加。
- 直後に `sudo systemctl restart adguard-cli` を実行。
  リダイレクト先ポートは `<port>` に変わり、待受キュー 0/128、
  `CLOSE-WAIT` 0、HTTPS は 200/204 で復旧した。
- 再起動後、watch は再起動直前の `ok` を最後に、その後しばらく
  新規レコードなし（状態変化なし）。`TcpExtListenOverflows` は
  55,393 で、再起動後の増加は 7 件のみ。直接 TCP は全期間で成功。
- 結論: AdGuard CLI の透過プロキシが `CLOSE-WAIT` を解放せず、
  accept queue が飽和して新規 SYN を落とす。ルーターの NAT
  枯渇や NIC ハングではない。

### 同日夜の再発と root helper 異常

- 夜に watchdog が port `<port>` で
  `recv_q=113 / send_q=128 / close_wait=64` を検出。
  2 秒後の再サンプルでも `recv_q=105 / close_wait=65` のため、
  直後に `adguard-cli` を再起動した。
- 再起動直後に新プロセス（PID `<pid>`）が起動したが、
  子の `adguard_root_helper`（PID `<pid>`）が `defunct` になった。
- 再起動後、`AGCLI` チェーンが存在せず、watchdog は
  `iptables chain AGCLI not found; skip` を 30 秒ごとに記録。
- しばらくして `adguard-cli status` は
  「The AdGuard proxy server is not running」を返し、
  `AGCLI` チェーンも存在しない。直接 HTTPS は 200 で、
  インターネット自体は通るが、AdGuard のフィルタは停止している。
- これは #136（defunct root helper）と一致する。v1.4.13 は
  影響を受け、v1.5.0-rc.1 以降に修正が含まれる可能性が高い。
- 復旧: `sudo systemctl restart adguard-cli` の後、
  `adguard-cli status`、`sudo iptables -t nat -S AGCLI`、
  `ps -o pid,stat,cmd -C adguard_root_helper` を確認する。
  それでも `defunct` の場合は v1.5.0-rc.2 以降を検討する。
- 導入済み watchdog unit は `TimeoutStartSec=60` のまま。
  `sudo ./contrib/adguard-watchdog/install.sh` を再実行して
  150 秒へ更新する。

### 1 週間後の同型停止

- 火曜 <時刻1> に `<vendor-driver>: <nic>: link down`、<時刻2> に
  `link up`。約 40 分の停止。
- リンク復帰後、DHCP は直後に
  `ip-config-unavailable`、その後リースを取得した。
  直近の事象と同じ経過で、ルーター起動中と一致する。
- 同時刻に NIC のエラー、PCIe/AER、サスペンド復帰はない。
- 初回も約 36 分の停止で、
  ルーター再投入により復旧した（既存の記録）。

### 3 週間後のリンク断（ルーター側）

- 水曜夜に `<vendor-driver>: <lan-iface>: link down`。約 50 分後に `link up`。
  復帰はルーターの電源ケーブルを外して再投入した操作による。
- 前回の約 39 分停止と同型。週次の再発が疑われる。
- リンク断直後の watch は `health=down`、`reason=carrier-down`。
  `ip -s link` は `NO-CARRIER`、`operstate=down`、`speed=-1`。
  rx_errors / rx_dropped / collisions はすべて 0。
- リンク断直後の `ip neigh` は
  `192.0.2.1 ... INCOMPLETE`。<時刻3> には同じ MAC
  `<MAC>` で `REACHABLE`。ルーター機器自体は
  変わっておらず、再起動で復旧した。
- リンク断時にサスペンドはない。`Device reseting!` は
  直前数日のレジューム時のみで、いずれも `PM: suspend exit` と同時。
  PCIe/AER エラーもない。
- リンク復帰後、DHCP は一度失敗し、
  その後リースを取得した。初回のルーター
  再投入時と同じ経過。
- AdGuard は同時刻に root helper エラー 0 件、`CLOSE_WAIT` 0 件。
  H3 ではない。
- 確認時点で、ルーターは <firmware>（最新）かつ
  自動更新有効。`newSoftware = 1` は更新画面で再確認する。

追加証言:

- 5 つのランプは全青。ファンで冷却している。
- 以前は毎日固定時刻の自動再起動を設定していたが、
  毎日ハングしたため無効にした。
- 無効化後は、一定周期（週次）でハングする。
- 停止は火曜夜に集中している。次回の火曜夜を
  注意して観察する（仮説）。

判定の使い分け:

- `carrier-down` と kernel の `link down`: PC とルーター間の
  物理リンク。ケーブル、ルーターの電源・LAN ポートを疑う。
- `carrier-down` が 1 分以上続く場合は、サスペンド復帰ではなく
  H2 の再発として扱う。
- carrier が上がったまま `tcp-failed` で
  `direct_tcp_ok=true`: AdGuard CLI の透過プロキシ故障（H3）。
