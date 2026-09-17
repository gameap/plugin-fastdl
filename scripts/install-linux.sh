#!/bin/sh
set -eu
umask 077
url=$1
sha256=$2
install_dir=$3
config=$4
[ "$(id -u)" -eq 0 ] || { echo 'Run installation through a privileged GameAP Daemon on Linux.' >&2; exit 1; }
command -v curl >/dev/null
command -v sha256sum >/dev/null
command -v systemctl >/dev/null
case "$url" in https://*) ;; *) echo 'HTTPS download required' >&2; exit 1;; esac
case "$install_dir$config" in *'$'*|*'%'*|*'"'*|*\\*|*'
'*) echo 'Unsupported service path' >&2; exit 1;; esac
assert_no_symlinks() {
    checked=$1
    while [ "$checked" != / ] && [ "$checked" != . ]; do
        [ ! -L "$checked" ] || { echo 'Installation paths must not contain symbolic links' >&2; exit 1; }
        checked=$(dirname "$checked")
    done
}
for checked_path in "$install_dir" "$install_dir/servers.d" "$install_dir/cache" "$config" "$install_dir/gameap-fastdl" /etc/systemd/system/gameap-fastdl.service; do
    assert_no_symlinks "$checked_path"
done
[ -f "$config" ] || { echo 'A regular FastDL configuration file is required' >&2; exit 1; }
mkdir -p "$install_dir/servers.d" "$install_dir/cache"
chown root:root "$install_dir" "$install_dir/servers.d" "$install_dir/cache" "$config"
chmod 600 "$config"
chmod 700 "$install_dir" "$install_dir/servers.d" "$install_dir/cache"
staging=$(mktemp -d "$install_dir/install.XXXXXX")
trap 'rm -rf "$staging"' EXIT HUP INT TERM
curl --proto '=https' --proto-redir '=https' --fail --location --silent --show-error --max-time 600 --max-filesize 134217728 --output "$staging/gameap-fastdl" "$url"
printf '%s  %s\n' "$sha256" "$staging/gameap-fastdl" | sha256sum --check --status
chmod 700 "$staging/gameap-fastdl"
"$staging/gameap-fastdl" version
"$staging/gameap-fastdl" validate --config "$config"
unit=/etc/systemd/system/gameap-fastdl.service
binary="$install_dir/gameap-fastdl"
had_binary=false
had_unit=false
if [ -f "$binary" ]; then cp -p "$binary" "$staging/previous"; had_binary=true; fi
if [ -f "$unit" ]; then cp -p "$unit" "$staging/previous.service"; had_unit=true; fi
was_active=false
if systemctl is-active --quiet gameap-fastdl; then was_active=true; fi
rollback() {
    systemctl stop gameap-fastdl >/dev/null 2>&1 || true
    if [ "$had_binary" = true ]; then cp -p "$staging/previous" "$binary"; else rm -f "$binary"; fi
    if [ "$had_unit" = true ]; then
        cp -p "$staging/previous.service" "$unit"
    else
        systemctl disable gameap-fastdl >/dev/null 2>&1 || true
        rm -f "$unit"
    fi
    systemctl daemon-reload || true
    if [ "$was_active" = true ]; then systemctl start gameap-fastdl || true; fi
}
cat > "$staging/gameap-fastdl.service" <<SERVICE
[Unit]
Description=GameAP FastDL
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart="$binary" serve --config "$config"
Restart=on-failure
RestartSec=3
UMask=0077
NoNewPrivileges=true
PrivateTmp=true
PrivateDevices=true
ProtectSystem=strict
ProtectHome=read-only
ReadWritePaths="$install_dir/cache"
ProtectKernelTunables=true
ProtectKernelModules=true
ProtectControlGroups=true
RestrictSUIDSGID=true
RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX
LockPersonality=true
LimitNOFILE=8192

[Install]
WantedBy=multi-user.target
SERVICE
if ! systemctl stop gameap-fastdl 2>/dev/null; then :; fi
if ! mv "$staging/gameap-fastdl" "$binary" || ! install -m 644 "$staging/gameap-fastdl.service" "$unit" || ! systemctl daemon-reload || ! systemctl enable gameap-fastdl || ! systemctl start gameap-fastdl; then
    rollback
    exit 1
fi
sleep 2
if ! systemctl is-active --quiet gameap-fastdl; then rollback; exit 1; fi
"$binary" version
