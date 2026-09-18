//! Node configuration and installation through daemon tasks.
//!
//! Installation becomes complete only after a successful task and a version
//! probe of the installed binary. Polling also expires abandoned tasks.

use crate::domain::{NodeConfig, NodeOs, NodeSetupStatus, SetupInput, SetupStatus};
use crate::host_api::{HostApi, NodeInfo, TaskStatus};
use crate::http::ApiError;
use crate::shell::{shell_join, shell_join_windows};

use super::paths::{CONFIG_PATH, PLUGIN_DIR, absolute, binary};
use super::{store, sync};

const INSTALLING_TIMEOUT_SECS: i64 = 1800;
const VERSION_PREFIX: &str = "gameap-fastdl ";
const LEGACY_VERIFICATION_ERROR: &str = "Installed binary could not be verified";
const MAX_DIAGNOSTIC_CHARS: usize = 4096;
const LINUX_RESTART_COMMAND: &str = "systemctl restart gameap-fastdl";
const WINDOWS_RESTART_COMMAND: &str = concat!(
    "powershell -NoProfile -NonInteractive -Command ",
    "\"$ErrorActionPreference='Stop'; Restart-Service -Name gameap-fastdl\"",
);

pub fn get_node<H: HostApi>(host: &mut H, node_id: u64) -> Result<NodeInfo, ApiError> {
    host.get_node(node_id)?
        .ok_or_else(|| ApiError::not_found("Node not found"))
}

pub fn get_config<H: HostApi>(host: &mut H, node_id: u64) -> Result<NodeConfig, ApiError> {
    store::get_config(host, node_id)
}

pub fn update_config<H: HostApi>(
    host: &mut H,
    node_id: u64,
    config: NodeConfig,
) -> Result<(), ApiError> {
    config.validate()?;
    let node = get_node(host, node_id)?;

    if config.public_base_url.is_empty() {
        for server in host.find_servers(&[], &[node_id])? {
            let state = store::get_server_state(host, server.id)?;
            if state.is_some_and(|state| state.settings.enabled) {
                return Err(ApiError::conflict(
                    "Disable FastDL on this node's game servers before clearing its public address",
                ));
            }
        }
    }

    store::save_config(host, node_id, &config)?;
    sync::write_node_config(host, node_id, &config)?;

    let status = store::get_status(host, node_id)?;
    if status.status == SetupStatus::Installed {
        sync::apply_node(host, node_id)?;
        restart(host, &node)?;
    }

    Ok(())
}

pub fn setup_node<H: HostApi>(
    host: &mut H,
    node_id: u64,
    input: SetupInput,
) -> Result<NodeSetupStatus, ApiError> {
    input.validate()?;
    let node = get_node(host, node_id)?;
    let (script_name, script) = match node.os_kind() {
        NodeOs::Linux => (
            "install-linux.sh",
            include_bytes!("../../scripts/install-linux.sh").as_slice(),
        ),
        NodeOs::Windows => (
            "install-windows.ps1",
            include_bytes!("../../scripts/install-windows.ps1").as_slice(),
        ),
        NodeOs::Unsupported => {
            return Err(ApiError::bad_request(
                "Only Linux and Windows nodes are supported",
            ));
        }
    };
    let script_path = format!("{PLUGIN_DIR}/{script_name}");
    let command = installation_command(&node, &input, &absolute(&node, &script_path)?)?;

    if get_status(host, node_id)?.status == SetupStatus::Installing {
        return Err(ApiError::conflict("Installation is already in progress"));
    }

    let config = get_config(host, node_id)?;
    config.validate()?;
    sync::write_node_config(host, node_id, &config)?;

    // Ship the script with the plugin so its options cannot lag behind the
    // caller or depend on a previously downloaded tool on the node.
    host.upload(node_id, &script_path, script, 0o700)?;
    let task_id = host.create_daemon_task(node_id, &command, None)?;

    let status = NodeSetupStatus {
        status: SetupStatus::Installing,
        task_id,
        started_at: host.now_unix(),
        ..Default::default()
    };
    store::save_status(host, node_id, &status)?;

    Ok(status)
}

pub fn get_status<H: HostApi>(host: &mut H, node_id: u64) -> Result<NodeSetupStatus, ApiError> {
    let mut status = store::get_status(host, node_id)?;
    let retry_legacy_verification =
        status.status == SetupStatus::Failed && status.error_message == LEGACY_VERIFICATION_ERROR;
    if status.status != SetupStatus::Installing && !retry_legacy_verification {
        return Ok(status);
    }

    refresh_installation_task(host, node_id, &mut status)?;

    if status.status == SetupStatus::Installing
        && host.now_unix().saturating_sub(status.started_at) > INSTALLING_TIMEOUT_SECS
    {
        status.status = SetupStatus::Failed;
        status.error_message = "Installation timed out".into();
    }

    store::save_status(host, node_id, &status)?;

    Ok(status)
}

pub(super) fn restart<H: HostApi>(host: &mut H, node: &NodeInfo) -> Result<(), ApiError> {
    let command = match node.os_kind() {
        NodeOs::Linux => LINUX_RESTART_COMMAND,
        NodeOs::Windows => WINDOWS_RESTART_COMMAND,
        NodeOs::Unsupported => {
            return Err(ApiError::bad_request(
                "Only Linux and Windows nodes are supported",
            ));
        }
    };

    let output = host.execute_command(node.id, command)?;
    if output.exit_code != 0 || output.error.is_some() {
        return Err(ApiError::new(
            502,
            "RESTART_FAILED",
            "FastDL service could not be restarted",
        ));
    }

    Ok(())
}

