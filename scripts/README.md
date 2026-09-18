# FastDL node installers

Installers for `gameap-fastdl`, the node-side Go service the FastDL plugin
manages. One per supported node platform:

| Script                | Platform             | Service                     |
|-----------------------|----------------------|-----------------------------|
| `install-linux.sh`    | Linux, root, systemd | `gameap-fastdl.service`     |
| `install-windows.ps1` | Windows, elevated    | `gameap-fastdl` SCM service |

## How they reach a node

Both scripts are bundled into `fastdl.wasm` at build time. The plugin selects the
script for the node's OS, uploads it through the node file API with mode `0700`
into `.plugins/i3z7ix336msd4`, and creates one daemon task to run it. The script, binary
and configuration live in this private directory below the daemon work path.

Each installation replaces the private script with the version bundled in the
plugin. Old `tools/install-linux.sh` and `tools/install-windows.ps1` files are no
longer used, so a stale copy in the tools directory cannot affect installation.

Linux task:

```text
/bin/bash '{node_work_path}/.plugins/i3z7ix336msd4/install-linux.sh' \
    '--install-dir={node_work_path}/.plugins/i3z7ix336msd4' \
    '--config={node_work_path}/.plugins/i3z7ix336msd4/config.json'
```

Windows task (the command is one line):

```text
powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "{node_work_path}\.plugins\i3z7ix336msd4\install-windows.ps1" -InstallDir "{node_work_path}\.plugins\i3z7ix336msd4" -ConfigPath "{node_work_path}\.plugins\i3z7ix336msd4\config.json"
```

The examples use `{node_work_path}` to stand for the node's configured work
directory. The plugin resolves the installer, install directory and configuration
to validated absolute paths and quotes them for the node's platform, including
paths containing spaces.

Rebuild and deploy `fastdl.wasm` after changing a script: the plugin and its
installers are shipped together. The tests in `src/handlers/tests.rs` check the
uploaded script contents and the installer command for each platform.

## Automatic releases

