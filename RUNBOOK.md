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

## 3. 初動（ルーターに触る前に PC 側を再交渉する）

```bash
sudo ip link set enp4s0 down
sleep 2
sudo ip link set enp4s0 up
sleep 10
```

疎通を 3 層で確認する。

```bash
ping -c 3 192.168.128.1         # ゲートウェイ（L3 到達性）
ping -c 3 8.8.8.8               # 上位網（NAT 越え）
getent ahostsv4 www.google.com  # DNS 解決
```

- 復旧した場合: H1 確定。`5. 恒久対策` へ進む。
- 復旧しない場合: H2 の可能性。従来どおりルーターの電源を
  10 分以上切って再投入する。

## 4. 証跡の自動記録

`watch` を常駐させると、状態変化の瞬間の `ip` / `nmcli` /
`journalctl` / sysfs 出力が JSONL に残る。手動操作の前後を
比較できるため、常時起動しておく。

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

## 5. 恒久対策（H1 確定後）

out-of-tree の `r8168` を撤去し、カーネル同梱の `r8169` に戻す。

```bash
sudo apt purge r8168-dkms
sudo reboot
ethtool -i enp4s0    # driver: r8169 を確認
```

ロールバックする場合は `sudo apt install r8168-dkms`。

## 6. 参考（2026-10 時点の実測）

- NIC: `10ec:8168` rev 15、ドライバ `r8168` 8.055.00（DKMS）。
- スリープ復帰のたびに `Device reseting!` とリンク再交渉が発生。
  5 日間で carrier 接続イベント 30 回。
- MTU は NM プロファイルに 1280 固定（実測: payload 1252 成功 /
  1472 失敗）。
