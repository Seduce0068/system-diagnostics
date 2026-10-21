#!/bin/bash
# Unit tests for adguard-watchdog.sh. Run without root.

set -euo pipefail

src_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=adguard-watchdog.sh
source "$src_dir/adguard-watchdog.sh"

fail() {
    printf 'FAIL: %s\n' "$*" >&2
    exit 1
}

pass() {
    printf 'ok: %s\n' "$*"
}

ALLOW_NONROOT=1
VERBOSE=0
DRY_RUN=0

iptables() {
    printf '%s\n' \
        '-A AGCLI -p tcp -m multiport --dports 80:5221 -j REDIRECT --to-ports 34349' \
        '-A AGCLI -p tcp -m multiport --dports 5300:49151 -j REDIRECT --to-ports 34349' \
        '-A AGCLI -p tcp -m multiport --dports 9999 -j REDIRECT --to-ports 11111'
}

[[ $(detect_ports | tr '\n' ' ') == "11111 34349 " ]] \
    || fail "detect_ports"
pass "detect_ports"

violates 96 128 0 || fail "recv 96/128 should violate"
if violates 95 128 0; then
    fail "recv 95/128 should not violate"
fi
violates 0 128 100 || fail "close-wait 100 should violate"
if violates 0 128 99; then
    fail "close-wait 99 should not violate"
fi
pass "violates"

ss() {
    case "$*" in
        *"state close-wait"*)
            printf '%s\n' \
                'CLOSE-WAIT 0 0 127.0.0.1:34349 127.0.0.1:50000'
            ;;
        *)
            printf '%s\n' \
                'LISTEN 129 128 127.0.0.1:34349 0.0.0.0:*'
            ;;
    esac
}
[[ $(queue_sample 34349) == "129 128 1" ]] || fail "queue_sample"
pass "queue_sample"

ss() {
    case "$*" in
        *"state close-wait"*) return 0 ;;
    esac
    printf '%s\n' \
        'LISTEN 0 128 [::1]:34349 [::]:*' \
        'LISTEN 129 128 127.0.0.1:34349 0.0.0.0:*'
}
[[ $(queue_sample 34349) == "129 128 0" ]] \
    || fail "queue_sample should prefer IPv4"
pass "queue_sample IPv4 preference"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
restarts="$tmp/restarts"

STATE_DIR=$tmp
CONFIRM_DELAY=0
POST_RESTART_DELAY=0
COOLDOWN_SECONDS=600

systemctl() {
    case "$*" in
        "is-active --quiet adguard-cli") return 0 ;;
        "restart adguard-cli")
            printf 'restart\n' >>"$restarts"
            return 0
            ;;
        *) return 0 ;;
    esac
}

iptables() {
    printf '%s\n' \
        '-A AGCLI -p tcp -m multiport --dports 80:5221 -j REDIRECT --to-ports 34349'
}

date() {
    if [[ ${1:-} == "+%s" ]]; then
        printf '1000\n'
    else
        printf '2026-10-21T00:00:00+09:00\n'
    fi
}

ss() {
    case "$*" in
        *"state close-wait"*)
            printf '%s\n' \
                'CLOSE-WAIT 0 0 127.0.0.1:34349 127.0.0.1:50000'
            ;;
        *)
            printf '%s\n' \
                'LISTEN 129 128 127.0.0.1:34349 0.0.0.0:*'
            ;;
    esac
}

rm -f "$restarts"
( main ) >/dev/null 2>&1 || fail "main should succeed"
[[ -s $restarts ]] || fail "main should restart"
pass "main restart"

rm -f "$restarts"
ss() {
    case "$*" in
        *"state close-wait"*) return 0 ;;
    esac
    printf '%s\n' 'LISTEN 0 128 127.0.0.1:34349 0.0.0.0:*'
}
( main ) >/dev/null 2>&1 || fail "main healthy should succeed"
[[ ! -e $restarts ]] || fail "main healthy should not restart"
pass "main healthy"

rm -f "$restarts"
printf '1000\n' >"$tmp/adguard-watchdog.last-restart"
ss() {
    case "$*" in
        *"state close-wait"*)
            printf '%s\n' \
                'CLOSE-WAIT 0 0 127.0.0.1:34349 127.0.0.1:50000'
            ;;
        *)
            printf '%s\n' \
                'LISTEN 129 128 127.0.0.1:34349 0.0.0.0:*'
            ;;
    esac
}
( main ) >/dev/null 2>&1 || fail "main cooldown should succeed"
[[ ! -e $restarts ]] || fail "main cooldown should not restart"
pass "main cooldown"

rm -f "$restarts"
printf '0\n' >"$tmp/sample_count"
ss() {
    case "$*" in
        *"state close-wait"*) return 0 ;;
    esac
    count=$(<"$tmp/sample_count")
    if (( count == 0 )); then
        printf '%s\n' 'LISTEN 129 128 127.0.0.1:34349 0.0.0.0:*'
    else
        printf '%s\n' 'LISTEN 0 128 127.0.0.1:34349 0.0.0.0:*'
    fi
    printf '%s\n' "$((count + 1))" >"$tmp/sample_count"
}
( main ) >/dev/null 2>&1 || fail "main transient should succeed"
[[ ! -e $restarts ]] || fail "main transient should not restart"
pass "main transient"

printf 'all watchdog tests passed\n'
