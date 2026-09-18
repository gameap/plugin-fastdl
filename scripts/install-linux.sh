#!/bin/bash

# GameAP FastDL installation script for Linux.
# Resolves the latest stable gameap-fastdl release, verifies its published
# SHA256 before it is ever executed, installs it into the private
# plugin directory and registers the systemd unit that serves FastDL content.
#
# Requires root and systemd. The unit is a system unit, and the plugin directory
# stays root-owned and closed to game-server accounts: the panel keeps writing
# config.json and servers.d/*.json into it through gameap-daemon, which only
# works when the daemon itself is privileged.
#
# --config must live in --install-dir: gameap-fastdl resolves a relative
# servers_dir and cache_dir against the configuration file's own directory, so
# the unit's ReadWritePaths would name a directory the service never writes to.
#
# Re-running the script updates an existing installation. The previous
# executable and unit are kept in the staging directory until the new service is
# confirmed running, and restored when it is not. A re-run that resolves to the
# same executable and the same unit leaves the running service alone.
#
# Invoked by the panel's FastDL plugin as a daemon task:
#   /bin/bash {node_work_path}/.plugins/i3z7ix336msd4/install-linux.sh \
#       --install-dir={node_work_path}/.plugins/i3z7ix336msd4 \
#       --config={node_work_path}/.plugins/i3z7ix336msd4/config.json

set -e
umask 077

COMPONENT="gameap-fastdl"
GITHUB_REPO="gameap/gameap-fastdl"

# The panel restarts the service by this name (services/node_setup.rs).
UNIT_NAME="gameap-fastdl"
UNIT_FILE="/etc/systemd/system/gameap-fastdl.service"
UNIT_DROPIN_DIR="/etc/systemd/system/gameap-fastdl.service.d"
SERVERS_SUBDIR="servers.d"
CACHE_SUBDIR="cache"

# The same cap the Windows installer enforces while streaming the response.
MAX_DOWNLOAD_BYTES="134217728"

# systemd calls a unit active as soon as the process is forked, which says
# nothing about a binary that dies on its listen address. Restart=on-failure
# with RestartSec=3 turns such a binary into a crash loop that looks healthy
# between restarts, so the start has to hold for SETTLE_SECS in a row and the
# restart counter has to stay where it was.
START_TIMEOUT_SECS="30"
SETTLE_SECS="5"

# Only staging directories older than this are swept, so a manual run started
# alongside a panel run cannot delete the other one's working directory.
STALE_STAGING_MINUTES="60"

DOWNLOAD_URL=""
SHA256=""
INSTALL_DIR=""
CONFIG=""
CHECK_ONLY=""

BINARY=""
STAGING=""
CHANGED=""
BINARY_REPLACED=""
UNIT_REPLACED=""
HAD_BINARY=""
HAD_UNIT=""
WAS_ACTIVE=""
WAS_ENABLED=""

show_help() {
    cat << EOF
GameAP FastDL installation script for Linux

Usage: $0 [OPTIONS]

Required:
    --install-dir=DIR       Private plugin directory, absolute; holds the
                            executable, ${SERVERS_SUBDIR}/ and ${CACHE_SUBDIR}/
    --config=FILE           FastDL configuration file, absolute, inside
                            --install-dir; the panel writes it before this script
                            runs

Other:
    --download-url=URL      Override the latest stable GitHub release with an
                            explicit HTTPS executable URL; requires --sha256
    --sha256=HEX            Expected SHA256 for --download-url (64 hex characters).
                            Without this pair, the release asset's matching
                            .sha256 sidecar is required
    --check                 Report the installed version and service state and
                            exit without changing anything (exit 1 when
                            unhealthy). The paths are read from the installed
                            unit when the options above are omitted
    --help                  Show this help

Re-running with the same --sha256 keeps the installed executable: the download
is skipped, and the service is restarted only when the executable or the unit
actually changed.
EOF
}