fn installation_command(
    node: &NodeInfo,
    input: &SetupInput,
    script_path: &str,
) -> Result<String, ApiError> {
    let config_path = absolute(node, CONFIG_PATH)?;
    let install_dir = absolute(node, PLUGIN_DIR)?;

    if node.os_kind() == NodeOs::Windows {
        let mut args = vec![
            "powershell",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            script_path,
        ];
        if !input.download_url.is_empty() {
            args.extend([
                "-DownloadUrl",
                &input.download_url,
                "-Sha256",
                &input.sha256,
            ]);
        }
        args.extend(["-InstallDir", &install_dir, "-ConfigPath", &config_path]);
        return Ok(shell_join_windows(&args));
    }

    // The daemon splits the command itself and never runs a shell, so the
    // interpreter is named rather than relying on the shebang: a work path
    // mounted noexec would otherwise defeat the uploaded script's mode.
    let download_url = format!("--download-url={}", input.download_url);
    let sha256 = format!("--sha256={}", input.sha256);
    let install_dir = format!("--install-dir={install_dir}");
    let config_path = format!("--config={config_path}");

    let mut args = vec!["/bin/bash", script_path];
    if !input.download_url.is_empty() {
        args.extend([download_url.as_str(), sha256.as_str()]);
    }
    args.extend([install_dir.as_str(), config_path.as_str()]);
    Ok(shell_join(&args))
}

fn refresh_installation_task<H: HostApi>(
    host: &mut H,
    node_id: u64,
    status: &mut NodeSetupStatus,
) -> Result<(), ApiError> {
    if status.download_task_id != 0 {
        let Some(download) = host.find_daemon_task(status.download_task_id)? else {
            return Ok(());
        };
        if download.node_id != node_id {
            return Err(ApiError::internal("Unexpected installation task node"));
        }
        match download.status {
            TaskStatus::Error | TaskStatus::Canceled => {
                status.status = SetupStatus::Failed;
                status.task_id = status.download_task_id;
                status.error_message =
                    "Installer download failed. Review the daemon task output.".into();
                return Ok(());
            }
            TaskStatus::Success => {}
            TaskStatus::Waiting | TaskStatus::Working | TaskStatus::Unknown => return Ok(()),
        }
    }
    let Some(task) = host.find_daemon_task(status.task_id)? else {
        return Ok(());
    };

    if task.node_id != node_id {
        return Err(ApiError::internal("Unexpected installation task node"));
    }

    match task.status {
        TaskStatus::Success => verify_installation(host, node_id, status)?,
        TaskStatus::Error | TaskStatus::Canceled => {
            status.status = SetupStatus::Failed;
            status.error_message = "Installation failed. Review the daemon task output.".into();
        }
        TaskStatus::Waiting | TaskStatus::Working | TaskStatus::Unknown => {}
    }

    Ok(())
}

fn verify_installation<H: HostApi>(
    host: &mut H,
    node_id: u64,
    status: &mut NodeSetupStatus,
) -> Result<(), ApiError> {
    let node = get_node(host, node_id)?;
    let binary_path = binary(&node)?;
    let command = if node.os_kind() == NodeOs::Windows {
        shell_join_windows(&[&binary_path, "version"])
    } else {
        shell_join(&[&binary_path, "version"])
    };

    let output = match host.execute_command(node_id, &command) {
        Ok(output) => output,
        Err(error) => {
            host.log_error(&format!(
                "[fastdl] Installation verification failed: node_id={node_id} task_id={} command={:?} host_error={:?}",
                status.task_id,
                diagnostic_detail(&command),
                diagnostic_detail(error.message()),
            ));
            return Err(error.into());
        }
    };
    let version = installation_version(&output.output);
    let failure = if output.error.is_some() {
        Some("the node could not execute the version command".to_owned())
    } else if output.exit_code != 0 {
        Some(format!(
            "the version command exited with code {}",
            output.exit_code
        ))
    } else if version.is_none() {
        Some("the command output has no valid gameap-fastdl version line".to_owned())
    } else {
        None
    };
    if let Some(reason) = failure {
        host.log_error(&format!(
            "[fastdl] Installation verification failed: node_id={node_id} task_id={} command={:?} exit_code={} error={:?} output={:?} reason={reason}",
            status.task_id,
            diagnostic_detail(&command),
            output.exit_code,
            output.error.as_deref().map(diagnostic_detail),
            diagnostic_detail(&output.output),
        ));
        status.status = SetupStatus::Failed;
        status.version.clear();
        status.error_message =
            format!("{LEGACY_VERIFICATION_ERROR}: {reason}. Check the GameAP plugin logs.");

        return Ok(());
    }

    status.status = SetupStatus::Installed;
    status.version = version.unwrap_or_default().to_owned();
    status.error_message.clear();

    Ok(())
}

fn installation_version(output: &str) -> Option<&str> {
    // Daemon responses can wrap stdout with the command prompt and exit code.
    output
        .lines()
        .filter_map(|line| line.trim().strip_prefix(VERSION_PREFIX))
        .map(str::trim)
        .find(|version| {
            !version.is_empty()
                && version.chars().count() <= 64
                && !version.chars().any(char::is_whitespace)
        })
}

fn diagnostic_detail(value: &str) -> String {
    let mut characters = value.chars();
    let mut detail: String = characters.by_ref().take(MAX_DIAGNOSTIC_CHARS).collect();
    if characters.next().is_some() {
        detail.push_str(" [truncated]");
    }
    detail
}
