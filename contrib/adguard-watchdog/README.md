# AdGuard CLI watchdog

## 目的

AdGuard CLI v1.4.13 の透過プロキシが `CLOSE-WAIT` を解放せず、
待受キューが飽和すると新規 TCP を落とす。本 watchdog は
待受キューと `CLOSE-WAIT` を監視し、閾値超過時に
`adguard-cli` を再起動して復旧する。

## 判定条件

1. `iptables -t nat -S AGCLI` からリダイレクト先ポートを取得する。
2. `ss -H -lnt "sport = :$PORT"` の `Recv-Q` と `Send-Q` を読む。
3. 次のいずれかで異常と判定する。
   - `Recv-Q >= Send-Q * QUEUE_THRESHOLD_PCT / 100`
   - `CLOSE-WAIT >= CLOSE_WAIT_THRESHOLD`
4. 2 秒後に再サンプルし、まだ異常なら `adguard-cli` を再起動する。
5. 再起動後 10 分はクールダウンする。
6. `adguard-cli` が inactive の場合は何もしない。

2026-10-21 の実測では `Recv-Q=129 / Send-Q=128`、
`CLOSE-WAIT=176`、`TcpExtListenDrops` の増加が発生し、
`systemctl restart adguard-cli` で復旧した。既定値は
このときの backlog 128 を前提に、75%（96）と
`CLOSE-WAIT=100` を用いる。

## 導入

```bash
sudo ./install.sh
```

`install.sh` は次を配置する。

- `/usr/local/sbin/adguard-watchdog.sh`
- `/etc/systemd/system/adguard-watchdog.service`
- `/etc/systemd/system/adguard-watchdog.timer`
- `/etc/default/adguard-watchdog`（既存なら保持）

タイマーは起動 2 分後と、前回起動の 30 秒後に実行される。

## 動作確認

```bash
systemctl list-timers adguard-watchdog.timer
sudo /usr/local/sbin/adguard-watchdog.sh --dry-run --verbose
journalctl -u adguard-watchdog.service -f
```

`--dry-run` は再起動せず判定だけを記録する。
`--verbose` は異常がない場合もサンプルを記録する。

## 設定

`/etc/default/adguard-watchdog` を編集し、
`systemctl restart adguard-watchdog.timer` で反映する。

| 変数 | 既定 | 意味 |
| --- | --- | --- |
| `QUEUE_THRESHOLD_PCT` | `75` | `Recv-Q` の閾値（`Send-Q` 比） |
| `CLOSE_WAIT_THRESHOLD` | `100` | `CLOSE-WAIT` の閾値 |
| `COOLDOWN_SECONDS` | `600` | 再起動の最短間隔 |
| `CONFIRM_DELAY` | `2` | 再サンプルまでの秒数 |
| `POST_RESTART_DELAY` | `2` | 再起動後の確認待ち |
| `DRY_RUN` | `0` | `1` で再起動しない |
| `VERBOSE` | `0` | `1` で全サンプルを記録 |

## アンインストール

```bash
sudo ./uninstall.sh
```

`/etc/default/adguard-watchdog` は保持される。不要なら削除する。

## 注意

- 再起動は数秒の通信断を伴う。`adguard-cli` の停止には
  最大 90 秒かかるため、service の `TimeoutStartSec` は 150 秒。
- 本 watchdog は回避策であり、AdGuard CLI の修正ではない。
- `adguard-cli` を意図的に停止している間は作動しない。
- しきい値は環境に合わせて調整する。
- `ss` と `iptables` の出力形式が変わった場合は判定式を見直す。

## テスト

```bash
./test.sh
```

ルート権限は不要。`iptables`、`ss`、`systemctl` を
シェル関数で置き換えて、判定と再起動の分岐を検証する。