while [ $# -gt 0 ]; do
    case "$1" in
        --download-url=*)
            DOWNLOAD_URL="${1#*=}"
            ;;
        --sha256=*)
            SHA256="${1#*=}"
            ;;
        --install-dir=*)
            INSTALL_DIR="${1#*=}"
            ;;
        --config=*)
            CONFIG="${1#*=}"
            ;;
        --check)
            CHECK_ONLY="1"
            ;;
        --help|-h)
            show_help
            exit 0
            ;;
        *)
            echo "Unknown option: $1" >&2
            echo "Use --help for usage information" >&2
            exit 1
            ;;
    esac
    shift
done

# ---------------------------------------------------------------------------
# Option and path validation
#
# All of it runs before anything on the node is touched, so a rejected argument
# leaves the installation exactly as it was. The path rules are stricter than
# the filesystem's because both values are written into a systemd unit, where
# %, $ and \ are expanded and a quote or a newline ends the directive early.
# Rejecting those characters is done once, here; escaping them instead would
# have to be repeated everywhere a path is written or compared.

_assert_plain_abs_path() {
    case "$2" in
        /*) ;;
        *)
            echo "Error: $1 must be an absolute path, got '$2'" >&2
            exit 1
            ;;
    esac
    case "$2" in
        *\"*|*\\*|*\$*|*%*|*\`*|*\'*|*[[:cntrl:]]*)
            echo "Error: $1 must not contain quotes, backslashes, \$, % or backticks: '$2'" >&2
            exit 1
            ;;
    esac
}

# A symbolic link anywhere in the chain would let whoever controls it redirect
# a root-owned write outside the private directory.
_assert_no_symlinks() {
    local checked="$1"

    while [ "$checked" != / ]; do
        if [ -L "$checked" ]; then
            echo "Error: installation paths must not contain symbolic links: ${checked}" >&2
            exit 1
        fi
        checked="$(dirname "$checked")"
    done
}

_assert_https_url() {
    local authority

    case "$2" in
        https://*) ;;
        *)
            echo "Error: $1 must use HTTPS, got '$2'" >&2
            exit 1
            ;;
    esac

    # Credentials in the URL would be copied into the daemon task log.
    authority="${2#https://}"
    authority="${authority%%/*}"
    case "$authority" in
        *@*)
            echo "Error: $1 must not carry credentials" >&2
            exit 1
            ;;
    esac
}

_is_sha256() {
    [ "${#1}" -eq 64 ] && [ -z "$(printf '%s' "$1" | tr -d '0-9a-fA-F')" ]
}

_require_command() {
    if ! command -v "$1" > /dev/null 2>&1; then
        echo "Error: $1 is required but was not found" >&2
        exit 1
    fi
}

# Lowercased on both sides: a digest pasted from a build log is often uppercase,
# and a case difference here would look exactly like a corrupted download.
_sha256_of() {
    local sum

    if command -v sha256sum > /dev/null 2>&1; then
        sum="$(sha256sum "$1" | awk '{print $1}')"
    elif command -v openssl > /dev/null 2>&1; then
        sum="$(openssl dgst -sha256 "$1" | awk '{print $NF}')"
    elif command -v shasum > /dev/null 2>&1; then
        sum="$(shasum -a 256 "$1" | awk '{print $1}')"
    else
        return 1
    fi

    printf '%s' "$sum" | tr 'A-F' 'a-f'
}

_download_release_text() {
    local content

    if ! content="$(curl --proto '=https' --proto-redir '=https' --fail --location --silent --show-error \
        --max-redirs 5 --connect-timeout 15 --max-time 60 --max-filesize "$2" \
        --user-agent gameap-fastdl-installer "$1")"; then
        echo "Error: could not download release metadata or checksum from $1" >&2
        return 1
    fi
    if [ "${#content}" -gt "$2" ]; then
        echo "Error: release metadata or checksum exceeds its size limit" >&2
        return 1
    fi
    printf '%s' "$content"
}

_sidecar_sha256() {
    awk -v wanted="$1" '
        { sub(/\r$/, "") }
        NF {
            if (++lines != 1 || (NF != 1 && NF != 2)) exit 1
            digest = $1
            if (length(digest) != 64 || digest ~ /[^0-9a-fA-F]/) exit 1
            if (NF == 2 && $2 != wanted && $2 != "*" wanted) exit 1
        }
        END { if (lines != 1) exit 1; print tolower(digest) }
    '
}

resolve_release() {
    local architecture asset release_url tag checksum

    if [ "$(uname -s)" != Linux ]; then
        echo "Error: this installer only runs on Linux" >&2
        return 1
    fi
    case "$(uname -m)" in
        x86_64|amd64) architecture=amd64 ;;
        aarch64|arm64) architecture=arm64 ;;
        *) echo "Error: only amd64 and arm64 releases are available" >&2; return 1 ;;
    esac
    if ! release_url="$(curl --proto '=https' --proto-redir '=https' --fail --location --silent --show-error \
        --head --max-redirs 5 --connect-timeout 15 --max-time 60 --output /dev/null \
        --user-agent gameap-fastdl-installer --write-out '%{url_effective}' \
        "https://github.com/${GITHUB_REPO}/releases/latest")"; then
        echo "Error: could not resolve the latest stable ${COMPONENT} release" >&2
        return 1
    fi
    case "$release_url" in
        "https://github.com/${GITHUB_REPO}/releases/tag/"*)
            tag="${release_url##*/}" ;;
        *) echo "Error: no stable ${COMPONENT} release is published" >&2; return 1 ;;
    esac
    if [[ ! "$tag" =~ ^[A-Za-z0-9][A-Za-z0-9._+-]*$ ]] || [ "${#tag}" -gt 128 ] \
        || [ "$release_url" != "https://github.com/${GITHUB_REPO}/releases/tag/${tag}" ]; then
        echo "Error: the latest release has an invalid tag" >&2
        return 1
    fi
    asset="${COMPONENT}-${tag}-linux-${architecture}"
    DOWNLOAD_URL="https://github.com/${GITHUB_REPO}/releases/download/${tag}/${asset}"
    checksum="$(_download_release_text "${DOWNLOAD_URL}.sha256" 4096)" || return 1
    if ! SHA256="$(printf '%s\n' "$checksum" | _sidecar_sha256 "$asset")"; then
        echo "Error: invalid SHA256 sidecar for ${asset}" >&2
        return 1
    fi
    echo "Selected ${COMPONENT} ${tag} for linux/${architecture}."
}

