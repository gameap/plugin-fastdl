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

## Node installation

1. Build the appropriate standalone `gameap-fastdl` executable from the sibling project. Publish it at a trusted HTTPS URL and obtain its SHA256 digest from the trusted build.
2. Open **Administration → FastDL**, select a node, and save its listen address and public base URL.
3. Supply the binary URL and SHA256 and select **Install**. The same action updates an existing installation.
4. Wait for installation to complete. The plugin tracks a daemon task and verifies `gameap-fastdl version` after successful service installation.
5. Open a GoldSource or Source server's **FastDL** tab, select its engine and game directory, and activate FastDL.

Installation uploads the bundled installer through GameAP Daemon. It downloads a binary over HTTPS, verifies SHA256 **before executing it**, validates configuration, registers or updates the service, and checks that it starts. Linux requires root/systemd; Windows requires administrative SCM access. Scripts preserve the previous executable and service definition for rollback on startup failure. A failed update is reported as failed even if rollback restores the previous running version.

The Linux installer uses a system service with `ProtectSystem=strict`, no-new-privileges and a private cache as its only writable service directory. The Windows service runs as LocalSystem. The private plugin directory must remain owned by a trusted daemon/service account and inaccessible to game-server accounts; Linux installation makes it root-owned with mode `0700`, and Windows removes inherited ACL access and grants only SYSTEM, Administrators and the installing daemon identity. The daemon identity must itself be trusted. Parent work directories must not be writable or replaceable by hosted game accounts.

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

The Go service reloads drop-ins every two seconds. Revocation therefore has a delay of up to two seconds; already-open downloads may finish. Concurrent edits use last-write semantics. One fixed private filename per game server prevents simultaneous first-time activation from leaving multiple public tokens active. Use **Synchronize** to reapply persisted desired state if a request failed midway or concurrent updates disagreed.

Service files live below the daemon work path:

```text
.plugins/fastdla/
  gameap-fastdl[.exe]
  config.json
  servers.d/server-<private-id>.json
  cache/
```

The numeric filename is private administrative state; HTTP uses only the independent token. The main JSON configuration is `{ "version": 1, "listen": "0.0.0.0:8080", "servers_dir": "servers.d", "cache_dir": "cache" }`. Each drop-in contains `token`, derived absolute `root`, `engine`, `enabled`, `autoindex`, and `generate_bz2`. The plugin does not offer custom extension allowlists; the Go service enforces its content policy.

## HTTP contract

All routes are relative to `/api/plugins/fastdla` and require an authenticated session.

| Method   | Route                         | Access            | Purpose                            |
|----------|-------------------------------|-------------------|------------------------------------|
| GET      | `/admin/nodes`                | Administrator     | Node list and installation state   |
| GET      | `/nodes/{nodeId}/config`      | Administrator     | Listen and public URL settings     |
| PUT      | `/nodes/{nodeId}/config`      | Administrator     | Save and apply node settings       |
| GET      | `/nodes/{nodeId}/status`      | Administrator     | Poll installation task             |
| POST     | `/nodes/{nodeId}/setup`       | Administrator     | Install/update verified binary     |
| POST     | `/nodes/{nodeId}/sync`        | Administrator     | Reapply desired node state         |
| GET      | `/servers/{serverId}/fastdl`  | View or manage    | Server settings and download URL   |
| PUT      | `/servers/{serverId}/fastdl`  | Manage            | Apply server settings              |

Node settings are `{ "listen": "0.0.0.0:8080", "public_base_url": "http://cdn.example" }`. Setup accepts `{ "download_url": "https://releases.example/gameap-fastdl", "sha256": "<64 hexadecimal digits>" }`. A server update accepts `enabled`, `autoindex`, `engine` (`goldsource` or `source`), `game_dir`, `manage_game_config`, and `generate_bz2`. Unknown fields are rejected.

Native Rust tests cover authorization boundaries, traversal and internal-path rejection, scoped helper invocation and failure handling, opaque public responses, install input checks, deletion after storage cleanup, and stable private registration filenames. The frontend has validation/i18n unit tests and a browser smoke check with mocked API responses. Actual Linux/Windows service installation must be verified in a provisioned GameAP environment; local checks do not replace that deployment test.
