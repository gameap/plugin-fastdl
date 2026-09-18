# GameAP FastDL plugin

Rust/WASI backend and Vue frontend for managing `gameap-fastdl` on GameAP Daemon nodes. Supports GoldSource and Source, Linux/systemd and Windows Service Control Manager.

## Build and verify

The neighboring `gameap-proto/rust/gameap-plugin-sdk` and frontend SDK package are required, following the same workspace layout as `plugin-files` and `plugin-respawn`.

```sh
rustup target add wasm32-wasip1
make build
cargo test
cargo clippy --all-targets -- -D warnings
cargo clippy --target wasm32-wasip1 -- -D warnings
cd frontend
npm test
npm run typecheck
```

Upload `fastdl.wasm` through GameAP's plugin management. Grant `files`, `listen_events`, `manage_servers`, and `node_commands`. The technical plugin ID is `fastdla`: it round-trips through GameAP's compact base32 ID encoding. The visible name is **FastDL**. The six-character name `fastdl` cannot be used as the technical ID because the host would normalize it to a different identifier.

## Rust backend structure

The backend follows the layout of `plugin-files`:

```text
src/
  lib.rs                Plugin registration and SDK entry points
  router.rs             Route table, matching and request dispatch
  http.rs               JSON parsing, responses and API errors
  handlers/             HTTP handlers, authorization and event dispatch
  domain/               Node/server models and input validation
  services/
    admin.rs            Node overview
    node_setup.rs       Installation, status and node settings
    servers.rs          Game server settings and activation
    sync.rs             Configuration publication and reconciliation
    game_config.rs      Scoped server.cfg helper invocation
    paths.rs            Validated daemon paths and public URLs
    store.rs            Typed access to persistent plugin state
  host_api.rs           SDK adapter and test host
  shell.rs              Daemon command argument quoting
```

Handlers check permissions, parse requests and delegate to services. Services use typed domain models; storage keys and serialization are centralized in `store`. The router table supplies both local dispatch and routes registered with GameAP. Tests exercise the same handlers and services through the host adapter.

## Node installation

1. Open **Administration → FastDL**, select a node, and save its listen address and public base URL.
2. Select **Install** and confirm. Use **Update** for an existing installation. The installer and the latest stable binary for the node's OS and architecture are selected automatically.
3. Wait for installation to complete. The plugin tracks the installation task and verifies `gameap-fastdl version` after successful service installation.
4. Open a GoldSource or Source server's **FastDL** tab, select its engine and game directory, and activate FastDL.

The Linux and Windows installers from this repository's `scripts` directory are bundled into `fastdl.wasm`. The plugin uploads the installer for the node's OS into `.plugins/fastdla` and creates one daemon task to run it. Each installation replaces that private copy with the bundled version; old `tools/install-linux.sh` and `tools/install-windows.ps1` files are no longer used. Rebuild and deploy `fastdl.wasm` after changing an installer so the plugin and its script are shipped together.

After the installation task succeeds, the plugin runs `gameap-fastdl version` and extracts its standalone version line from the daemon's response, allowing a command prompt and exit-status footer around it. This probe verifies the executable; the installer performs the service checks described below. After updating the plugin, refresh the FastDL page to recheck installations whose saved error is exactly `Installed binary could not be verified`. If their installation task succeeded and the version probe passes, no installation rerun is needed.

Verification failures are logged with the prefix `[fastdl] Installation verification failed:` in the GameAP panel process log. The diagnostic includes the node and installation task, the version command, and either a host-call error or the exit code, execution error and bounded command output. These records are separate from the installer task output and the `gameap-fastdl` service log. If the panel runs as `gameap.service` under systemd, read them with `journalctl -u gameap -n 100 --no-pager`; error logging does not require debug mode.