validate_download_options() {
    if { [ -n "$DOWNLOAD_URL" ] && [ -z "$SHA256" ]; } \
        || { [ -z "$DOWNLOAD_URL" ] && [ -n "$SHA256" ]; }; then
        echo "Error: --download-url and --sha256 must be supplied together" >&2
        return 1
    fi
    if [ -n "$DOWNLOAD_URL" ]; then
        _assert_https_url --download-url "$DOWNLOAD_URL"
        if ! _is_sha256 "$SHA256"; then
            echo "Error: --sha256 must be 64 hexadecimal characters" >&2
            return 1
        fi
        SHA256="$(printf '%s' "$SHA256" | tr 'A-F' 'a-f')"
    fi
}

# ---------------------------------------------------------------------------
# The systemd unit
#
# One builder for the ExecStart line, so the installer and --check can never
# disagree about what a correct installation looks like. Both paths are
# double-quoted because a node work path may contain spaces; systemd removes
# the quotes and passes each as one argument.
#
# The hardening block is the deployment half of the service's threat model and
# every directive is load-bearing. The absence of User= is deliberate: the
# service reads game directories owned by the server accounts.

_exec_start() {
    echo "\"${BINARY}\" serve --config \"${CONFIG}\""
}

_write_unit() {
    cat > "$1" << EOF
[Unit]
Description=GameAP FastDL
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=$(_exec_start)
Restart=on-failure
RestartSec=3
UMask=0077
NoNewPrivileges=true
PrivateTmp=true
PrivateDevices=true
ProtectSystem=strict
ProtectHome=read-only
ReadWritePaths="${INSTALL_DIR}/${CACHE_SUBDIR}"
ProtectKernelTunables=true
ProtectKernelModules=true
ProtectControlGroups=true
RestrictSUIDSGID=true
RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX
LockPersonality=true
LimitNOFILE=8192
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
EOF
}