With no download options, the script resolves GitHub's latest stable release of
[`gameap/gameap-fastdl`](https://github.com/gameap/gameap-fastdl/releases), detects
the node's native architecture, then fetches the binary and checksum from that
specific tag. This keeps the two downloads together even if another release is
published during installation. No extra JSON parser is required on the node.

Publish these assets for each supported target. The filename includes the exact
release tag, shown here as `v0.0.1`:

| Target        | Binary asset                             | Checksum asset                                  |
|---------------|------------------------------------------|-------------------------------------------------|
| Linux amd64   | `gameap-fastdl-v0.0.1-linux-amd64`       | `gameap-fastdl-v0.0.1-linux-amd64.sha256`       |
| Linux arm64   | `gameap-fastdl-v0.0.1-linux-arm64`       | `gameap-fastdl-v0.0.1-linux-arm64.sha256`       |
| Windows amd64 | `gameap-fastdl-v0.0.1-windows-amd64.exe` | `gameap-fastdl-v0.0.1-windows-amd64.exe.sha256` |
| Windows arm64 | `gameap-fastdl-v0.0.1-windows-arm64.exe` | `gameap-fastdl-v0.0.1-windows-arm64.exe.sha256` |

Each `.sha256` file contains one 64-digit hexadecimal checksum, optionally followed
by its exact binary filename in `sha256sum` format. Generate it from the trusted
build output, for example from its `dist` directory:

```sh
sha256sum gameap-fastdl-v0.0.1-linux-amd64 > gameap-fastdl-v0.0.1-linux-amd64.sha256
```

A stable release and its binary/checksum assets must exist before automatic
installation can succeed. Unsupported architectures, missing releases/assets,
invalid checksum files, and checksum mismatches fail with a diagnostic in the
daemon task output. Existing installations remain available when release
resolution or verification fails.

For custom builds, pass both `--download-url=URL` and `--sha256=HEX` on Linux, or
`-DownloadUrl URL -Sha256 HEX` on Windows. These bypass release discovery and
retain the same HTTPS and checksum validation. Passing only one is rejected.
The plugin's setup API retains this optional pair for existing integrations;
the installation form uses automatic selection.

## What each step is for

1. **Validate arguments.** Absolute paths only, no quotes, `%` or control
   characters: both paths are written into a systemd `ExecStart=` line or an SCM
   `ImagePath`, where `%` is expanded at service start. `--config` has to live in
   `--install-dir` because `gameap-fastdl` resolves a relative `servers_dir` and
   `cache_dir` against the configuration file's own directory.
2. **Reject unsafe paths.** A symbolic link or reparse point anywhere in the path
   chain would redirect a privileged write out of the private directory.
3. **Download over HTTPS only.** Redirects are re-checked for scheme and
   credentials at every hop, and the size cap is enforced while reading, not just
   from `Content-Length`.
4. **Verify SHA256 before executing anything.** The expected digest comes from
   the release checksum asset or the explicit custom-build option; nothing runs
   the download before it matches.
5. **Validate the configuration** with `gameap-fastdl validate --config`, before
   the service definition is touched, so a bad configuration is one readable
   message instead of a restart loop.
6. **Install, then start.** The running service keeps its old executable until
   the last step: on Linux the swap is a rename inside the same directory, on
   Windows the service is stopped and the file is waited out until it unlocks.
7. **Confirm the service stays up.** Both installers poll to a deadline and then
   require the service to hold for five seconds, because
   `Restart=on-failure` / SCM recovery makes a crash loop look healthy between
   restarts.
8. **Roll back on any failure after the first change.** The previous executable
   and service definition are restored and the service is put back the way it
   was. A failed rollback says so loudly rather than leaving a node silently down.

Re-running with the same resolved or supplied SHA256 skips the binary download and, when the service
definition also matches and the service is running, changes nothing at all — so
a re-run is a safe way to repair a damaged service without an outage.

## Checking a node

Both scripts report an installation without changing it, and exit 1 when it is
unhealthy. The paths are read from the installed service when they are omitted:

```
/bin/bash /srv/gameap/.plugins/i3z7ix336msd4/install-linux.sh --check
powershell -NoProfile -ExecutionPolicy Bypass -File "C:\gameap\.plugins\i3z7ix336msd4\install-windows.ps1" -Check
```

## Testing locally

Run `make test-scripts` for release-resolution tests with mocked downloads. These
cover architecture selection, same-tag binary/checksum URLs, missing releases,
malformed checksums, and custom download overrides without touching services.
The Linux tests use Python 3 and Bash. The Windows suite also parses the full
installer and runs under PowerShell on any OS; set `POWERSHELL=powershell` or an
absolute executable path if it is not named `pwsh`. It is skipped when that
executable is unavailable.

`install-linux.sh` can be exercised end to end in a systemd container, against a
real HTTPS source, without weakening the script:

```bash
docker run -d --name fastdl-test --privileged --cgroupns=host \
    --tmpfs /run --tmpfs /run/lock -v /sys/fs/cgroup:/sys/fs/cgroup:rw \
    <image with systemd, curl, ca-certificates, openssl, python3>

# Inside the container: publish the binary over HTTPS with a self-signed
# certificate added to /usr/local/share/ca-certificates, then
# update-ca-certificates. curl --proto '=https' is then satisfied normally.
```

Worth covering, because each one exercises a different branch: a clean install;
a re-run with the same digest (the service must keep its PID); a tampered unit
file (must be repaired and restarted); a wrong `--sha256` (must report expected
and actual and change nothing); an executable that passes `version` and
`validate` but exits on `serve` (must roll back to the previous version and leave
the service running); `--check` before and after.

Static checks, from the repository root:

```bash
shellcheck scripts/install-linux.sh
bash -n scripts/install-linux.sh
```

For `install-windows.ps1`, a parse check and `Invoke-ScriptAnalyzer` run under
PowerShell on any platform; `Write-Host` and the `ShouldProcess` warnings are
expected and shared with the other GameAP installers. Service registration
itself can only be verified on Windows.
