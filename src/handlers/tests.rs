//! Route tests verify authorization, persisted settings and daemon-side effects.

use gameap_plugin_sdk::proto::gameap::plugin as pb;
use serde_json::{Value, json};

use crate::domain::{NodeConfig, NodeSetupStatus, ServerState, SetupInput, SetupStatus};
use crate::host_api::mock::MockHost;
use crate::host_api::{CommandOutput, HostApi, HostApiError, StorageEntity, TaskStatus};
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

fn half_life_host() -> MockHost {
    let mut host = installed_host();
    let server = host.servers.get_mut(&3).unwrap();
    server.name = "Half-Life".into();
    server.game_id = "valve".into();
    server.dir = "servers/half-life".into();
    host.fail_on.clear();
    host
}

fn half_life_settings() -> Value {
    json!({
        "enabled": true,
        "autoindex": false,
        "engine": "goldsource",
        "game_dir": "valve",
        "manage_game_config": true,
        "generate_bz2": true,
    })
}

fn configured_half_life_host(os: &str, work_path: &str) -> MockHost {
    let mut host = half_life_host();
    let node = host.nodes.get_mut(&1).unwrap();
    node.os = os.into();
    node.work_path = work_path.into();
    store::save_config(
        &mut host,
        1,
        &NodeConfig {
            listen: "127.0.0.1:9090".into(),
            public_base_url: "http://cdn.example".into(),
        },
    )
    .unwrap();
    host.command_results.push_back(CommandOutput {
        output: "{\"configured\":true}".into(),
        exit_code: 0,
        error: None,
    });
    let response = router::dispatch(
        &mut host,
        &request(1, "PUT", "/servers/3/fastdl", half_life_settings()),
    );
    assert_eq!(response.status_code, 200);
    host.commands.clear();
    host.uploads.clear();
    host
}

fn stored_server(host: &mut MockHost, server_id: u64) -> ServerState {
    store::get_server_state(host, server_id)
        .expect("server settings can be loaded")
        .expect("server settings are stored")
}

fn uploaded_server_config(host: &MockHost, node_id: u64, server_id: u64) -> Value {
    let path = format!(".plugins/i3z7ix336msd4/servers.d/server-{server_id}.json");
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

    host.created_tasks
        .last()
        .expect("installation task exists")
        .0
}

#[test]
fn explicit_configuration_uses_saved_settings_for_both_engines_and_manual_mode() {
    for engine in ["GoldSource", "Source"] {
        for manage_game_config in [true, false] {
            let mut host = installed_host();
            host.game_engines.insert("cstrike".into(), engine.into());
            let mut settings = server_settings();
            settings["manage_game_config"] = json!(manage_game_config);
            let saved =
                router::dispatch(&mut host, &request(1, "PUT", "/servers/3/fastdl", settings));
            assert_eq!(saved.status_code, 200);
            host.commands.clear();
            host.uploads.clear();
            let storage = host.storage.clone();
            host.grants
                .push((7, 3, "plugin:i3z7ix336msd4:fastdl-manage".into()));

            let response = router::dispatch(
                &mut host,
                &request(
                    7,
                    "POST",
                    "/servers/3/fastdl/configure",
                    json!({
                        "download_url": "http://untrusted.example/",
                        "game_dir": "../other-server",
                    }),
                ),
            );
            assert_eq!(response.status_code, 200);
            let body: Value = serde_json::from_slice(&response.body).unwrap();
            let saved: Value = serde_json::from_slice(&saved.body).unwrap();
            assert_eq!(body["configured"], true);
            assert_eq!(body["configuration"], saved["configuration"]);
            assert_eq!(body.as_object().unwrap().len(), 2);
            assert_eq!(host.commands.len(), 1);
            assert!(
                host.commands[0]
                    .contains("configure --root /srv/gameap/servers/cs --game-dir cstrike")
            );
            assert!(host.commands[0].contains(&format!("--engine {}", engine.to_lowercase())));
            assert!(host.commands[0].contains(saved["download_url"].as_str().unwrap()));
            assert!(!host.commands[0].contains("untrusted"));
            assert!(host.uploads.is_empty());
            assert_eq!(host.storage, storage);
        }
    }
}

#[test]
fn explicit_configuration_requires_manage_access_to_this_server() {
    for grant in [
        None,
        Some((3, "plugin:i3z7ix336msd4:fastdl-view")),
        Some((4, "plugin:i3z7ix336msd4:fastdl-manage")),
    ] {
        let mut host = installed_host();
        enable_server(&mut host);
        host.commands.clear();
        host.uploads.clear();
        if let Some((server_id, ability)) = grant {
            host.grants.push((7, server_id, ability.into()));
        }
        let response = router::dispatch(
            &mut host,
            &request(7, "POST", "/servers/3/fastdl/configure", json!({})),
        );
        assert_eq!(response.status_code, 403);
        assert!(host.commands.is_empty());
        assert!(host.uploads.is_empty());
    }

    let mut host = installed_host();
    for (user, status) in [(0, 401), (1, 502)] {
        host.authz_down = true;
        let response = router::dispatch(
            &mut host,
            &request(user, "POST", "/servers/3/fastdl/configure", json!({})),
        );
        assert_eq!(response.status_code, status);
        assert!(host.commands.is_empty());
    }
}

