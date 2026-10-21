#!/bin/bash
# Install the AdGuard CLI watchdog. Run as root.

set -euo pipefail

if (( EUID != 0 )); then
    printf 'run as root: sudo %s\n' "$0" >&2
    exit 1
fi

for cmd in iptables ss systemctl flock; do
    if ! command -v "$cmd" >/dev/null 2>&1; then
        printf 'missing command: %s\n' "$cmd" >&2
        exit 1
    fi
done

src_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)

install -d -m 0755 /usr/local/sbin
install -m 0755 "$src_dir/adguard-watchdog.sh" \
    /usr/local/sbin/adguard-watchdog.sh
install -m 0644 "$src_dir/adguard-watchdog.service" \
    /etc/systemd/system/adguard-watchdog.service
install -m 0644 "$src_dir/adguard-watchdog.timer" \
    /etc/systemd/system/adguard-watchdog.timer

if [[ ! -e /etc/default/adguard-watchdog ]]; then
    install -m 0644 "$src_dir/adguard-watchdog.default" \
        /etc/default/adguard-watchdog
fi

systemctl daemon-reload
systemctl enable adguard-watchdog.timer
systemctl restart adguard-watchdog.timer

systemctl list-timers adguard-watchdog.timer --no-pager
printf '\ninstalled. logs: journalctl -u adguard-watchdog.service -f\n'