_unit_exec_start() {
    sed -n 's/^ExecStart=//p' "$1" 2>/dev/null | head -n 1
}

_unit_binary() {
    sed -n \
        -e 's/^ExecStart="\([^"]*\)".*/\1/p' \
        -e 's/^ExecStart=\([^"[:space:]][^[:space:]]*\).*/\1/p' "$1" 2>/dev/null | head -n 1
}

_unit_config() {
    sed -n 's/.*--config "\{0,1\}\([^"]*\)"\{0,1\}.*/\1/p' "$1" 2>/dev/null | head -n 1
}

_unit_enabled_state() {
    systemctl is-enabled "$UNIT_NAME" 2>/dev/null || true
}

_unit_active_state() {
    systemctl is-active "$UNIT_NAME" 2>/dev/null || true
}

# Empty on systemd older than v230, which is harmless: the baseline and the
# second read are then both empty and compare equal.
_unit_restarts() {
    local shown

    shown="$(systemctl show -p NRestarts "$UNIT_NAME" 2>/dev/null || true)"
    echo "${shown#*=}"
}

_installed_version() {
    "$1" version 2>/dev/null | head -n 1 || true
}

_file_state() {
    if [ -e "$1" ]; then
        echo "present"
    else
        echo "missing"
    fi
}

# ---------------------------------------------------------------------------
# Diagnostics
#
# Collected at the point of failure, never from inside the rollback: the
# rollback restores the previous unit and restarts the service, after which
# systemctl and journalctl describe the recovery instead of the fault.

_report_exec_failure() {
    echo "Error: ${2} could not be executed (exit ${1})." >&2

    case "$1" in
        126)
            echo "  The file is not executable on this node. Check that the download matches its architecture ($(uname -m)) and that the filesystem is not mounted noexec." >&2
            ;;
        127)
            echo "  A shared library the executable needs is missing." >&2
            ;;
    esac
}

_report_unit_failure() {
    echo "--- systemctl status ${UNIT_NAME} ---" >&2
    systemctl status "$UNIT_NAME" --no-pager --full -n 30 2>&1 | tail -n 40 >&2 || true

    if command -v journalctl > /dev/null 2>&1; then
        echo "--- journalctl -u ${UNIT_NAME} -n 50 ---" >&2
        journalctl -u "$UNIT_NAME" -n 50 --no-pager 2>&1 | tail -n 50 >&2 || true
    fi

    echo "  A unit that fails with status=226/NAMESPACE is usually a container in which the hardening directives cannot be applied." >&2

    if command -v getenforce > /dev/null 2>&1 && [ "$(getenforce 2>/dev/null || true)" = "Enforcing" ]; then
        echo "  SELinux is enforcing. A binary outside the standard directories may need: semanage fcontext -a -t bin_t '${BINARY}' && restorecon -v '${BINARY}'" >&2
    fi
}

_systemctl_or_die() {
    if ! systemctl "$@"; then
        echo "Error: systemctl $* failed." >&2
        _report_unit_failure
        exit 1
    fi
}

# ---------------------------------------------------------------------------
# Rollback
#
# A failed upgrade has to leave the node on the executable and unit it had.
# Every step tolerates failure: under set -e a failing command inside the EXIT
# trap would abandon the rest of the restore and replace the exit status that
# tells the panel what went wrong.
#
# The executable is restored with mv, not cp: cp writes into the inode of a
# file that may still be running and fails with ETXTBSY. A rename cannot, and
# it is atomic because the staging directory is created inside --install-dir,
# on the same filesystem.

