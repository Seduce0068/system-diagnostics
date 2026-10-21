#!/bin/bash
# AdGuard CLI transparent proxy watchdog.
#
# Watches the redirect port used by the AGCLI iptables chain. When the
# accept queue is nearly full or CLOSE-WAIT sockets accumulate, the
# transparent proxy is no longer accepting new connections. Restart
# adguard-cli to clear the leaked sockets.
#
# Intended to run as root from adguard-watchdog.timer.

set -euo pipefail

SERVICE=${SERVICE:-adguard-cli}
CHAIN=${CHAIN:-AGCLI}
QUEUE_THRESHOLD_PCT=${QUEUE_THRESHOLD_PCT:-75}
CLOSE_WAIT_THRESHOLD=${CLOSE_WAIT_THRESHOLD:-100}
COOLDOWN_SECONDS=${COOLDOWN_SECONDS:-600}
CONFIRM_DELAY=${CONFIRM_DELAY:-2}
POST_RESTART_DELAY=${POST_RESTART_DELAY:-2}
STATE_DIR=${STATE_DIR:-/run}
DRY_RUN=${DRY_RUN:-0}
VERBOSE=${VERBOSE:-0}
ALLOW_NONROOT=${ALLOW_NONROOT:-0}

LOCK_FILE=""
STAMP_FILE=""

log() {
    printf '%s %s\n' "$(date -Is)" "$*"
}

usage() {
    cat <<'EOF'
Usage: adguard-watchdog.sh [--dry-run] [--verbose]

  --dry-run  Log decisions but do not restart adguard-cli.
  --verbose  Log every queue sample, not only violations.

Environment overrides:
  SERVICE, CHAIN, QUEUE_THRESHOLD_PCT, CLOSE_WAIT_THRESHOLD,
  COOLDOWN_SECONDS, CONFIRM_DELAY, POST_RESTART_DELAY, STATE_DIR,
  DRY_RUN, VERBOSE
EOF
}

parse_args() {
    while (($#)); do
        case "$1" in
            --dry-run) DRY_RUN=1 ;;
            --verbose) VERBOSE=1 ;;
            -h|--help) usage; exit 0 ;;
            *)
                printf 'unknown option: %s\n' "$1" >&2
                usage >&2
                exit 2
                ;;
        esac
        shift
    done
}

validate_config() {
    [[ -n $SERVICE ]] || SERVICE=adguard-cli
    [[ -n $CHAIN ]] || CHAIN=AGCLI
    [[ -n $STATE_DIR ]] || STATE_DIR=/run

    [[ $QUEUE_THRESHOLD_PCT =~ ^[0-9]+$ ]] || QUEUE_THRESHOLD_PCT=75
    (( QUEUE_THRESHOLD_PCT >= 1 && QUEUE_THRESHOLD_PCT <= 100 )) \
        || QUEUE_THRESHOLD_PCT=75

    [[ $CLOSE_WAIT_THRESHOLD =~ ^[0-9]+$ ]] || CLOSE_WAIT_THRESHOLD=100
    (( CLOSE_WAIT_THRESHOLD >= 1 )) || CLOSE_WAIT_THRESHOLD=100

    [[ $COOLDOWN_SECONDS =~ ^[0-9]+$ ]] || COOLDOWN_SECONDS=600
    [[ $CONFIRM_DELAY =~ ^[0-9]+$ ]] || CONFIRM_DELAY=2
    [[ $POST_RESTART_DELAY =~ ^[0-9]+$ ]] || POST_RESTART_DELAY=2

    [[ $DRY_RUN == 0 || $DRY_RUN == 1 ]] || DRY_RUN=0
    [[ $VERBOSE == 0 || $VERBOSE == 1 ]] || VERBOSE=0
    [[ $ALLOW_NONROOT == 0 || $ALLOW_NONROOT == 1 ]] || ALLOW_NONROOT=0
}

detect_ports() {
    local rules
    rules=$(iptables -w 5 -t nat -S "$CHAIN" 2>/dev/null) || return 1
    printf '%s\n' "$rules" \
        | awk '
            {
                for (i = 1; i < NF; i++) {
                    if ($i == "--to-ports") {
                        print $(i + 1)
                    }
                }
            }' \
        | tr ',' '\n' \
        | grep -E '^[0-9]+$' \
        | sort -n -u \
        || true
}