#[test]
fn explicit_configuration_rejects_disabled_unsaved_and_stale_settings() {
    for scenario in [
        "unsaved",
        "disabled",
        "unsynced",
        "moved-node",
        "moved-directory",
        "changed-engine",
        "disabled-server",
    ] {
        let mut host = installed_host();
        if scenario != "unsaved" {
            enable_server(&mut host);
            let mut state = stored_server(&mut host, 3);
            match scenario {
                "disabled" => state.settings.enabled = false,
                "unsynced" => state.synced = false,
                "moved-node" => host.servers.get_mut(&3).unwrap().node_id = 2,
                "moved-directory" => host.servers.get_mut(&3).unwrap().dir = "servers/other".into(),
                "changed-engine" => {
                    host.game_engines.insert("cstrike".into(), "Source".into());
                }
                "disabled-server" => host.servers.get_mut(&3).unwrap().enabled = false,
                _ => unreachable!(),
            }
            store::save_server_state(&mut host, 3, &state).unwrap();
        }
        host.commands.clear();
        host.uploads.clear();

        let response = router::dispatch(
            &mut host,
            &request(1, "POST", "/servers/3/fastdl/configure", json!({})),
        );
        assert_eq!(response.status_code, 409, "{scenario}");
        let body: Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(body["code"], "CONFIGURATION_NOT_READY", "{scenario}");
        assert!(host.commands.is_empty(), "{scenario}");
        assert!(host.uploads.is_empty(), "{scenario}");
    }
}

#[test]
fn explicit_configuration_requires_an_installed_node_public_address_and_supported_game() {
    for scenario in [
        "not-installed",
        "no-address",
        "unsupported",
        "missing-server",
    ] {
        let mut host = installed_host();
        enable_server(&mut host);
        host.commands.clear();
        let expected_status = match scenario {
            "not-installed" => {
                store::save_status(&mut host, 1, &NodeSetupStatus::default()).unwrap();
                409
            }
            "no-address" => {
                store::save_config(&mut host, 1, &NodeConfig::default()).unwrap();
                409
            }
            "unsupported" => {
                host.game_engines.insert("cstrike".into(), "Source2".into());
                400
            }
            "missing-server" => {
                host.servers.remove(&3);
                404
            }
            _ => unreachable!(),
        };
        let response = router::dispatch(
            &mut host,
            &request(1, "POST", "/servers/3/fastdl/configure", json!({})),
        );
        assert_eq!(response.status_code, expected_status, "{scenario}");
        assert!(host.commands.is_empty(), "{scenario}");
    }
}

#[test]
fn explicit_configuration_reports_helper_failure_without_returning_rcon_commands() {
    for (output, exit_code, status, code) in [
        ("permission denied", 1, 409, "GAME_CONFIG_UPDATE_FAILED"),
        ("", 0, 502, "CONFIGURE_FAILED"),
    ] {
        let mut host = installed_host();
        enable_server(&mut host);
        host.commands.clear();
        host.uploads.clear();
        host.fail_on.clear();
        let storage = host.storage.clone();
        host.command_results.push_back(CommandOutput {
            output: output.into(),
            exit_code,
            error: None,
        });

        let response = router::dispatch(
            &mut host,
            &request(1, "POST", "/servers/3/fastdl/configure", json!({})),
        );
        assert_eq!(response.status_code, status);
        let body: Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(body["code"], code);
        assert!(body.get("configuration").is_none());
        assert_eq!(host.commands.len(), 1);
        assert!(host.uploads.is_empty());
        assert_eq!(host.storage, storage);
    }
}

#[test]
fn game_entity_determines_the_engine_without_a_client_selection() {
    for (game_code, game_engine, expected_engine, directory) in [
        ("cstrike", "Source", "source", "cstrike"),
        ("custom-goldsource", "GoldSource", "goldsource", ""),
        ("custom-source", "source", "source", ""),
    ] {
        let mut host = installed_host();
        host.servers.get_mut(&3).unwrap().game_id = game_code.into();
        host.game_engines
            .insert(game_code.into(), game_engine.into());

        let response = router::dispatch(
            &mut host,
            &request(1, "GET", "/servers/3/fastdl", json!({})),
        );
        assert_eq!(response.status_code, 200);
        let body: Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(body["engine"], expected_engine);
        assert_eq!(body["supported"], true);
        assert_eq!(body["game_dir"], directory);

        let mut input = server_settings();
        input.as_object_mut().unwrap().remove("engine");
        let response = router::dispatch(&mut host, &request(1, "PUT", "/servers/3/fastdl", input));
        assert_eq!(response.status_code, 200);
        let body: Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(body["engine"], expected_engine);
        assert_eq!(body["synced"], true);
        assert_eq!(
            uploaded_server_config(&host, 1, 3)["engine"],
            expected_engine
        );
        assert_eq!(
            uploaded_server_config(&host, 1, 3)["generate_bz2"],
            expected_engine == "source"
        );
        assert!(host.commands[0].contains(&format!("--engine {expected_engine}")));
    }
}

#[test]
fn game_engine_overrides_legacy_client_and_stored_selections() {
    let mut host = installed_host();
    let mut input = server_settings();
    input["engine"] = json!("source");

    let response = router::dispatch(&mut host, &request(1, "PUT", "/servers/3/fastdl", input));
    assert_eq!(response.status_code, 200);
    assert_eq!(uploaded_server_config(&host, 1, 3)["engine"], "goldsource");

    host.game_engines.insert("cstrike".into(), "Source".into());
    let response = router::dispatch(
        &mut host,
        &request(1, "GET", "/servers/3/fastdl", json!({})),
    );
    assert_eq!(response.status_code, 200);
    let body: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(body["engine"], "source");
    assert_eq!(body["synced"], false);

    let response = router::dispatch(
        &mut host,
        &request(1, "PUT", "/servers/3/fastdl", server_settings()),
    );
    assert_eq!(response.status_code, 200);
    assert_eq!(uploaded_server_config(&host, 1, 3)["engine"], "source");
    assert_eq!(
        stored_server(&mut host, 3).settings.engine.as_str(),
        "source"
    );
}