_rollback() {
    local restored_state

    echo "Restoring the previous ${COMPONENT}..." >&2

    if [ -n "$BINARY_REPLACED" ]; then
        if [ -n "$HAD_BINARY" ]; then
            mv -f "${STAGING}/previous" "$BINARY" 2>/dev/null || true
        else
            rm -f "$BINARY" 2>/dev/null || true
        fi
    fi

    if [ -n "$UNIT_REPLACED" ]; then
        if [ -n "$HAD_UNIT" ]; then
            cp -p "${STAGING}/previous.service" "$UNIT_FILE" 2>/dev/null || true
            systemctl daemon-reload > /dev/null 2>&1 || true
            if [ "$WAS_ENABLED" = "enabled-runtime" ]; then
                systemctl disable "$UNIT_NAME" > /dev/null 2>&1 || true
                systemctl enable --runtime "$UNIT_NAME" > /dev/null 2>&1 || true
            elif [ -n "$WAS_ENABLED" ]; then
                systemctl enable "$UNIT_NAME" > /dev/null 2>&1 || true
            else
                systemctl disable "$UNIT_NAME" > /dev/null 2>&1 || true
            fi
        else
            systemctl disable "$UNIT_NAME" > /dev/null 2>&1 || true
            rm -f "$UNIT_FILE" 2>/dev/null || true
            systemctl daemon-reload > /dev/null 2>&1 || true
        fi
    fi

    # A crash loop trips StartLimitBurst within seconds, after which every
    # start is refused with "start request repeated too quickly" - including
    # this one, which would leave the node down with a healthy executable.
    systemctl reset-failed "$UNIT_NAME" > /dev/null 2>&1 || true

    if [ -n "$HAD_UNIT" ] && [ -n "$WAS_ACTIVE" ]; then
        systemctl restart "$UNIT_NAME" > /dev/null 2>&1 || true

        restored_state="$(_unit_active_state)"
        if [ "$restored_state" != "active" ]; then
            echo "Warning: the previous ${COMPONENT} service could not be restarted (state: ${restored_state:-unknown}); this node is left with FastDL stopped." >&2
        fi
    else
        systemctl stop "$UNIT_NAME" > /dev/null 2>&1 || true
    fi
}

# One trap for the staging directory and the rollback, installed before the
# directory exists: set -e can end the run at any step in between. The rollback
# restores out of the staging directory, so the removal comes after it.
#
# The signal traps exit instead of returning, which is what lets an interrupted
# run reach this trap at all. Disarming the traps first keeps a second signal
# during the rollback from re-entering it.
_cleanup() {
    local status=$?

    trap - EXIT INT TERM HUP

    if [ "$status" -ne 0 ] && [ -n "$CHANGED" ]; then
        _rollback
    fi

    if [ -n "$STAGING" ]; then
        rm -rf "$STAGING" 2>/dev/null || true
    fi

    exit "$status"
}

# The daemon cancels a task with SIGKILL, so an abandoned run leaves its staging
# directory behind with a copy of the previous executable in it. -type d is what
# keeps the glob off this script's own file name.
_sweep_stale_staging() {
    find "$INSTALL_DIR" -maxdepth 1 -type d -name 'install.*' \
        -mmin "+${STALE_STAGING_MINUTES}" -exec rm -rf {} + 2>/dev/null || true
}

# ---------------------------------------------------------------------------
# Reporting

