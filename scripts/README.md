# FastDL node installers

Installers for `gameap-fastdl`, the node-side Go service the FastDL plugin
manages. One per supported node platform:

| Script                 | Platform             | Service                    |
|------------------------|----------------------|----------------------------|
| `install-linux.sh`     | Linux, root, systemd | `gameap-fastdl.service`    |
| `install-windows.ps1`  | Windows, elevated    | `gameap-fastdl` SCM service |

## How they reach a node

Unlike the installers in the [`gameap/scripts`](https://github.com/gameap/scripts)
repository, these are **not** fetched with `get-tool`. They are compiled into
`fastdl.wasm` with `include_bytes!` (`src/services/node_setup.rs`), uploaded to
`<work path>/.plugins/fastdla/` with mode `0700` and run as a single daemon task:

```
/bin/bash {node_work_path}/.plugins/fastdla/install-linux.sh \
    --download-url=https://... --sha256=<64 hex characters> \
    --install-dir={node_work_path}/.plugins/fastdla \
    --config={node_work_path}/.plugins/fastdla/config.json
```

```
powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File
    "{node_work_path}\.plugins\fastdla\install-windows.ps1"
    -DownloadUrl https://... -Sha256 <64 hex characters>
    -InstallDir "{node_work_path}\.plugins\fastdla"
    -ConfigPath "{node_work_path}\.plugins\fastdla\config.json"
```

**Editing a script therefore requires `make build` and a plugin upload.** A node
never sees a newer script than the plugin that uploaded it, which is why the two
argument lists can change together without a compatibility window.
`installers_accept_the_options_the_plugin_passes` in `src/handlers/tests.rs`
fails if an option is renamed on only one side.

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
4. **Verify SHA256 before executing anything.** This ordering is the reason the
   digest is passed in at all; nothing runs the download before it matches.
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

Re-running with the same `--sha256` skips the download and, when the service
definition also matches and the service is running, changes nothing at all — so
a re-run is a safe way to repair a damaged service without an outage.

## Checking a node

Both scripts report an installation without changing it, and exit 1 when it is
unhealthy. The paths are read from the installed service when they are omitted:

```
/bin/bash /srv/gameap/.plugins/fastdla/install-linux.sh --check
powershell -NoProfile -ExecutionPolicy Bypass -File install-windows.ps1 -Check
```

## Testing locally

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
