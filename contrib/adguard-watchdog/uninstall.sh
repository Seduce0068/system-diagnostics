#!/bin/bash
# Remove the AdGuard CLI watchdog. Run as root.

set -euo pipefail

if (( EUID != 0 )); then
    printf 'run as root: sudo %s\n' "$0" >&2
    exit 1
fi

systemctl disable --now adguard-watchdog.timer 2>/dev/null || true
rm -f /etc/systemd/system/adguard-watchdog.service \
    /etc/systemd/system/adguard-watchdog.timer \
    /usr/local/sbin/adguard-watchdog.sh
systemctl daemon-reload

printf 'removed. kept /etc/default/adguard-watchdog\n'