#[test]
fn unsupported_or_missing_game_cannot_enable_fastdl() {
    for engine in [Some("Source2"), None] {
        let mut host = installed_host();
        match engine {
            Some(engine) => {
                host.game_engines.insert("cstrike".into(), engine.into());
            }
            None => {
                host.game_engines.remove("cstrike");
            }
        }

        let response = router::dispatch(
            &mut host,
            &request(1, "GET", "/servers/3/fastdl", json!({})),
        );
        assert_eq!(response.status_code, 200);
        let body: Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(body["supported"], false);

        let response = router::dispatch(
            &mut host,
            &request(1, "PUT", "/servers/3/fastdl", server_settings()),
        );
        assert_eq!(response.status_code, 400);
        assert!(host.commands.is_empty());
        assert!(host.uploads.is_empty());
        assert!(store::get_server_state(&mut host, 3).unwrap().is_none());
    }
}

#[test]
fn changing_to_an_unsupported_game_revokes_the_existing_publication() {
    for engine in [Some("Source2"), None] {
        for operation in ["server_updated", "node_sync", "save"] {
            let mut host = installed_host();
            enable_server(&mut host);
            host.commands.clear();
            host.servers.get_mut(&3).unwrap().game_id = "another-game".into();
            if let Some(engine) = engine {
                host.game_engines
                    .insert("another-game".into(), engine.into());
            }

            match operation {
                "server_updated" => assert!(sync::on_updated(&mut host, 3).is_err()),
                "node_sync" => assert!(sync::sync_node(&mut host, 1).is_err()),
                _ => {
                    let response = router::dispatch(
                        &mut host,
                        &request(1, "PUT", "/servers/3/fastdl", server_settings()),
                    );
                    assert_eq!(response.status_code, 400);
                }
            }

            assert_eq!(uploaded_server_config(&host, 1, 3)["enabled"], false);
            assert!(!stored_server(&mut host, 3).synced);
            assert!(host.commands.is_empty());

            let response = router::dispatch(
                &mut host,
                &request(1, "GET", "/servers/3/fastdl", json!({})),
            );
            assert_eq!(response.status_code, 200);
            let body: Value = serde_json::from_slice(&response.body).unwrap();
            assert_eq!(body["supported"], false);
            assert_eq!(body["synced"], false);
        }
    }
}