report_status() {
    local binary config version enabled active healthy exec_start expected

    binary="$BINARY"
    config="$CONFIG"

    if [ -f "$UNIT_FILE" ]; then
        [ -n "$binary" ] || binary="$(_unit_binary "$UNIT_FILE")"
        [ -n "$config" ] || config="$(_unit_config "$UNIT_FILE")"
        enabled="$(_unit_enabled_state)"
        active="$(_unit_active_state)"
        exec_start="$(_unit_exec_start "$UNIT_FILE")"
    else
        enabled="not installed"
        active="not installed"
        exec_start=""
    fi

    healthy="1"
    [ "$active" = "active" ] || healthy=""

    version=""
    if [ -n "$binary" ] && [ -x "$binary" ]; then
        version="$(_installed_version "$binary")"
    else
        healthy=""
    fi

    echo "${version:-${COMPONENT} is not installed}"
    echo "  binary:    ${binary:-unknown} ($(_file_state "${binary:-/nonexistent}"))"
    echo "  config:    ${config:-unknown} ($(_file_state "${config:-/nonexistent}"))"
    if [ -n "$config" ]; then
        echo "  servers:   $(dirname "$config")/${SERVERS_SUBDIR} ($(_file_state "$(dirname "$config")/${SERVERS_SUBDIR}"))"
        echo "  cache:     $(dirname "$config")/${CACHE_SUBDIR} ($(_file_state "$(dirname "$config")/${CACHE_SUBDIR}"))"
    fi
    echo "  unit:      ${UNIT_FILE} (${enabled:-unknown}, ${active:-unknown})"

    if [ -n "$BINARY" ] && [ -n "$CONFIG" ] && [ -n "$exec_start" ]; then
        expected="$(_exec_start)"
        if [ "$exec_start" != "$expected" ]; then
            echo "             ExecStart is ${exec_start}; re-run the installer to point it at ${expected}"
            healthy=""
        fi
    fi

    if [ -d "$UNIT_DROPIN_DIR" ]; then
        echo "  drop-ins:  ${UNIT_DROPIN_DIR} exists and may override the settings above"
    fi

    [ -n "$healthy" ]
}

# ---------------------------------------------------------------------------
# Pre-flight
#
# Everything that can be rejected before a byte is downloaded or written.

if [ -n "$CHECK_ONLY" ]; then
    if [ -n "$INSTALL_DIR" ]; then
        INSTALL_DIR="${INSTALL_DIR%/}"
        _assert_plain_abs_path --install-dir "$INSTALL_DIR"
        BINARY="${INSTALL_DIR}/${COMPONENT}"
    fi
    [ -z "$CONFIG" ] || _assert_plain_abs_path --config "$CONFIG"

    _require_command systemctl

    report_status || exit 1
    exit 0
fi

if [ -z "$INSTALL_DIR" ] || [ -z "$CONFIG" ]; then
    echo "Error: --install-dir and --config are required" >&2
    echo "Use --help for usage information" >&2
    exit 1
fi

validate_download_options || exit 1

INSTALL_DIR="${INSTALL_DIR%/}"
if [ -z "$INSTALL_DIR" ]; then
    echo "Error: --install-dir must not be the filesystem root" >&2
    exit 1
fi

_assert_plain_abs_path --install-dir "$INSTALL_DIR"
_assert_plain_abs_path --config "$CONFIG"

if [ "$(dirname "$CONFIG")" != "$INSTALL_DIR" ]; then
    echo "Error: --config must live directly in --install-dir, got '${CONFIG}'" >&2
    echo "  ${COMPONENT} resolves ${SERVERS_SUBDIR} and ${CACHE_SUBDIR} against the configuration file's own directory, so elsewhere the service would use directories this installation does not prepare." >&2
    exit 1
fi

if [ "$(id -u)" -ne 0 ]; then
    echo "Error: run the installation through a privileged GameAP Daemon; root is required to install a system service" >&2
    exit 1
fi

_require_command systemctl
if [ ! -d /run/systemd/system ]; then
    echo "Error: systemd is not running on this node; the FastDL service cannot be installed" >&2
    exit 1
fi

# Masking replaces the unit file with a link to /dev/null, which the symlink
# check below would report as a tampered path.
if [ "$(_unit_enabled_state)" = "masked" ]; then
    echo "Error: ${UNIT_NAME} is masked on this node; run 'systemctl unmask ${UNIT_NAME}' before installing" >&2
    exit 1
fi

for required_command in curl install mktemp chown dirname find sed awk; do
    _require_command "$required_command"
done

if ! command -v sha256sum > /dev/null 2>&1 \
    && ! command -v openssl > /dev/null 2>&1 \
    && ! command -v shasum > /dev/null 2>&1; then
    echo "Error: none of sha256sum, openssl or shasum is available; the download cannot be verified" >&2
    exit 1