The installer selects the latest stable [gameap-fastdl release](https://github.com/gameap/gameap-fastdl/releases), detects amd64 or arm64, and downloads the binary and its `.sha256` file from that same release over HTTPS. A published release with both assets is required; missing assets or an invalid checksum fail installation. See [scripts/README.md](scripts/README.md) for the release asset contract.

The installer verifies SHA256 **before executing the binary**, validates configuration, registers or updates the service, and checks that it stays up: both installers poll to a deadline and then require the service to hold for several seconds, because a restart policy makes a crash loop look healthy between restarts. Linux requires root/systemd; Windows requires administrative SCM access. Scripts preserve the previous executable and service definition and roll back any change they made once something fails, reporting a failed update as failed even when rollback restores the previous running version.

Re-running installation when the resolved SHA256 is already installed skips the binary download and leaves a healthy service untouched, while still reconciling the service definition -- so it repairs a damaged service without an outage. The installers take named options and support `--help`/`-Help` and `--check`/`-Check`, which report the installed version and service state without changing anything. See [scripts/README.md](scripts/README.md) for the exact invocation and a local test recipe.

The Linux installer uses a system service with `ProtectSystem=strict`, no-new-privileges and a private cache as its only writable service directory, restarted by systemd after a crash. The Windows service runs as LocalSystem with equivalent SCM recovery actions. The private plugin directory must remain owned by a trusted daemon/service account and inaccessible to game-server accounts; Linux installation makes it root-owned with mode `0700`, and Windows applies a protected ACL to the plugin directory -- granting only SYSTEM, Administrators and the installing daemon identity -- which its contents inherit. Nodes installed by an earlier release carry those rules per file instead, which blocks inheritance; `-FixAcl` restores it once. The daemon identity must itself be trusted. Parent work directories must not be writable or replaceable by hosted game accounts. Installation refuses a path containing a symbolic link, reparse point, quote, `%` or control character, since both paths are written into a service definition.

Expose the configured port through the existing firewall or reverse proxy. HTTPS can terminate at that proxy; choose an HTTP public URL if a particular legacy client cannot use HTTPS. If the public base URL has a path prefix, configure the reverse proxy to remove that prefix before forwarding to FastDL, whose route starts at `/<token>/`. Preserve the remaining path without decoding or normalizing it. No firewall rules are changed automatically.

## Per-server behavior

- `game_dir` is relative to GameAP's stored server directory; an empty value means that server directory already is the game/mod directory. Absolute paths, traversal and internal directories such as `cfg`, `addons`, `plugins`, `logs` and `bin` are refused.
- Known GameAP game codes receive defaults. Custom game codes can select GoldSource or Source explicitly.
- The public URL contains a random 128-bit token. Server and node database IDs, filesystem paths and configuration contents are absent from regular users' response bodies.
- Autoindex is disabled by default. The Go server displays only files allowed by its fixed content policy.
- Source `.bz2` generation is enabled by default and occurs on demand in the Go service's private bounded cache. GoldSource uses original files.
- Automatic game configuration is enabled by default and can be disabled. The plugin calls the installed Go helper to maintain an exact marked block in `<game_dir>/server.cfg` for GoldSource or `<game_dir>/cfg/server.cfg` for Source. The helper scopes filesystem access to the server root with no-follow handles; the plugin never reads or overwrites game configuration through the broader node file API. Missing final files may be created when their parent directory exists. Unrelated settings are preserved; disabling removes only the managed block. The helper writes through a checked open file handle to prevent path replacement attacks; this is an in-place write, so a process or machine failure during the write can leave a partial configuration. Keep operational configuration backups. Apply changes with a game-server restart or configuration reload.
- `fastdl-manage` authorizes mutations and `fastdl-view` grants read-only access. Assign both abilities to managers so the GameAP tab is visible; the host tab SDK requires the view ability even though the backend treats manage as including view. Administrators can manage all servers. Every route repeats its authorization check inside the plugin, including entity-specific checks, and authorization service errors fail closed.

## Synchronization and failure handling

The plugin persists desired settings before publishing a server configuration. Each update first publishes an explicitly disabled entry, invokes the game-configuration helper when needed, and publishes enabled state only after all preceding steps succeed. A failure is returned to the UI and remains visible as unsynchronized state. Disabling and deleting a server revoke its registration; node-scoped registration records preserve this ability even after GameAP removes server-scoped storage. A node synchronization also revokes registrations for deleted or moved servers.

The game-configuration helper must exit successfully and return a single `{"configured":true}` confirmation. Daemon command prompts and exit-status lines around that response are supported. `GAME_CONFIG_UPDATE_FAILED` means the command failed; check the game directory and file permissions. `CONFIGURE_FAILED` means the command did not return a valid confirmation; `server.cfg` may already have changed, but the FastDL entry remains disabled. The interface shows **Changes not applied**, preserves the edited form, and allows another save after the problem is resolved.

Saving an installed node's settings and using **Apply configuration** both reconcile its registered game servers through the same helper. A game-configuration failure can therefore interrupt either node operation before the FastDL service restarts; some game servers may already have been updated. Node settings are persisted before this step; the admin message distinguishes saved settings from incomplete application and identifies the affected game server by name. Such API errors retain their original `code` and add `server_name`, without exposing database IDs. Correct the reported problem and repeat the same save or apply operation.

Configuration failures are recorded with the prefix `[fastdl] Game configuration failed:` in the **GameAP panel process log**, separately from installation task output and the `gameap-fastdl` service log. These diagnostics contain the node and server references, failure reason, and bounded command output or host error; download tokens are redacted. If the panel runs as `gameap.service`, use `journalctl -u gameap -n 100 --no-pager`. Review that record before retrying **Save** or **Apply configuration**.

The Go service reloads drop-ins every two seconds. Revocation therefore has a delay of up to two seconds; already-open downloads may finish. Concurrent edits use last-write semantics. One fixed private filename per game server prevents simultaneous first-time activation from leaving multiple public tokens active. Use **Synchronize** to reapply persisted desired state if a request failed midway or concurrent updates disagreed.

Service files live below the daemon work path:

```text
.plugins/fastdla/
  gameap-fastdl[.exe]
  install-linux.sh or install-windows.ps1
  config.json
  servers.d/server-<private-id>.json
  cache/
  install.<random>/   staging, only while installing
```

The installer is uploaded through the same node file API as the configuration and runs from this private directory.

The numeric filename is private administrative state; HTTP uses only the independent token. The main JSON configuration is `{ "version": 1, "listen": "0.0.0.0:8080", "servers_dir": "servers.d", "cache_dir": "cache" }`. Each drop-in contains `token`, derived absolute `root`, `engine`, `enabled`, `autoindex`, and `generate_bz2`. The plugin does not offer custom extension allowlists; the Go service enforces its content policy.

## HTTP contract

All routes are relative to `/api/plugins/fastdla` and require an authenticated session.

| Method | Route                        | Access         | Purpose                          |
|--------|------------------------------|----------------|----------------------------------|
| GET    | `/admin/nodes`               | Administrator  | Node list and installation state |
| GET    | `/nodes/{nodeId}/config`     | Administrator  | Listen and public URL settings   |
| PUT    | `/nodes/{nodeId}/config`     | Administrator  | Save and apply node settings     |
| GET    | `/nodes/{nodeId}/status`     | Administrator  | Poll installation task           |
| POST   | `/nodes/{nodeId}/setup`      | Administrator  | Install/update verified binary   |
| POST   | `/nodes/{nodeId}/sync`       | Administrator  | Reapply desired node state       |
| GET    | `/servers/{serverId}/fastdl` | View or manage | Server settings and download URL |
| PUT    | `/servers/{serverId}/fastdl` | Manage         | Apply server settings            |

Node settings are `{ "listen": "0.0.0.0:8080", "public_base_url": "http://cdn.example" }`. Setup accepts an empty body or `{}` for automatic installation. Existing integrations and custom builds can still pass `{ "download_url": "https://releases.example/gameap-fastdl", "sha256": "<64 hexadecimal digits>" }`; both fields must be supplied together. Setup status identifies the installation task. The `download_task_id` field remains available for installations started by earlier plugin versions and is `0` for new installations. A server update accepts `enabled`, `autoindex`, `engine` (`goldsource` or `source`), `game_dir`, `manage_game_config`, and `generate_bz2`. Unknown fields are rejected.

Native Rust tests cover authorization boundaries, traversal and internal-path rejection, scoped helper invocation and failure handling, opaque public responses, install input checks, deletion after storage cleanup, stable private registration filenames, and the exact installer command for each platform together with the options the scripts accept. The frontend has validation/i18n unit tests and a browser smoke check with mocked API responses. `make lint` runs shellcheck over the Linux installer. [scripts/README.md](scripts/README.md) describes running it end to end against a local HTTPS source in a systemd container, including the rollback and re-run cases. Actual Windows service installation must be verified in a provisioned GameAP environment; local checks do not replace that deployment test.
