//! Route tests verify authorization, persisted settings and daemon-side effects.

use gameap_plugin_sdk::proto::gameap::plugin as pb;
use serde_json::{Value, json};

use crate::domain::{NodeConfig, NodeSetupStatus, ServerState, SetupInput, SetupStatus};
use crate::host_api::mock::MockHost;
use crate::host_api::{CommandOutput, HostApi, StorageEntity, TaskStatus};
use crate::router;
use crate::services::{node_setup, store, sync};

fn request(user_id: u64, method: &str, path: &str, body: Value) -> pb::HttpRequest {
    pb::HttpRequest {
        method: method.into(),
        path: path.into(),
        body: serde_json::to_vec(&body).expect("request body is JSON"),
        session: Some(pb::Session {
            user: Some(gameap_plugin_sdk::proto::gameap::User {
                id: user_id,
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn installed_host() -> MockHost {
    let mut host = MockHost::standard();
    store::save_status(
        &mut host,
        1,
        &NodeSetupStatus {
            status: SetupStatus::Installed,
            ..Default::default()
        },
    )
    .expect("installed status is stored");

    store::save_config(
        &mut host,
        1,
        &NodeConfig {
            listen: "0.0.0.0:8080".into(),
            public_base_url: "http://cdn.example".into(),
        },
    )
    .expect("node configuration is stored");

    host.fail_on.push((
        " configure ".into(),
        CommandOutput {
            exit_code: 0,
            error: None,
            output: "{\"configured\":true}".into(),
        },
    ));
    host.files.insert(
        (1, "servers/cs/cstrike/server.cfg".into()),
        b"hostname test\nrcon_password secret\n".to_vec(),
    );
    host
}

fn server_settings() -> Value {
    json!({
        "enabled": true,
        "autoindex": true,
        "engine": "goldsource",
        "game_dir": "cstrike",
        "manage_game_config": true,
        "generate_bz2": true,
    })
}

fn stored_server(host: &mut MockHost, server_id: u64) -> ServerState {
    store::get_server_state(host, server_id)
        .expect("server settings can be loaded")
        .expect("server settings are stored")
}

fn uploaded_server_config(host: &MockHost, node_id: u64, server_id: u64) -> Value {
    let path = format!(".plugins/fastdla/servers.d/server-{server_id}.json");
    let content = host
        .file(node_id, &path)
        .expect("server config is uploaded");

    serde_json::from_slice(content).expect("uploaded server config is valid JSON")
}

fn enable_server(host: &mut MockHost) {
    let response = router::dispatch(
        host,
        &request(1, "PUT", "/servers/3/fastdl", server_settings()),
    );
    assert_eq!(response.status_code, 200);
}

fn start_installation(host: &mut MockHost) -> u64 {
    let response = router::dispatch(host, &request(1, "POST", "/nodes/1/setup", json!({})));
    assert_eq!(response.status_code, 202);
    let download_id = host.created_tasks[0].0;
    host.task_states.get_mut(&download_id).unwrap().status = TaskStatus::Success;

    host.created_tasks
        .last()
        .expect("installation task exists")
        .0
}

#[test]
fn denies_cross_server_access_before_read_or_write() {
    let mut host = installed_host();
    host.grants
        .push((7, 4, "plugin:fastdla:fastdl-manage".into()));
    for method in ["GET", "PUT"] {
        let response = router::dispatch(
            &mut host,
            &request(7, method, "/servers/3/fastdl", server_settings()),
        );
        assert_eq!(response.status_code, 403);
    }
    assert!(host.uploads.is_empty());
}

#[test]
fn denies_anonymous_and_nonadmin_node_operations() {
    let mut host = installed_host();
    assert_eq!(
        router::dispatch(
            &mut host,
            &pb::HttpRequest {
                path: "/admin/nodes".into(),
                method: "GET".into(),
                ..Default::default()
            }
        )
        .status_code,
        401
    );
    assert_eq!(
        router::dispatch(&mut host, &request(7, "POST", "/nodes/1/setup", json!({}))).status_code,
        403
    );
}

#[test]
fn authz_unavailable_fails_closed() {
    let mut host = installed_host();
    host.authz_down = true;
    assert_eq!(
        router::dispatch(
            &mut host,
            &request(1, "PUT", "/servers/3/fastdl", server_settings())
        )
        .status_code,
        502
    );
    assert!(host.uploads.is_empty());
}

#[test]
fn view_ability_does_not_allow_mutation() {
    let mut host = installed_host();
    host.grants
        .push((7, 3, "plugin:fastdla:fastdl-view".into()));
    assert_eq!(
        router::dispatch(
            &mut host,
            &request(7, "GET", "/servers/3/fastdl", json!({}))
        )
        .status_code,
        200
    );
    assert_eq!(
        router::dispatch(
            &mut host,
            &request(7, "PUT", "/servers/3/fastdl", server_settings())
        )
        .status_code,
        403
    );
}

#[test]
fn enable_disable_uses_scoped_helper_and_opaque_public_url() {
    let mut host = installed_host();
    let response = router::dispatch(
        &mut host,
        &request(1, "PUT", "/servers/3/fastdl", server_settings()),
    );
    assert_eq!(
        response.status_code,
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    let body: Value = serde_json::from_slice(&response.body).expect("response body is JSON");
    assert!(body.get("node_id").is_none());
    assert!(body.get("server_id").is_none());
    assert!(!String::from_utf8_lossy(&response.body).contains("/srv/gameap"));
    let state = stored_server(&mut host, 3);
    assert_eq!(state.token.len(), 32);

    let command = host
        .commands
        .last()
        .expect("configuration command executed");
    assert!(command.contains(concat!(
        "configure --root /srv/gameap/servers/cs ",
        "--game-dir cstrike --engine goldsource ",
        "--url http://cdn.example/",
    )));
    assert!(
        !host
            .uploads
            .iter()
            .any(|(_, path, _)| path.ends_with("server.cfg"))
    );
    let mut disabled = server_settings();
    disabled["enabled"] = false.into();
    assert_eq!(
        router::dispatch(&mut host, &request(1, "PUT", "/servers/3/fastdl", disabled)).status_code,
        200
    );
    let command = host
        .commands
        .last()
        .expect("configuration command executed");
    assert!(!command.contains("--url"));
    let dropin = uploaded_server_config(&host, 1, state.server_id);
    assert_eq!(dropin["enabled"], false);
}

#[test]
fn rejects_traversal_and_arbitrary_roots() {
    for value in [
        "../other",
        "/etc",
        "cstrike/../../private",
        "cstrike\\..\\private",
        "C:/games",
        "cstrike/%2e%2e",
        "cstrike//maps",
        ".git",
        "cstrike/.",
        "cfg",
        "cstrike/addons",
        "plugins",
    ] {
        let mut host = installed_host();
        let mut body = server_settings();
        body["game_dir"] = value.into();
        assert_eq!(
            router::dispatch(&mut host, &request(1, "PUT", "/servers/3/fastdl", body)).status_code,
            400,
            "{value}"
        );
        assert!(host.uploads.is_empty());
    }
    let mut host = installed_host();
    let mut body = server_settings();
    body["root"] = "/etc".into();
    assert_eq!(
        router::dispatch(&mut host, &request(1, "PUT", "/servers/3/fastdl", body)).status_code,
        400
    );
}

#[test]
fn configuration_helper_failure_keeps_route_disabled_and_unsynced() {
    let mut host = installed_host();
    host.fail_on.clear();
    host.command_results.push_back(CommandOutput {
        exit_code: 1,
        error: None,
        output: "rejected symbolic link".into(),
    });
    let response = router::dispatch(
        &mut host,
        &request(1, "PUT", "/servers/3/fastdl", server_settings()),
    );
    assert_eq!(response.status_code, 409);
    let state = stored_server(&mut host, 3);
    assert!(!state.synced);
    let dropin = uploaded_server_config(&host, 1, 3);
    assert_eq!(dropin["enabled"], false);
}

#[test]
fn custom_installation_requires_https_and_digest() {
    for (url, digest) in [
        ("http://example.com/gameap-fastdl", "a".repeat(64)),
        ("https://example.com/file", "bad".into()),
        ("https://example.com/$(touch evil)", "a".repeat(64)),
        ("https://example.com/file", String::new()),
        ("", "a".repeat(64)),
    ] {
        assert!(
            SetupInput {
                download_url: url.into(),
                sha256: digest
            }
            .validate()
            .is_err()
        );
    }
}

#[test]
fn delete_disables_dropin_even_after_server_is_gone() {
    let mut host = installed_host();
    assert_eq!(
        router::dispatch(
            &mut host,
            &request(1, "PUT", "/servers/3/fastdl", server_settings())
        )
        .status_code,
        200
    );
    let state = stored_server(&mut host, 3);
    host.servers.clear();
    sync::on_deleted(&mut host, 3, 1).unwrap();
    let dropin = uploaded_server_config(&host, 1, state.server_id);
    assert_eq!(dropin["enabled"], false);
    assert!(store::get_server_state(&mut host, 3).unwrap().is_none());
}

#[test]
fn deletion_uses_node_registration_after_server_storage_is_removed() {
    let mut host = installed_host();
    assert_eq!(
        router::dispatch(
            &mut host,
            &request(1, "PUT", "/servers/3/fastdl", server_settings())
        )
        .status_code,
        200
    );
    host.servers.clear();
    host.storage_delete("fastdl:server", StorageEntity::server(3))
        .unwrap();
    sync::on_deleted(&mut host, 3, 1).unwrap();
    let value = uploaded_server_config(&host, 1, 3);
    assert_eq!(value["enabled"], false);
}

#[test]
fn repeated_initial_setup_uses_one_private_registration_filename() {
    let mut host = installed_host();
    assert_eq!(
        router::dispatch(
            &mut host,
            &request(1, "PUT", "/servers/3/fastdl", server_settings())
        )
        .status_code,
        200
    );
    host.storage_delete("fastdl:server", StorageEntity::server(3))
        .unwrap();
    assert_eq!(
        router::dispatch(
            &mut host,
            &request(1, "PUT", "/servers/3/fastdl", server_settings())
        )
        .status_code,
        200
    );
    assert_eq!(
        host.files
            .keys()
            .filter(|(_, path)| path.starts_with(".plugins/fastdla/servers.d/"))
            .count(),
        1
    );
}

#[test]
fn moving_to_unconfigured_node_revokes_original_route_first() {
    let mut host = installed_host();
    assert_eq!(
        router::dispatch(
            &mut host,
            &request(1, "PUT", "/servers/3/fastdl", server_settings())
        )
        .status_code,
        200
    );
    let mut target = host.nodes.get(&1).unwrap().clone();
    target.id = 2;
    host.nodes.insert(2, target);
    host.servers.get_mut(&3).unwrap().node_id = 2;
    assert!(sync::on_updated(&mut host, 3).is_err());
    let value = uploaded_server_config(&host, 1, 3);
    assert_eq!(value["enabled"], false);
    assert!(!stored_server(&mut host, 3).synced);
}

#[test]
fn failed_disable_remains_visibly_unsynchronized() {
    let mut host = installed_host();
    assert_eq!(
        router::dispatch(
            &mut host,
            &request(1, "PUT", "/servers/3/fastdl", server_settings())
        )
        .status_code,
        200
    );
    host.fail_on.clear();
    host.command_results.push_back(CommandOutput {
        exit_code: 1,
        error: None,
        output: String::new(),
    });
    let mut value = server_settings();
    value["enabled"] = false.into();
    assert_eq!(
        router::dispatch(&mut host, &request(1, "PUT", "/servers/3/fastdl", value)).status_code,
        409
    );
    let response = router::dispatch(
        &mut host,
        &request(1, "GET", "/servers/3/fastdl", json!({})),
    );
    let body: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(body["synced"], false);
}

#[test]
fn cannot_clear_public_url_while_any_server_is_enabled() {
    let mut host = installed_host();
    assert_eq!(
        router::dispatch(
            &mut host,
            &request(1, "PUT", "/servers/3/fastdl", server_settings())
        )
        .status_code,
        200
    );
    assert_eq!(
        router::dispatch(
            &mut host,
            &request(
                1,
                "PUT",
                "/nodes/1/config",
                json!({
                    "listen": "0.0.0.0:8080",
                    "public_base_url": "",
                })
            )
        )
        .status_code,
        409
    );
    assert_eq!(
        store::get_config(&mut host, 1).unwrap().public_base_url,
        "http://cdn.example"
    );
}

#[test]
fn automatic_installation_uses_the_node_platform_and_saved_configuration() {
    for os in ["linux", " Windows ", ""] {
        for empty_body in [false, true] {
            let mut host = installed_host();
            let node = host.nodes.get_mut(&1).unwrap();
            node.os = os.into();
            node.work_path = if os == " Windows " {
                r"C:\GameAP Data".into()
            } else {
                "/srv/gameap data".into()
            };
            let mut req = request(1, "POST", "/nodes/1/setup", json!({}));
            if empty_body {
                req.body.clear();
            }
            let response = router::dispatch(&mut host, &req);
            assert_eq!(response.status_code, 202);
            assert_eq!(host.created_tasks.len(), 2);
            let download = &host.created_tasks[0];
            let install = &host.created_tasks[1];
            assert_eq!(download.3, None);
            assert_eq!(install.3, Some(download.0));
            let (script, expected) = if os == " Windows " {
                (
                    "install-windows.ps1",
                    concat!(
                        "powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass ",
                        r#"-File "{node_tools_path}/install-windows.ps1" "#,
                        r#"-InstallDir "C:\GameAP Data\.plugins\fastdla" "#,
                        r#"-ConfigPath "C:\GameAP Data\.plugins\fastdla\config.json""#,
                    ),
                )
            } else {
                (
                    "install-linux.sh",
                    concat!(
                        "/bin/bash '{node_tools_path}/install-linux.sh' ",
                        "'--install-dir=/srv/gameap data/.plugins/fastdla' ",
                        "'--config=/srv/gameap data/.plugins/fastdla/config.json'",
                    ),
                )
            };
            assert_eq!(
                download.2,
                format!(
                    "get-tool https://raw.githubusercontent.com/gameap/plugin-fastdl/main/scripts/{script}"
                )
            );
            assert_eq!(install.2, expected);
            let status: NodeSetupStatus = serde_json::from_slice(&response.body).unwrap();
            assert_eq!(status.task_id, install.0);
            assert_eq!(status.download_task_id, download.0);
            assert_eq!(status, store::get_status(&mut host, 1).unwrap());
            assert_eq!(
                store::get_config(&mut host, 1).unwrap().public_base_url,
                "http://cdn.example"
            );
            assert!(
                host.uploads
                    .iter()
                    .all(|(_, path, _)| !path.ends_with(script))
            );
            assert!(
                host.uploads
                    .iter()
                    .any(|(_, path, _)| path.ends_with("config.json"))
            );
        }
    }
}

#[test]
fn setup_rejects_invalid_requests_and_unsupported_nodes_before_side_effects() {
    for body in [
        json!({"download_url": "https://example.com/file"}),
        json!({"sha256": "a".repeat(64)}),
        json!({"script_url": "https://example.com/install.sh"}),
        json!(null),
    ] {
        let mut host = installed_host();
        let response = router::dispatch(&mut host, &request(1, "POST", "/nodes/1/setup", body));
        assert_eq!(response.status_code, 400);
        assert!(host.uploads.is_empty());
        assert!(host.created_tasks.is_empty());
    }
    let mut host = installed_host();
    host.nodes.get_mut(&1).unwrap().os = "macos".into();
    let response = router::dispatch(&mut host, &request(1, "POST", "/nodes/1/setup", json!({})));
    assert_eq!(response.status_code, 400);
    assert!(host.uploads.is_empty());
    assert!(host.created_tasks.is_empty());
}

#[test]
fn automatic_installation_waits_for_download_and_rejects_duplicate_setup() {
    let mut host = installed_host();
    let req = request(1, "POST", "/nodes/1/setup", json!({}));
    assert_eq!(router::dispatch(&mut host, &req).status_code, 202);
    assert_eq!(router::dispatch(&mut host, &req).status_code, 409);
    assert_eq!(host.created_tasks.len(), 2);
    let install_id = host.created_tasks[1].0;
    host.task_states.get_mut(&install_id).unwrap().status = TaskStatus::Success;
    assert_eq!(
        node_setup::get_status(&mut host, 1).unwrap().status,
        SetupStatus::Installing
    );
    assert!(host.commands.is_empty());
}

#[test]
fn failed_installer_download_fails_setup_without_probing_the_binary() {
    for failure in [TaskStatus::Error, TaskStatus::Canceled] {
        let mut host = installed_host();
        start_installation(&mut host);
        let download_id = host.created_tasks[0].0;
        host.task_states.get_mut(&download_id).unwrap().status = failure;
        let status = node_setup::get_status(&mut host, 1).unwrap();
        assert_eq!(status.status, SetupStatus::Failed);
        assert!(
            status
                .error_message
                .starts_with("Installer download failed")
        );
        assert_eq!(status.task_id, download_id);
        assert!(host.commands.is_empty());
    }
}

#[test]
fn missing_installer_download_task_expires() {
    let mut host = installed_host();
    start_installation(&mut host);
    let download_id = host.created_tasks[0].0;
    host.task_states.remove(&download_id);
    host.now += 1801;
    let status = node_setup::get_status(&mut host, 1).unwrap();
    assert_eq!(status.status, SetupStatus::Failed);
    assert_eq!(status.error_message, "Installation timed out");
}

#[test]
fn linux_and_windows_install_tasks_preserve_custom_verified_downloads() {
    for os in ["linux", "windows"] {
        let mut host = installed_host();
        host.nodes.get_mut(&1).unwrap().os = os.into();
        if os == "windows" {
            host.nodes.get_mut(&1).unwrap().work_path = r"C:\GameAP Data".into();
        }
        let response = router::dispatch(
            &mut host,
            &request(
                1,
                "POST",
                "/nodes/1/setup",
                json!({
                    "download_url": "https://releases.example/gameap-fastdl",
                    "sha256": "a".repeat(64),
                }),
            ),
        );
        assert_eq!(
            response.status_code,
            202,
            "{}",
            String::from_utf8_lossy(&response.body)
        );
        assert_eq!(host.created_tasks.len(), 2);
        assert_eq!(host.created_tasks[1].3, Some(host.created_tasks[0].0));

        // Pinned in full: a substring check passes just as happily when the
        // caller and the installer disagree about the argument names.
        let digest = "a".repeat(64);
        let expected = if os == "windows" {
            format!(
                concat!(
                    "powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass ",
                    r#"-File "{node_tools_path}/install-windows.ps1" "#,
                    "-DownloadUrl https://releases.example/gameap-fastdl ",
                    "-Sha256 {digest} ",
                    r#"-InstallDir "C:\GameAP Data\.plugins\fastdla" "#,
                    r#"-ConfigPath "C:\GameAP Data\.plugins\fastdla\config.json""#,
                ),
                digest = digest,
                node_tools_path = "{node_tools_path}",
            )
        } else {
            format!(
                concat!(
                    "/bin/bash '{node_tools_path}/install-linux.sh' ",
                    "--download-url=https://releases.example/gameap-fastdl ",
                    "--sha256={digest} ",
                    "--install-dir=/srv/gameap/.plugins/fastdla ",
                    "--config=/srv/gameap/.plugins/fastdla/config.json",
                ),
                digest = digest,
                node_tools_path = "{node_tools_path}",
            )
        };
        assert_eq!(host.created_tasks[1].2, expected);

        let installer_name = if os == "windows" {
            "install-windows.ps1"
        } else {
            "install-linux.sh"
        };
        assert_eq!(
            host.created_tasks[0].2,
            format!(
                "get-tool https://raw.githubusercontent.com/gameap/plugin-fastdl/main/scripts/{installer_name}"
            )
        );
        assert!(
            !host
                .uploads
                .iter()
                .any(|(_, path, _)| path.ends_with(installer_name))
        );
    }
}

#[test]
fn installers_accept_the_options_the_plugin_passes() {
    let linux = include_str!("../../scripts/install-linux.sh");
    for option in [
        "--download-url=",
        "--sha256=",
        "--install-dir=",
        "--config=",
        "--check",
        "--help",
    ] {
        assert!(
            linux.contains(option),
            "{option} is missing from install-linux.sh"
        );
    }

    let windows = include_str!("../../scripts/install-windows.ps1");
    for parameter in [
        "$DownloadUrl",
        "$Sha256",
        "$InstallDir",
        "$ConfigPath",
        "$Check",
        "$Help",
    ] {
        assert!(
            windows.contains(parameter),
            "{parameter} is missing from install-windows.ps1"
        );
    }
}

#[test]
fn failed_setup_task_is_not_reported_as_installed() {
    let mut host = installed_host();
    let body = json!({
        "download_url": "https://releases.example/gameap-fastdl",
        "sha256": "a".repeat(64),
    });
    let response = router::dispatch(&mut host, &request(1, "POST", "/nodes/1/setup", body));
    assert_eq!(response.status_code, 202);
    let download_id = host.created_tasks[0].0;
    host.task_states.get_mut(&download_id).unwrap().status = TaskStatus::Success;
    let task_id = host.created_tasks[1].0;
    host.task_states.get_mut(&task_id).unwrap().status = TaskStatus::Error;
    assert_eq!(
        node_setup::get_status(&mut host, 1).unwrap().status,
        SetupStatus::Failed
    );
}

#[test]
fn windows_helper_preserves_spaces_and_never_invokes_a_shell_for_paths() {
    let mut host = installed_host();
    let node = host.nodes.get_mut(&1).unwrap();
    node.os = "windows".into();
    node.work_path = r"C:\GameAP Data".into();
    let response = router::dispatch(
        &mut host,
        &request(1, "PUT", "/servers/3/fastdl", server_settings()),
    );
    assert_eq!(response.status_code, 200);
    assert!(host.commands[0].starts_with(concat!(
        r#""C:\GameAP Data\.plugins\fastdla\gameap-fastdl.exe" "#,
        r#"configure --root "C:\GameAP Data\servers\cs""#,
    )));
}

#[test]
fn changing_managed_configuration_removes_the_previous_block() {
    for (field, value) in [
        ("game_dir", json!("valve")),
        ("manage_game_config", json!(false)),
    ] {
        let mut host = installed_host();
        enable_server(&mut host);
        host.commands.clear();

        let mut settings = server_settings();
        settings[field] = value;
        let response =
            router::dispatch(&mut host, &request(1, "PUT", "/servers/3/fastdl", settings));
        assert_eq!(response.status_code, 200, "{field}");

        let cleanup = host.commands.first().expect("old configuration is cleaned");
        assert!(cleanup.contains("--game-dir cstrike --engine goldsource"));
        assert!(!cleanup.contains("--url"));

        let state = stored_server(&mut host, 3);
        let dropin = uploaded_server_config(&host, 1, 3);
        assert!(state.synced);
        assert_eq!(dropin["enabled"], true);

        if field == "game_dir" {
            assert_eq!(host.commands.len(), 2);
            assert!(host.commands[1].contains("--game-dir valve --engine goldsource --url"));
            assert_eq!(state.settings.game_dir, "valve");
            assert_eq!(dropin["root"], "/srv/gameap/servers/cs/valve");
        } else {
            assert_eq!(host.commands.len(), 1);
            assert!(!state.settings.manage_game_config);
            assert_eq!(dropin["root"], "/srv/gameap/servers/cs/cstrike");
        }
    }
}

#[test]
fn cleanup_failure_keeps_the_previous_route_disabled() {
    for (field, value) in [
        ("game_dir", json!("valve")),
        ("manage_game_config", json!(false)),
    ] {
        let mut host = installed_host();
        enable_server(&mut host);
        host.commands.clear();
        host.fail_on.clear();
        host.command_results.push_back(CommandOutput {
            output: "configuration is not writable".into(),
            exit_code: 1,
            error: None,
        });

        let mut settings = server_settings();
        settings[field] = value;
        let response =
            router::dispatch(&mut host, &request(1, "PUT", "/servers/3/fastdl", settings));
        assert_eq!(response.status_code, 409, "{field}");

        assert_eq!(host.commands.len(), 1);
        assert!(host.commands[0].contains("--game-dir cstrike --engine goldsource"));
        assert!(!host.commands[0].contains("--url"));
        assert_eq!(uploaded_server_config(&host, 1, 3)["enabled"], false);

        let state = stored_server(&mut host, 3);
        assert!(!state.synced);
        assert_eq!(state.settings.game_dir, "cstrike");
        assert!(state.settings.manage_game_config);
    }
}

#[test]
fn successful_installation_task_requires_a_verified_binary() {
    for (version_output, expected_status) in [
        ("gameap-fastdl 0.2.3\n", SetupStatus::Installed),
        ("another-service 0.2.3\n", SetupStatus::Failed),
    ] {
        let mut host = installed_host();
        let task_id = start_installation(&mut host);
        host.task_states.get_mut(&task_id).unwrap().status = TaskStatus::Success;
        host.command_results.push_back(CommandOutput {
            output: version_output.into(),
            exit_code: 0,
            error: None,
        });

        let response =
            router::dispatch(&mut host, &request(1, "GET", "/nodes/1/status", json!({})));
        assert_eq!(response.status_code, 200);

        let status: NodeSetupStatus =
            serde_json::from_slice(&response.body).expect("installation status is JSON");
        assert_eq!(status.status, expected_status);
        assert_eq!(status.task_id, task_id);
        assert_eq!(store::get_status(&mut host, 1).unwrap(), status);

        if expected_status == SetupStatus::Installed {
            assert_eq!(status.version, "0.2.3");
            assert!(status.error_message.is_empty());
        } else {
            assert!(status.version.is_empty());
            assert_eq!(
                status.error_message,
                "Installed binary could not be verified"
            );
        }
    }
}

#[test]
fn missing_installation_task_expires_after_the_timeout() {
    let mut host = installed_host();
    let task_id = start_installation(&mut host);
    host.task_states.remove(&task_id);
    host.now += 1800;

    let status = node_setup::get_status(&mut host, 1).unwrap();
    assert_eq!(status.status, SetupStatus::Installing);

    host.now += 1;
    let response = router::dispatch(&mut host, &request(1, "GET", "/nodes/1/status", json!({})));
    assert_eq!(response.status_code, 200);

    let status: NodeSetupStatus =
        serde_json::from_slice(&response.body).expect("installation status is JSON");
    assert_eq!(status.status, SetupStatus::Failed);
    assert_eq!(status.error_message, "Installation timed out");
    assert_eq!(store::get_status(&mut host, 1).unwrap(), status);
    assert!(host.commands.is_empty());
}

#[test]
fn node_sync_revokes_routes_for_deleted_and_moved_servers() {
    for server_moved in [false, true] {
        let mut host = installed_host();
        enable_server(&mut host);
        host.commands.clear();

        if server_moved {
            let mut destination = host.nodes.get(&1).unwrap().clone();
            destination.id = 2;
            host.nodes.insert(2, destination);
            host.servers.get_mut(&3).unwrap().node_id = 2;
        } else {
            host.servers.remove(&3);
            host.storage_delete("fastdl:server", StorageEntity::server(3))
                .unwrap();
        }

        let response = router::dispatch(&mut host, &request(1, "POST", "/nodes/1/sync", json!({})));
        assert_eq!(response.status_code, 200);
        assert_eq!(uploaded_server_config(&host, 1, 3)["enabled"], false);
        assert!(store::get_registration(&mut host, 1, 3).unwrap().is_none());
        assert_eq!(
            store::get_server_state(&mut host, 3).unwrap().is_some(),
            server_moved,
        );
        assert!(
            host.commands
                .iter()
                .all(|command| !command.contains("configure"))
        );
    }
}