fi

if [ ! -f "$CONFIG" ]; then
    echo "Error: a regular FastDL configuration file is required at ${CONFIG}" >&2
    exit 1
fi

BINARY="${INSTALL_DIR}/${COMPONENT}"

for checked_path in \
    "$INSTALL_DIR" \
    "${INSTALL_DIR}/${SERVERS_SUBDIR}" \
    "${INSTALL_DIR}/${CACHE_SUBDIR}" \
    "$CONFIG" \
    "$BINARY" \
    "$UNIT_FILE"
do
    _assert_no_symlinks "$checked_path"
done

if [ -z "$DOWNLOAD_URL" ]; then
    resolve_release || exit 1
fi

# ---------------------------------------------------------------------------
# Installation
#
# The order keeps the running service on its old executable until the very last
# step: a failure while writing the unit or reloading systemd never takes FastDL
# down, and replacing the executable is a rename, not a truncating write.

echo "Preparing ${INSTALL_DIR}..."

mkdir -p "${INSTALL_DIR}/${SERVERS_SUBDIR}" "${INSTALL_DIR}/${CACHE_SUBDIR}"
chown root:root "$INSTALL_DIR" "${INSTALL_DIR}/${SERVERS_SUBDIR}" "${INSTALL_DIR}/${CACHE_SUBDIR}" "$CONFIG"
chmod 600 "$CONFIG"
chmod 700 "$INSTALL_DIR" "${INSTALL_DIR}/${SERVERS_SUBDIR}" "${INSTALL_DIR}/${CACHE_SUBDIR}"

_sweep_stale_staging

trap _cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM HUP

if ! STAGING="$(mktemp -d "${INSTALL_DIR}/install.XXXXXX")"; then
    echo "Error: could not create a staging directory in ${INSTALL_DIR}" >&2
    exit 1
fi

[ ! -f "$BINARY" ] || HAD_BINARY="1"
[ ! -f "$UNIT_FILE" ] || HAD_UNIT="1"
[ "$(_unit_active_state)" != "active" ] || WAS_ACTIVE="1"
case "$(_unit_enabled_state)" in
    enabled-runtime) WAS_ENABLED="enabled-runtime" ;;
    enabled) WAS_ENABLED="1" ;;
esac

STAGED="${STAGING}/${COMPONENT}"
RUN_BINARY="$STAGED"

if [ -n "$HAD_BINARY" ] && [ "$(_sha256_of "$BINARY")" = "$SHA256" ]; then
    echo "The installed executable already matches the requested checksum, keeping it."
    RUN_BINARY="$BINARY"
else
    echo "Downloading ${COMPONENT}..."
    curl --proto '=https' --proto-redir '=https' --fail --location --silent --show-error \
        --max-time 600 --max-filesize "$MAX_DOWNLOAD_BYTES" \
        --output "$STAGED" "$DOWNLOAD_URL"

    # --max-filesize only fires when the response declares a Content-Length, so
    # a chunked response walks straight past it.
    downloaded_bytes="$(wc -c < "$STAGED" | tr -d '[:space:]')"
    if [ "$downloaded_bytes" -gt "$MAX_DOWNLOAD_BYTES" ]; then
        echo "Error: the download is ${downloaded_bytes} bytes, over the ${MAX_DOWNLOAD_BYTES} byte limit" >&2
        exit 1
    fi

    actual_sha256="$(_sha256_of "$STAGED")"
    if [ "$actual_sha256" != "$SHA256" ]; then
        echo "Error: SHA256 verification failed for ${DOWNLOAD_URL}" >&2
        echo "  expected: ${SHA256}" >&2
        echo "  actual:   ${actual_sha256}" >&2
        exit 1
    fi

    echo "Checksum verified."
    chmod 700 "$STAGED"
fi

# The digest is what makes the file safe to run at all, so nothing above this
# point has executed it and nothing below runs before the comparison passed.
echo "Verifying the executable and the configuration..."