#[test]
fn denies_cross_server_access_before_read_or_write() {
    let mut host = installed_host();
    host.grants
        .push((7, 4, "plugin:i3z7ix336msd4:fastdl-manage".into()));
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
        .push((7, 3, "plugin:i3z7ix336msd4:fastdl-view".into()));
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
fn half_life_configuration_accepts_plain_and_decorated_confirmation() {
    for (os, work_path, output, expected_root) in [
        (
            "linux",
            "/srv/gameap",
            "{\"configured\":true}",
            "/srv/gameap/servers/half-life/valve",
        ),
        (
            "linux",
            "/srv/gameap",
            "{\n  \"configured\": true\n}\n",
            "/srv/gameap/servers/half-life/valve",
        ),
        (
            "linux",
            "/srv/gameap",
            concat!(
                "/srv/gameap# /srv/gameap/.plugins/i3z7ix336msd4/gameap-fastdl configure ",
                "--root /srv/gameap/servers/half-life --game-dir valve --engine goldsource\n\n",
                "{\"configured\":true}\n\nExited with 0\n",
            ),
            "/srv/gameap/servers/half-life/valve",
        ),
        (
            "windows",
            r"C:\GameAP Data",
            concat!(
                "C:\\GameAP Data# \"C:\\GameAP Data\\.plugins\\i3z7ix336msd4\\gameap-fastdl.exe\" ",
                "configure --root \"C:\\GameAP Data\\servers\\half-life\" ",
                "--game-dir valve --engine goldsource\r\n\r\n",
                "  {\"configured\":true} \r\n\r\nExited with 0\r\n",
            ),
            r"C:\GameAP Data\servers\half-life\valve",
        ),
    ] {
        let mut host = half_life_host();
        let node = host.nodes.get_mut(&1).unwrap();
        node.os = os.into();
        node.work_path = work_path.into();
        host.command_results.push_back(CommandOutput {
            output: output.into(),
            exit_code: 0,
            error: None,
        });

        let response = router::dispatch(
            &mut host,
            &request(1, "PUT", "/servers/3/fastdl", half_life_settings()),
        );
        assert_eq!(response.status_code, 200, "{output}");
        let body: Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(body["synced"], true);
        assert_eq!(body["enabled"], true);
        assert_eq!(body["server_name"], "Half-Life");
        assert_eq!(body["engine"], "goldsource");
        assert_eq!(body["game_dir"], "valve");

        let state = stored_server(&mut host, 3);
        assert!(state.synced);
        assert_eq!(
            serde_json::to_value(&state.settings).unwrap(),
            half_life_settings()
        );
        let dropin = uploaded_server_config(&host, 1, 3);
        assert_eq!(dropin["enabled"], true);
        assert_eq!(dropin["root"], expected_root);
        assert_eq!(dropin["engine"], "goldsource");
        assert_eq!(dropin["generate_bz2"], false);
        assert_eq!(host.commands.len(), 1);
        assert!(host.commands[0].contains("--game-dir valve --engine goldsource --url"));
        assert!(host.logs.is_empty());
    }
}

#[test]
fn unverified_configuration_keeps_half_life_disabled_without_exposing_diagnostics() {
    for confirmation in [
        "",
        "confirmation missing",
        "{invalid JSON}",
        "{\"configured\":false}",
        "{\"configured\":\"true\"}",
        "{\"configured\":true}\n{\"configured\":true}",
        "{\"configured\":true}\n{\"configured\":false}",
        "status: {\"configured\":true}",
    ] {
        let mut host = half_life_host();
        let token = format!("{:032x}", 1);
        let output = if confirmation.is_empty() {
            String::new()
        } else {
            format!(
                "/srv/private/configure --url http://cdn.example/{token}/\n\n{confirmation}\n\nExited with 0\n"
            )
        };
        host.command_results.push_back(CommandOutput {
            output,
            exit_code: 0,
            error: None,
        });

        let response = router::dispatch(
            &mut host,
            &request(1, "PUT", "/servers/3/fastdl", half_life_settings()),
        );
        assert_eq!(response.status_code, 502, "{confirmation}");
        let body: Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(body["code"], "CONFIGURE_FAILED", "{confirmation}");
        assert!(body.get("server_name").is_none());
        let public_body = String::from_utf8_lossy(&response.body);
        assert!(!public_body.contains("/srv/private"));
        assert!(!public_body.contains(&token));
        assert!(!public_body.contains("Exited with"));
        assert!(!public_body.contains("configured"));

        let state = stored_server(&mut host, 3);
        assert!(!state.synced);
        assert!(state.settings.enabled);
        assert_eq!(state.token, token);
        assert_eq!(uploaded_server_config(&host, 1, 3)["enabled"], false);
        assert_eq!(host.logs.len(), 1);
        assert!(!host.logs[0].contains(&token));
        assert!(host.logs[0].contains("exit_code=0"));
    }
}

#[test]
fn failed_configuration_command_cannot_be_overridden_by_a_success_confirmation() {
    for (exit_code, error) in [(7, None), (0, Some("private daemon error".into()))] {
        let mut host = half_life_host();
        host.command_results.push_back(CommandOutput {
            output: "{\"configured\":true}".into(),
            exit_code,
            error,
        });

        let response = router::dispatch(
            &mut host,
            &request(1, "PUT", "/servers/3/fastdl", half_life_settings()),
        );
        assert_eq!(response.status_code, 409);
        let body: Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(body["code"], "GAME_CONFIG_UPDATE_FAILED");
        assert!(!String::from_utf8_lossy(&response.body).contains("private daemon error"));
        assert!(!stored_server(&mut host, 3).synced);
        assert_eq!(uploaded_server_config(&host, 1, 3)["enabled"], false);
        assert_eq!(host.logs.len(), 1);
        assert!(host.logs[0].contains(&format!("exit_code={exit_code}")));
    }
}

#[test]
fn configuration_diagnostics_are_bounded_and_redact_the_download_token() {
    let mut host = half_life_host();
    let token = format!("{:032x}", 1);
    host.command_results.push_back(CommandOutput {
        output: format!(
            "/srv/private/configure --url http://cdn.example/{token}/\n\u{1b}[31m{}",
            "ж".repeat(10_000),
        ),
        exit_code: 0,
        error: None,
    });

    let response = router::dispatch(
        &mut host,
        &request(1, "PUT", "/servers/3/fastdl", half_life_settings()),
    );
    assert_eq!(response.status_code, 502);
    assert_eq!(host.logs.len(), 1);
    let log = &host.logs[0];
    assert!(!log.contains(&token));
    assert!(!log.contains('\u{1b}'));
    assert_eq!(log.lines().count(), 1);
    assert!(log.chars().count() < 6000);
}

#[test]
fn retrying_unverified_half_life_configuration_applies_the_same_settings() {
    let mut host = half_life_host();
    host.command_results.extend([
        CommandOutput {
            output: "confirmation missing".into(),
            exit_code: 0,
            error: None,
        },
        CommandOutput {
            output: "/srv/gameap# configure\n\n{\"configured\":true}\n\nExited with 0\n".into(),
            exit_code: 0,
            error: None,
        },
    ]);

    let settings = half_life_settings();
    let failed = router::dispatch(
        &mut host,
        &request(1, "PUT", "/servers/3/fastdl", settings.clone()),
    );
    assert_eq!(failed.status_code, 502);
    let pending = stored_server(&mut host, 3);
    assert!(!pending.synced);
    assert_eq!(uploaded_server_config(&host, 1, 3)["enabled"], false);

    let retried = router::dispatch(&mut host, &request(1, "PUT", "/servers/3/fastdl", settings));
    assert_eq!(retried.status_code, 200);
    let state = stored_server(&mut host, 3);
    assert!(state.synced);
    assert_eq!(state.settings, pending.settings);
    assert_eq!(state.token, pending.token);
    assert_eq!(uploaded_server_config(&host, 1, 3)["enabled"], true);
    assert_eq!(host.commands.len(), 2);
    assert!(host.command_results.is_empty());
    assert_eq!(host.logs.len(), 1);
}

#[test]
fn admin_node_routes_apply_half_life_configuration_before_restarting() {
    for (os, work_path, newline, restart) in [
        (
            "linux",
            "/srv/gameap",
            "\n",
            "systemctl restart gameap-fastdl",
        ),
        (
            "windows",
            r"C:\GameAP Data",
            "\r\n",
            "Restart-Service -Name gameap-fastdl",
        ),
    ] {
        for (method, path) in [("PUT", "/nodes/1/config"), ("POST", "/nodes/1/sync")] {
            let mut host = configured_half_life_host(os, work_path);
            let config = NodeConfig {
                listen: "0.0.0.0:8080".into(),
                public_base_url: "http://192.0.2.10:8080".into(),
            };
            let body = if method == "PUT" {
                serde_json::to_value(&config).unwrap()
            } else {
                store::save_config(&mut host, 1, &config).unwrap();
                json!({})
            };
            host.command_results.push_back(CommandOutput {
                output: format!(
                    "{work_path}# gameap-fastdl configure{newline}{newline}{{\"configured\":true}}{newline}{newline}Exited with 0{newline}"
                ),
                exit_code: 0,
                error: None,
            });

            let response = router::dispatch(&mut host, &request(1, method, path, body.clone()));
            assert_eq!(response.status_code, 200, "{os} {method} {path}");
            let response_body: Value = serde_json::from_slice(&response.body).unwrap();
            if method == "PUT" {
                assert_eq!(response_body, body);
            } else {
                assert_eq!(response_body["synced"], true);
            }
            let saved = store::get_config(&mut host, 1).unwrap();
            assert_eq!(saved.listen, config.listen);
            assert_eq!(saved.public_base_url, config.public_base_url);
            let daemon_config: Value =
                serde_json::from_slice(host.file(1, ".plugins/i3z7ix336msd4/config.json").unwrap())
                    .unwrap();
            assert_eq!(daemon_config["listen"], config.listen);

            let state = stored_server(&mut host, 3);
            assert!(state.synced);
            assert!(state.settings.enabled);
            assert_eq!(uploaded_server_config(&host, 1, 3)["enabled"], true);
            assert_eq!(host.commands.len(), 2, "{:?}", host.commands);
            assert!(host.commands[0].contains("--game-dir valve --engine goldsource"));
            assert!(host.commands[0].contains(&format!(
                "--url {}/{}/",
                config.public_base_url, state.token
            )));
            assert!(host.commands[1].contains(restart), "{:?}", host.commands);
            assert!(host.logs.is_empty());
        }
    }
}

#[test]
fn admin_node_routes_skip_restart_after_unverified_configuration_and_allow_retry() {
    for (method, path) in [("PUT", "/nodes/1/config"), ("POST", "/nodes/1/sync")] {
        let mut host = configured_half_life_host("linux", "/srv/gameap");
        let config = NodeConfig {
            listen: "0.0.0.0:8080".into(),
            public_base_url: "http://192.0.2.10:8080".into(),
        };
        let body = if method == "PUT" {
            serde_json::to_value(&config).unwrap()
        } else {
            store::save_config(&mut host, 1, &config).unwrap();
            json!({})
        };
        host.command_results.push_back(CommandOutput {
            output: "/srv/private# configure\n\n{invalid}\n\nExited with 0\n".into(),
            exit_code: 0,
            error: None,
        });

        let response = router::dispatch(&mut host, &request(1, method, path, body.clone()));
        assert_eq!(response.status_code, 502, "{method} {path}");
        let error_body: Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(error_body["code"], "CONFIGURE_FAILED");
        assert_eq!(error_body["server_name"], "Half-Life");
        assert!(
            error_body["message"]
                .as_str()
                .unwrap()
                .contains("Half-Life")
        );
        for field in ["server_id", "node_id", "token"] {
            assert!(error_body.get(field).is_none());
        }
        assert!(!String::from_utf8_lossy(&response.body).contains("/srv/private"));
        assert!(!String::from_utf8_lossy(&response.body).contains("{invalid}"));
        let pending = stored_server(&mut host, 3);
        assert!(!String::from_utf8_lossy(&response.body).contains(&pending.token));
        assert!(!pending.synced);
        assert!(pending.settings.enabled);
        assert_eq!(uploaded_server_config(&host, 1, 3)["enabled"], false);
        assert_eq!(host.commands.len(), 1);
        assert!(host.commands[0].contains(" configure "));
        assert_eq!(
            store::get_config(&mut host, 1).unwrap().public_base_url,
            config.public_base_url
        );

        host.commands.clear();
        host.command_results.push_back(CommandOutput {
            output: "/srv/gameap# configure\n\n{\"configured\":true}\n\nExited with 0\n".into(),
            exit_code: 0,
            error: None,
        });
        let retried = router::dispatch(&mut host, &request(1, method, path, body));
        assert_eq!(retried.status_code, 200, "{method} {path}");
        let state = stored_server(&mut host, 3);
        assert!(state.synced);
        assert_eq!(state.settings, pending.settings);
        assert_eq!(state.token, pending.token);
        assert_eq!(uploaded_server_config(&host, 1, 3)["enabled"], true);
        assert_eq!(host.commands.len(), 2);
        assert!(host.commands[0].contains(" configure "));
        assert_eq!(host.commands[1], "systemctl restart gameap-fastdl");
        assert_eq!(host.logs.len(), 1);
    }
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
            .filter(|(_, path)| path.starts_with(".plugins/i3z7ix336msd4/servers.d/"))
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
fn automatic_installation_replaces_stale_scripts_and_keeps_saved_configuration() {
    for os in ["linux", " Windows ", ""] {
        for empty_body in [false, true] {
            let mut host = installed_host();
            let windows = os == " Windows ";
            let node = host.nodes.get_mut(&1).unwrap();
            node.os = os.into();
            node.work_path = if windows {
                r"C:\GameAP Data".into()
            } else {
                "/srv/gameap data".into()
            };
            let (script_name, script) = if windows {
                (
                    "install-windows.ps1",
                    include_bytes!("../../scripts/install-windows.ps1").as_slice(),
                )
            } else {
                (
                    "install-linux.sh",
                    include_bytes!("../../scripts/install-linux.sh").as_slice(),
                )
            };
            let private_path = format!(".plugins/i3z7ix336msd4/{script_name}");
            let tools_path = format!("tools/{script_name}");
            let stale = b"Error: --download-url, --sha256, --install-dir and --config are required";
            host.files.insert((1, private_path.clone()), stale.to_vec());
            host.files.insert((1, tools_path.clone()), stale.to_vec());

            let mut req = request(1, "POST", "/nodes/1/setup", json!({}));
            if empty_body {
                req.body.clear();
            }
            let response = router::dispatch(&mut host, &req);
            assert_eq!(response.status_code, 202);
            assert_eq!(host.created_tasks.len(), 1);
            let install = &host.created_tasks[0];
            assert_eq!(install.3, None);
            let expected = if windows {
                concat!(
                    "powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass ",
                    r#"-File "C:\GameAP Data\.plugins\i3z7ix336msd4\install-windows.ps1" "#,
                    r#"-InstallDir "C:\GameAP Data\.plugins\i3z7ix336msd4" "#,
                    r#"-ConfigPath "C:\GameAP Data\.plugins\i3z7ix336msd4\config.json""#,
                )
            } else {
                concat!(
                    "/bin/bash '/srv/gameap data/.plugins/i3z7ix336msd4/install-linux.sh' ",
                    "'--install-dir=/srv/gameap data/.plugins/i3z7ix336msd4' ",
                    "'--config=/srv/gameap data/.plugins/i3z7ix336msd4/config.json'",
                )
            };
            assert_eq!(install.2, expected);
            let status: NodeSetupStatus = serde_json::from_slice(&response.body).unwrap();
            assert_eq!(status.task_id, install.0);
            assert_eq!(status.download_task_id, 0);
            assert_eq!(status, store::get_status(&mut host, 1).unwrap());
            assert_eq!(
                store::get_config(&mut host, 1).unwrap().public_base_url,
                "http://cdn.example"
            );
            assert_eq!(host.files.get(&(1, private_path.clone())).unwrap(), script);
            assert_eq!(host.files.get(&(1, tools_path)).unwrap(), stale);
            assert!(host.uploads.contains(&(1, private_path, 0o700)));
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
fn automatic_installation_rejects_duplicate_setup() {
    let mut host = installed_host();
    let req = request(1, "POST", "/nodes/1/setup", json!({}));
    assert_eq!(router::dispatch(&mut host, &req).status_code, 202);
    let uploads = host.uploads.len();
    assert_eq!(router::dispatch(&mut host, &req).status_code, 409);
    assert_eq!(host.created_tasks.len(), 1);
    assert_eq!(host.uploads.len(), uploads);
    assert!(host.commands.is_empty());
}

fn legacy_installation(host: &mut MockHost) -> (u64, u64) {
    let download_id = host
        .create_daemon_task(1, "get-tool https://example.com/install-linux.sh", None)
        .unwrap();
    let install_id = host
        .create_daemon_task(
            1,
            "/bin/bash /srv/gameap/tools/install-linux.sh",
            Some(download_id),
        )
        .unwrap();
    store::save_status(
        host,
        1,
        &NodeSetupStatus {
            status: SetupStatus::Installing,
            task_id: install_id,
            download_task_id: download_id,
            started_at: host.now,
            ..Default::default()
        },
    )
    .unwrap();
    (download_id, install_id)
}

#[test]
fn legacy_installation_still_waits_for_download() {
    let mut host = installed_host();
    let (download_id, install_id) = legacy_installation(&mut host);
    host.task_states.get_mut(&install_id).unwrap().status = TaskStatus::Success;
    assert_eq!(
        node_setup::get_status(&mut host, 1).unwrap().status,
        SetupStatus::Installing
    );
    assert!(host.commands.is_empty());
    host.task_states.get_mut(&download_id).unwrap().status = TaskStatus::Success;
    host.command_results.push_back(CommandOutput {
        output: "gameap-fastdl 0.2.3\n".into(),
        exit_code: 0,
        error: None,
    });
    assert_eq!(
        node_setup::get_status(&mut host, 1).unwrap().status,
        SetupStatus::Installed
    );
}

#[test]
fn failed_legacy_installer_download_fails_setup_without_probing_the_binary() {
    for failure in [TaskStatus::Error, TaskStatus::Canceled] {
        let mut host = installed_host();
        let (download_id, _) = legacy_installation(&mut host);
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
fn missing_legacy_installer_download_task_expires() {
    let mut host = installed_host();
    let (download_id, _) = legacy_installation(&mut host);
    host.task_states.remove(&download_id);
    host.now += 1801;
    let status = node_setup::get_status(&mut host, 1).unwrap();
    assert_eq!(status.status, SetupStatus::Failed);
    assert_eq!(status.error_message, "Installation timed out");
}

#[test]
fn retrying_failed_legacy_installation_uses_the_bundled_script() {
    let mut host = installed_host();
    let (download_id, install_id) = legacy_installation(&mut host);
    host.task_states.get_mut(&download_id).unwrap().status = TaskStatus::Success;
    let failed_install = host.task_states.get_mut(&install_id).unwrap();
    failed_install.status = TaskStatus::Error;
    failed_install.output =
        "Error: --download-url, --sha256, --install-dir and --config are required".into();

    let response = router::dispatch(&mut host, &request(1, "POST", "/nodes/1/setup", json!({})));
    assert_eq!(response.status_code, 202);
    assert_eq!(host.created_tasks.len(), 3);
    let retry = host.created_tasks.last().unwrap();
    assert_eq!(retry.3, None);
    assert!(
        retry
            .2
            .starts_with("/bin/bash /srv/gameap/.plugins/i3z7ix336msd4/install-linux.sh ")
    );
    let status: NodeSetupStatus = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(status.status, SetupStatus::Installing);
    assert_eq!(status.task_id, retry.0);
    assert_eq!(status.download_task_id, 0);
    assert!(status.error_message.is_empty());
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
        assert_eq!(host.created_tasks.len(), 1);
        assert_eq!(host.created_tasks[0].3, None);

        // Pinned in full: a substring check passes just as happily when the
        // caller and the installer disagree about the argument names.
        let digest = "a".repeat(64);
        let expected = if os == "windows" {
            format!(
                concat!(
                    "powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass ",
                    r#"-File "C:\GameAP Data\.plugins\i3z7ix336msd4\install-windows.ps1" "#,
                    "-DownloadUrl https://releases.example/gameap-fastdl ",
                    "-Sha256 {digest} ",
                    r#"-InstallDir "C:\GameAP Data\.plugins\i3z7ix336msd4" "#,
                    r#"-ConfigPath "C:\GameAP Data\.plugins\i3z7ix336msd4\config.json""#,
                ),
                digest = digest,
            )
        } else {
            format!(
                concat!(
                    "/bin/bash /srv/gameap/.plugins/i3z7ix336msd4/install-linux.sh ",
                    "--download-url=https://releases.example/gameap-fastdl ",
                    "--sha256={digest} ",
                    "--install-dir=/srv/gameap/.plugins/i3z7ix336msd4 ",
                    "--config=/srv/gameap/.plugins/i3z7ix336msd4/config.json",
                ),
                digest = digest,
            )
        };
        assert_eq!(host.created_tasks[0].2, expected);

        let installer_name = if os == "windows" {
            "install-windows.ps1"
        } else {
            "install-linux.sh"
        };
        assert!(
            host.uploads
                .iter()
                .any(|(_, path, mode)| path.ends_with(installer_name) && *mode == 0o700)
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
    let task_id = host.created_tasks[0].0;
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
        r#""C:\GameAP Data\.plugins\i3z7ix336msd4\gameap-fastdl.exe" "#,
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
    for (os, work_path, version_output, expected_version) in [
        ("linux", "/srv/gameap", "gameap-fastdl 0.2.3\n", "0.2.3"),
        (
            "linux",
            "/srv/gameap",
            concat!(
                "/srv/gameap# /srv/gameap/.plugins/i3z7ix336msd4/gameap-fastdl version\n\n",
                "gameap-fastdl v0.0.1\n\nExited with 0\n",
            ),
            "v0.0.1",
        ),
        (
            "windows",
            r"C:\GameAP",
            concat!(
                "C:\\GameAP# C:\\GameAP\\.plugins\\i3z7ix336msd4\\gameap-fastdl.exe version\r\n\r\n",
                "  gameap-fastdl v0.0.1 \r\n\r\nExited with 0\r\n",
            ),
            "v0.0.1",
        ),
    ] {
        let mut host = installed_host();
        let node = host.nodes.get_mut(&1).unwrap();
        node.os = os.into();
        node.work_path = work_path.into();
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
        assert_eq!(status.status, SetupStatus::Installed, "{version_output}");
        assert_eq!(status.task_id, task_id);
        assert_eq!(store::get_status(&mut host, 1).unwrap(), status);
        assert_eq!(status.version, expected_version);
        assert!(status.error_message.is_empty());
        assert!(host.logs.is_empty());
    }
}

#[test]
fn installation_verification_rejects_invalid_versions_and_failed_commands() {
    for (output, exit_code, error) in [
        ("another-service 0.2.3\n".into(), 0, None),
        ("gameap-fastdl \n".into(), 0, None),
        ("gameap-fastdl v0.2.3 unexpected\n".into(), 0, None),
        (format!("gameap-fastdl {}\n", "v".repeat(65)), 0, None),
        (
            "/srv/gameap# /srv/gameap/.plugins/i3z7ix336msd4/gameap-fastdl version\n\nExited with 0\n"
                .into(),
            0,
            None,
        ),
        ("gameap-fastdl v0.2.3\n".into(), 9, None),
        (
            "gameap-fastdl v0.2.3\n".into(),
            0,
            Some("private daemon error".into()),
        ),
    ] {
        let mut host = installed_host();
        let task_id = start_installation(&mut host);
        host.task_states.get_mut(&task_id).unwrap().status = TaskStatus::Success;
        host.command_results.push_back(CommandOutput {
            output,
            exit_code,
            error,
        });

        let status = node_setup::get_status(&mut host, 1).unwrap();
        assert_eq!(status.status, SetupStatus::Failed);
        assert!(status.version.is_empty());
        assert!(
            status
                .error_message
                .starts_with("Installed binary could not be verified:")
        );
        assert!(
            status
                .error_message
                .contains("Check the GameAP plugin logs.")
        );
        if exit_code != 0 {
            assert!(status.error_message.contains(&format!("code {exit_code}")));
        }
        assert!(!status.error_message.contains("private daemon error"));
        assert!(!status.error_message.contains("/srv/gameap"));
        assert_eq!(store::get_status(&mut host, 1).unwrap(), status);
        assert_eq!(host.logs.len(), 1);
        let log = &host.logs[0];
        assert!(
            log.starts_with("ERROR [fastdl] Installation verification failed:"),
            "{log}"
        );
        assert!(log.contains("node_id=1"), "{log}");
        assert!(log.contains(&format!("task_id={task_id}")), "{log}");
        assert!(log.contains(&format!("exit_code={exit_code}")), "{log}");
        assert!(
            log.contains("/srv/gameap/.plugins/i3z7ix336msd4/gameap-fastdl version"),
            "{log}"
        );

        assert_eq!(node_setup::get_status(&mut host, 1).unwrap(), status);
        assert_eq!(host.commands.len(), 1);
        assert_eq!(host.logs.len(), 1);
    }
}

#[test]
fn installation_verification_details_are_escaped_bounded_and_private() {
    let mut host = installed_host();
    let task_id = start_installation(&mut host);
    host.task_states.get_mut(&task_id).unwrap().status = TaskStatus::Success;
    host.command_results.push_back(CommandOutput {
        output: format!("private output\n\r\t{}private tail", "я".repeat(5000)),
        exit_code: 7,
        error: Some("private daemon error\nforged log line".into()),
    });

    let response = router::dispatch(&mut host, &request(1, "GET", "/nodes/1/status", json!({})));
    assert_eq!(response.status_code, 200);
    let body = String::from_utf8(response.body).unwrap();
    assert!(!body.contains("private"));
    assert!(!body.contains("/srv/gameap"));
    assert!(!body.contains("gameap-fastdl version"));
    let status = store::get_status(&mut host, 1).unwrap();
    assert!(status.error_message.contains("node could not execute"));

    assert_eq!(host.logs.len(), 1);
    let log = &host.logs[0];
    assert!(log.contains("private output\\n\\r\\t"), "{log}");
    assert!(
        log.contains("private daemon error\\nforged log line"),
        "{log}"
    );
    assert!(log.contains(" [truncated]"));
    assert!(!log.contains("private tail"));
    assert_eq!(log.lines().count(), 1);
    assert!(log.chars().count() < 4600);
}

#[test]
fn installation_verification_transport_failure_is_logged_and_retryable() {
    let mut host = installed_host();
    let task_id = start_installation(&mut host);
    host.task_states.get_mut(&task_id).unwrap().status = TaskStatus::Success;
    host.command_error = Some(HostApiError::Call(
        "private connection detail\nfailed".into(),
    ));

    let response = router::dispatch(&mut host, &request(1, "GET", "/nodes/1/status", json!({})));
    assert_eq!(response.status_code, 502);
    assert!(!String::from_utf8_lossy(&response.body).contains("private connection detail"));
    assert_eq!(
        store::get_status(&mut host, 1).unwrap().status,
        SetupStatus::Installing
    );
    assert!(host.logs.iter().any(|log| {
        log.starts_with("ERROR [fastdl] Installation verification failed:")
            && log.contains("node_id=1")
            && log.contains(&format!("task_id={task_id}"))
            && log.contains("private connection detail\\nfailed")
    }));

    host.command_results.push_back(CommandOutput {
        output: "gameap-fastdl v0.0.1\n".into(),
        exit_code: 0,
        error: None,
    });
    assert_eq!(
        node_setup::get_status(&mut host, 1).unwrap().status,
        SetupStatus::Installed
    );
    assert_eq!(host.commands.len(), 2);
}

#[test]
fn legacy_verification_failure_is_rechecked_without_reinstalling() {
    for path in ["/nodes/1/status", "/admin/nodes"] {
        let mut host = installed_host();
        let task_id = start_installation(&mut host);
        host.task_states.get_mut(&task_id).unwrap().status = TaskStatus::Success;
        let mut status = store::get_status(&mut host, 1).unwrap();
        status.status = SetupStatus::Failed;
        status.error_message = "Installed binary could not be verified".into();
        store::save_status(&mut host, 1, &status).unwrap();
        host.uploads.clear();
        host.command_results.push_back(CommandOutput {
            output: "/srv/gameap# gameap-fastdl version\n\ngameap-fastdl v0.0.1\n\nExited with 0\n"
                .into(),
            exit_code: 0,
            error: None,
        });

        let response = router::dispatch(&mut host, &request(1, "GET", path, json!({})));
        assert_eq!(response.status_code, 200, "{path}");
        let status = store::get_status(&mut host, 1).unwrap();
        assert_eq!(status.status, SetupStatus::Installed, "{path}");
        assert_eq!(status.version, "v0.0.1");
        assert!(status.error_message.is_empty());
        assert!(host.uploads.is_empty());
        assert_eq!(host.created_tasks.len(), 1);
        assert_eq!(host.commands.len(), 1);
    }
}

#[test]
fn unrelated_installation_failures_are_not_rechecked() {
    for (task_status, error_message) in [
        (TaskStatus::Success, "Installation timed out"),
        (TaskStatus::Error, "Installed binary could not be verified"),
    ] {
        let mut host = installed_host();
        let task_id = start_installation(&mut host);
        host.task_states.get_mut(&task_id).unwrap().status = task_status;
        let mut status = store::get_status(&mut host, 1).unwrap();
        status.status = SetupStatus::Failed;
        status.error_message = error_message.into();
        store::save_status(&mut host, 1, &status).unwrap();

        let refreshed = node_setup::get_status(&mut host, 1).unwrap();
        assert_eq!(refreshed.status, SetupStatus::Failed);
        if task_status == TaskStatus::Success {
            assert_eq!(refreshed, status);
        }
        assert!(host.commands.is_empty());
        assert!(host.logs.is_empty());
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