queue_sample() {
    local port=$1 line candidate recv send close_wait
    local -a lines=()
    local -a cw_lines=()

    mapfile -t lines < <(ss -H -lnt "sport = :$port" 2>/dev/null)
    if (( ${#lines[@]} == 0 )); then
        return 1
    fi
    line=${lines[0]}
    for candidate in "${lines[@]}"; do
        if [[ $candidate == *"127.0.0.1:$port"* ]]; then
            line=$candidate
            break
        fi
    done
    read -r _ recv send _ <<<"$line"
    [[ $recv =~ ^[0-9]+$ ]] || recv=0
    [[ $send =~ ^[0-9]+$ ]] || send=0

    mapfile -t cw_lines < <(
        ss -H -tan state close-wait "sport = :$port" 2>/dev/null
    )
    close_wait=${#cw_lines[@]}

    printf '%s %s %s\n' "$recv" "$send" "$close_wait"
}

violates() {
    local recv=$1 send=$2 close_wait=$3 threshold
    (( send > 0 )) || send=128
    threshold=$(( send * QUEUE_THRESHOLD_PCT / 100 ))
    (( threshold > 0 )) || threshold=1
    if (( recv >= threshold )); then
        return 0
    fi
    if (( close_wait >= CLOSE_WAIT_THRESHOLD )); then
        return 0
    fi
    return 1
}

cooldown_active() {
    local now last
    now=$(date +%s)
    [[ -f $STAMP_FILE ]] || return 1
    last=$(<"$STAMP_FILE")
    [[ $last =~ ^[0-9]+$ ]] || last=0
    (( now - last < COOLDOWN_SECONDS ))
}

sample_and_report() {
    local ports=$1 port sample recv send close_wait
    local found=0 listeners=0 violation=1

    while IFS= read -r port; do
        [[ -n $port ]] || continue
        found=1
        if ! sample=$(queue_sample "$port"); then
            log "port $port: listener not found"
            continue
        fi
        listeners=1
        read -r recv send close_wait <<<"$sample"
        if violates "$recv" "$send" "$close_wait"; then
            log "port $port: threshold exceeded" \
                "(recv_q=$recv send_q=$send close_wait=$close_wait)"
            violation=0
        elif (( VERBOSE )); then
            log "port $port: recv_q=$recv send_q=$send close_wait=$close_wait"
        fi
    done <<<"$ports"

    (( found )) || return 2
    (( listeners )) || return 2
    return "$violation"
}

restart_service() {
    local ports port sample recv send close_wait

    if (( DRY_RUN )); then
        log "dry-run: would restart $SERVICE"
        return 0
    fi

    log "restarting $SERVICE"
    date +%s >"$STAMP_FILE"
    if ! systemctl restart "$SERVICE"; then
        log "restart failed"
        return 1
    fi
    log "restart issued"

    sleep "$POST_RESTART_DELAY"
    ports=$(detect_ports) || ports=""
    if [[ -z $ports ]]; then
        log "AGCLI chain not found after restart"
        return 0
    fi

    while IFS= read -r port; do
        [[ -n $port ]] || continue
        if sample=$(queue_sample "$port"); then
            read -r recv send close_wait <<<"$sample"
            log "port $port after restart:" \
                "recv_q=$recv send_q=$send close_wait=$close_wait"
        else
            log "port $port after restart: listener not found"
        fi
    done <<<"$ports"
}

main() {
    local ports status

    parse_args "$@"
    validate_config

    if (( EUID != 0 && ALLOW_NONROOT != 1 )); then
        log "must be run as root (or set ALLOW_NONROOT=1 for tests)"
        exit 1
    fi

    LOCK_FILE="$STATE_DIR/adguard-watchdog.lock"
    STAMP_FILE="$STATE_DIR/adguard-watchdog.last-restart"
    mkdir -p "$STATE_DIR"
    exec 9>"$LOCK_FILE"
    if ! flock -n 9; then
        log "another instance is running"
        exit 0
    fi

    if ! systemctl is-active --quiet "$SERVICE"; then
        log "$SERVICE is not active; skip"
        exit 0
    fi

    if ! ports=$(detect_ports); then
        log "iptables chain $CHAIN not found; skip"
        exit 0
    fi
    if [[ -z $ports ]]; then
        log "no redirect port in $CHAIN; skip"
        exit 0
    fi

    status=0
    sample_and_report "$ports" || status=$?
    if (( status == 1 )); then
        exit 0
    fi
    if (( status == 2 )); then
        log "no listener for any redirect port; skip"
        exit 0
    fi

    log "threshold exceeded; confirming after ${CONFIRM_DELAY}s"
    sleep "$CONFIRM_DELAY"

    if ! ports=$(detect_ports); then
        log "AGCLI chain disappeared during confirmation; skip"
        exit 0
    fi

    status=0
    sample_and_report "$ports" || status=$?
    if (( status != 0 )); then
        log "condition cleared; no restart"
        exit 0
    fi

    if cooldown_active; then
        log "cooldown active; skip restart"
        exit 0
    fi

    restart_service
}

if [[ ${BASH_SOURCE[0]} == "$0" ]]; then
    main "$@"
fi