version_status=0
"$RUN_BINARY" version || version_status=$?
if [ "$version_status" -ne 0 ]; then
    _report_exec_failure "$version_status" "$RUN_BINARY"
    exit 1
fi

if ! "$RUN_BINARY" validate --config "$CONFIG"; then
    echo "Error: the FastDL configuration at ${CONFIG} is not valid; nothing was changed." >&2
    echo "  Validation also rejects ${SERVERS_SUBDIR} entries whose game directory no longer exists, which the running service would merely skip." >&2
    echo "  Fix or remove the files named above in ${INSTALL_DIR}/${SERVERS_SUBDIR} and install again." >&2
    exit 1
fi

_write_unit "${STAGING}/${UNIT_NAME}.service"

if [ -z "$HAD_UNIT" ] \
    || [ "$(cat "${STAGING}/${UNIT_NAME}.service")" != "$(cat "$UNIT_FILE")" ]; then
    echo "Installing the systemd unit ${UNIT_FILE}..."

    [ -z "$HAD_UNIT" ] || cp -p "$UNIT_FILE" "${STAGING}/previous.service"
    CHANGED="1"
    UNIT_REPLACED="1"
    install -m 0644 "${STAGING}/${UNIT_NAME}.service" "$UNIT_FILE"

    # A unit file written with an unexpected SELinux context is loaded but
    # refused at start, with no indication of why.
    if command -v restorecon > /dev/null 2>&1; then
        restorecon -F "$UNIT_FILE" > /dev/null 2>&1 || true
    fi
fi

_systemctl_or_die daemon-reload

if [ "$RUN_BINARY" != "$BINARY" ]; then
    echo "Installing ${BINARY}..."

    [ -z "$HAD_BINARY" ] || cp -p "$BINARY" "${STAGING}/previous"
    CHANGED="1"
    BINARY_REPLACED="1"
    mv -f "$STAGED" "$BINARY"
fi

chown root:root "$BINARY"
chmod 700 "$BINARY"

_systemctl_or_die enable "$UNIT_NAME"

if [ -z "$BINARY_REPLACED" ] && [ -z "$UNIT_REPLACED" ] && [ -n "$WAS_ACTIVE" ]; then
    echo
    echo "$(_installed_version "$BINARY") is already installed and running; the service was left alone."
    echo "  binary:    ${BINARY}"
    echo "  config:    ${CONFIG}"
    echo "  unit:      ${UNIT_FILE} (enabled, active)"
    exit 0
fi

echo "Restarting ${UNIT_NAME}..."

CHANGED="1"
systemctl reset-failed "$UNIT_NAME" > /dev/null 2>&1 || true
restart_baseline="$(_unit_restarts)"
_systemctl_or_die restart "$UNIT_NAME"

settled=0
waited=0

while [ "$waited" -lt "$START_TIMEOUT_SECS" ]; do
    if [ "$(_unit_active_state)" = "active" ]; then
        settled=$((settled + 1))
        [ "$settled" -le "$SETTLE_SECS" ] || break
    else
        settled=0
    fi
    sleep 1
    waited=$((waited + 1))
done

if [ "$settled" -le "$SETTLE_SECS" ]; then
    echo "Error: ${UNIT_NAME} did not stay active for ${SETTLE_SECS}s within ${START_TIMEOUT_SECS}s (state: $(_unit_active_state), restarts: $(_unit_restarts))." >&2
    _report_unit_failure
    exit 1
fi

# An executable that dies a little later is restarted before the next probe and
# would read as healthy; the restart counter is what gives it away.
if [ "$(_unit_restarts)" != "$restart_baseline" ]; then
    echo "Error: ${UNIT_NAME} restarted during its first ${SETTLE_SECS}s and is not stable." >&2
    _report_unit_failure
    exit 1
fi

echo
echo "$(_installed_version "$BINARY") installed successfully."
echo "  binary:    ${BINARY}"
echo "  config:    ${CONFIG}"
echo "  unit:      ${UNIT_FILE} (enabled, active)"
