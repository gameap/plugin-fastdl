//! Host operations used by FastDL services.
//!
//! The SDK host functions are available only on wasm32. Native tests run the
//! same router and services against [`mock::MockHost`].

use crate::domain::NodeOs;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostApiError {
    /// ABI/transport failure of the host call itself.
    Call(String),
    /// The daemon/panel reported a failed operation (`success: false` / `error`).
    Op(String),
}

impl HostApiError {
    pub fn message(&self) -> &str {
        match self {
            Self::Call(message) | Self::Op(message) => message,
        }
    }

    pub fn into_message(self) -> String {
        match self {
            Self::Call(message) | Self::Op(message) => message,
        }
    }
}

pub type HostResult<T> = Result<T, HostApiError>;

/// Storage entity scope (mirrors `gameap.EntityType` + entity id).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageEntity {
    pub entity_type: i32,
    pub entity_id: u64,
}

/// `gameap.EntityType::Node`.
pub const ENTITY_NODE: i32 = 2;
/// `gameap.EntityType::Server`.
pub const ENTITY_SERVER: i32 = 6;

impl StorageEntity {
    pub fn node(id: u64) -> Self {
        Self {
            entity_type: ENTITY_NODE,
            entity_id: id,
        }
    }

    pub fn server(id: u64) -> Self {
        Self {
            entity_type: ENTITY_SERVER,
            entity_id: id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerInfo {
    pub id: u64,
    pub node_id: u64,
    pub name: String,
    pub game_id: String,
    pub dir: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeInfo {
    pub id: u64,
    pub name: String,
    pub ips: Vec<String>,
    pub work_path: String,
    /// `linux`, `windows`, `macos` or `other` as normalized by the panel;
    /// empty on hosts predating the field.
    pub os: String,
}

impl NodeInfo {
    pub fn os_kind(&self) -> NodeOs {
        NodeOs::parse(&self.os)
    }
}

/// A daemon command outcome; both the exit status and transport error are checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub output: String,
    pub exit_code: i32,
    pub error: Option<String>,
}

/// `gameap.DaemonTaskStatus`, narrowed to what this plugin branches on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Waiting,
    Working,
    Error,
    Success,
    Canceled,
    Unknown,
}

impl TaskStatus {
    pub fn from_i32(value: i32) -> Self {
        match value {
            1 => Self::Waiting,
            2 => Self::Working,
            3 => Self::Error,
            4 => Self::Success,
            5 => Self::Canceled,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonTaskInfo {
    pub id: u64,
    pub node_id: u64,
    pub status: TaskStatus,
    pub output: String,
}

pub trait HostApi {
    fn get_server(&mut self, id: u64) -> HostResult<Option<ServerInfo>>;

    /// Returns all matching, non-deleted servers without pagination or ordering.
    /// An empty filter list leaves that field unrestricted.
    fn find_servers(&mut self, ids: &[u64], node_ids: &[u64]) -> HostResult<Vec<ServerInfo>>;
    fn get_node(&mut self, id: u64) -> HostResult<Option<NodeInfo>>;

    /// Returns all non-deleted nodes without pagination or ordering.
    fn find_nodes(&mut self) -> HostResult<Vec<NodeInfo>>;

    fn execute_command(&mut self, node_id: u64, command: &str) -> HostResult<CommandOutput>;

    fn upload(
        &mut self,
        node_id: u64,
        path: &str,
        content: &[u8],
        permissions: u32,
    ) -> HostResult<()>;

    /// Creates a CMD_EXEC daemon task; returns the task id.
    fn create_daemon_task(
        &mut self,
        node_id: u64,
        command: &str,
        run_after_id: Option<u64>,
    ) -> HostResult<u64>;
    fn find_daemon_task(&mut self, task_id: u64) -> HostResult<Option<DaemonTaskInfo>>;

    fn storage_get(&mut self, key: &str, entity: StorageEntity) -> HostResult<Option<Vec<u8>>>;
    fn storage_set(&mut self, key: &str, entity: StorageEntity, payload: &[u8]) -> HostResult<()>;
    fn storage_delete(&mut self, key: &str, entity: StorageEntity) -> HostResult<()>;
    /// Entries matching a key prefix within one entity scope: (key, payload).
    fn storage_list(
        &mut self,
        key_prefix: &str,
        entity: StorageEntity,
    ) -> HostResult<Vec<(String, Vec<u8>)>>;

    fn authz_can(&mut self, user_id: u64, abilities: &[&str]) -> HostResult<bool>;
    fn authz_can_any_for_entity(
        &mut self,
        user_id: u64,
        entity: StorageEntity,
        abilities: &[&str],
    ) -> HostResult<bool>;
    fn random_string(&mut self, length: i32, charset: Option<&str>) -> HostResult<String>;

    /// Wall-clock seconds since the Unix epoch.
    fn now_unix(&mut self) -> i64;

    fn log_error(&mut self, message: &str);
}

pub struct WasmHost;

#[cfg(target_arch = "wasm32")]
mod wasm {
    use gameap_plugin_sdk::host;
    use gameap_plugin_sdk::proto::gameap::plugin::sdk::{
        authz, crypto, daemontasks, nodecmd, nodefs, nodes, servers, storage,
    };
    use gameap_plugin_sdk::proto::gameap::{DaemonTaskType, Node, Server};

    use super::{
        CommandOutput, DaemonTaskInfo, HostApi, HostApiError, HostResult, NodeInfo, ServerInfo,
        StorageEntity, TaskStatus, WasmHost,
    };

    fn call_err(err: gameap_plugin_sdk::HostError) -> HostApiError {
        HostApiError::Call(err.to_string())
    }

    fn server_info(server: Server) -> ServerInfo {
        ServerInfo {
            id: server.id,
            node_id: server.ds_id,
            name: server.name,
            game_id: server.game_id,
            dir: server.dir,
            enabled: server.enabled,
        }
    }

    fn node_info(node: Node) -> NodeInfo {
        NodeInfo {
            id: node.id,
            name: node.name,
            ips: node.ips,
            work_path: node.work_path,
            os: node.os,
        }
    }

    impl HostApi for WasmHost {
        fn get_server(&mut self, id: u64) -> HostResult<Option<ServerInfo>> {
            let response =
                host::servers::get_server(&servers::GetServerRequest { id }).map_err(call_err)?;

            if !response.found {
                return Ok(None);
            }

            Ok(response.server.map(server_info))
        }

        fn find_servers(&mut self, ids: &[u64], node_ids: &[u64]) -> HostResult<Vec<ServerInfo>> {
            // A present filter keeps the host's soft-delete guard enabled.
            let filter = Some(servers::ServerFilter {
                ids: ids.to_vec(),
                node_ids: node_ids.to_vec(),
                game_ids: Vec::new(),
                enabled: None,
                process_active: None,
                installed: None,
            });
            let response = host::servers::find_servers(&servers::FindServersRequest {
                filter,
                sorting: Vec::new(),
                pagination: None,
            })
            .map_err(call_err)?;

            Ok(response.servers.into_iter().map(server_info).collect())
        }

        fn get_node(&mut self, id: u64) -> HostResult<Option<NodeInfo>> {
            let response =
                host::nodes::get_node(&nodes::GetNodeRequest { id }).map_err(call_err)?;

            if !response.found {
                return Ok(None);
            }

            Ok(response.node.map(node_info))
        }

        fn find_nodes(&mut self) -> HostResult<Vec<NodeInfo>> {
            // A present filter keeps the host's soft-delete guard enabled.
            let response = host::nodes::find_nodes(&nodes::FindNodesRequest {
                filter: Some(nodes::NodeFilter::default()),
                sorting: Vec::new(),
                pagination: None,
            })
            .map_err(call_err)?;

            Ok(response.nodes.into_iter().map(node_info).collect())
        }

        fn execute_command(&mut self, node_id: u64, command: &str) -> HostResult<CommandOutput> {
            let response = host::nodecmd::execute_command(&nodecmd::ExecuteCommandRequest {
                node_id,
                command: command.to_owned(),
                work_dir: None,
            })
            .map_err(call_err)?;

            Ok(CommandOutput {
                output: response.output,
                exit_code: response.exit_code,
                error: response.error,
            })
        }

        fn upload(
            &mut self,
            node_id: u64,
            path: &str,
            content: &[u8],
            permissions: u32,
        ) -> HostResult<()> {
            let response = host::nodefs::upload(&nodefs::UploadRequest {
                node_id,
                path: path.to_owned(),
                content: content.to_vec(),
                permissions,
            })
            .map_err(call_err)?;

            if response.success {
                Ok(())
            } else {
                Err(HostApiError::Op(format!(
                    "upload failed: {}",
                    response.error.unwrap_or_default()
                )))
            }
        }

        fn create_daemon_task(
            &mut self,
            node_id: u64,
            command: &str,
            run_after_id: Option<u64>,
        ) -> HostResult<u64> {
            let response =
                host::daemontasks::create_daemon_task(&daemontasks::CreateDaemonTaskRequest {
                    node_id,
                    server_id: None,
                    task_type: DaemonTaskType::CmdExec as i32,
                    data: None,
                    cmd: Some(command.to_owned()),
                    run_after_id,
                })
                .map_err(call_err)?;

            if response.success {
                Ok(response.task_id)
            } else {
                Err(HostApiError::Op(response.error.unwrap_or_default()))
            }
        }

        fn find_daemon_task(&mut self, task_id: u64) -> HostResult<Option<DaemonTaskInfo>> {
            let response =
                host::daemontasks::find_daemon_tasks(&daemontasks::FindDaemonTasksRequest {
                    filter: Some(daemontasks::DaemonTaskFilter {
                        ids: vec![task_id],
                        ..Default::default()
                    }),
                    ..Default::default()
                })
                .map_err(call_err)?;

            Ok(response
                .tasks
                .into_iter()
                .next()
                .map(|task| DaemonTaskInfo {
                    id: task.id,
                    node_id: task.node_id,
                    status: TaskStatus::from_i32(task.status),
                    output: task.output.unwrap_or_default(),
                }))
        }

        fn storage_get(&mut self, key: &str, entity: StorageEntity) -> HostResult<Option<Vec<u8>>> {
            let response = host::storage::get(&storage::StorageGetRequest {
                key: key.to_owned(),
                entity_type: Some(entity.entity_type),
                entity_id: Some(entity.entity_id),
            })
            .map_err(call_err)?;

            if !response.found {
                return Ok(None);
            }

            Ok(response.payload)
        }

        fn storage_set(
            &mut self,
            key: &str,
            entity: StorageEntity,
            payload: &[u8],
        ) -> HostResult<()> {
            let response = host::storage::set(&storage::StorageSetRequest {
                key: key.to_owned(),
                entity_type: Some(entity.entity_type),
                entity_id: Some(entity.entity_id),
                payload: payload.to_vec(),
            })
            .map_err(call_err)?;

            if response.success {
                Ok(())
            } else {
                Err(HostApiError::Op(response.error.unwrap_or_default()))
            }
        }

        fn storage_delete(&mut self, key: &str, entity: StorageEntity) -> HostResult<()> {
            let response = host::storage::delete(&storage::StorageDeleteRequest {
                key: key.to_owned(),
                entity_type: Some(entity.entity_type),
                entity_id: Some(entity.entity_id),
            })
            .map_err(call_err)?;

            if response.success {
                Ok(())
            } else {
                Err(HostApiError::Op(
                    response
                        .error
                        .unwrap_or_else(|| "Storage deletion failed".into()),
                ))
            }
        }

        fn storage_list(
            &mut self,
            key_prefix: &str,
            entity: StorageEntity,
        ) -> HostResult<Vec<(String, Vec<u8>)>> {
            let response = host::storage::list(&storage::StorageListRequest {
                key_prefix: Some(key_prefix.to_owned()),
                entity_type: Some(entity.entity_type),
                entity_id: Some(entity.entity_id),
                limit: None,
                offset: None,
            })
            .map_err(call_err)?;

            Ok(response
                .entries
                .into_iter()
                .map(|entry| (entry.key, entry.payload))
                .collect())
        }

        fn authz_can(&mut self, user_id: u64, abilities: &[&str]) -> HostResult<bool> {
            let response = host::authz::can(&authz::CanRequest {
                user_id,
                abilities: abilities.iter().map(|ability| (*ability).into()).collect(),
            })
            .map_err(call_err)?;

            match response.error {
                Some(err) => Err(HostApiError::Op(err)),
                None => Ok(response.allowed),
            }
        }

        fn authz_can_any_for_entity(
            &mut self,
            user_id: u64,
            entity: StorageEntity,
            abilities: &[&str],
        ) -> HostResult<bool> {
            let response = host::authz::can_any_for_entity(&authz::CanForEntityRequest {
                user_id,
                entity_type: entity.entity_type,
                entity_id: entity.entity_id,
                abilities: abilities.iter().map(|ability| (*ability).into()).collect(),
            })
            .map_err(call_err)?;

            match response.error {
                Some(err) => Err(HostApiError::Op(err)),
                None => Ok(response.allowed),
            }
        }

        fn random_string(&mut self, length: i32, charset: Option<&str>) -> HostResult<String> {
            let response = host::crypto::random_string(&crypto::RandomStringRequest {
                length,
                charset: charset.map(str::to_owned),
            })
            .map_err(call_err)?;

            match response.error {
                Some(err) if !err.is_empty() => Err(HostApiError::Op(err)),
                _ => Ok(response.value),
            }
        }

        fn now_unix(&mut self) -> i64 {
            use std::time::{SystemTime, UNIX_EPOCH};

            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_secs() as i64)
                .unwrap_or(0)
        }

        fn log_error(&mut self, message: &str) {
            host::log::error(message);
        }
    }
}

/// In-memory host used by native handler tests.
#[cfg(test)]
pub mod mock {
    use std::collections::{BTreeMap, VecDeque};

    use super::{
        CommandOutput, DaemonTaskInfo, HostApi, HostApiError, HostResult, NodeInfo, ServerInfo,
        StorageEntity, TaskStatus,
    };

    type StorageKey = (String, (i32, u64));

    pub struct MockHost {
        pub servers: BTreeMap<u64, ServerInfo>,
        pub nodes: BTreeMap<u64, NodeInfo>,
        pub storage: BTreeMap<StorageKey, Vec<u8>>,
        /// (node_id, path) → uploaded content.
        pub files: BTreeMap<(u64, String), Vec<u8>>,
        /// (node_id, path, permissions) uploads, in call order.
        pub uploads: Vec<(u64, String, u32)>,
        /// Commands passed to execute_command, in call order.
        pub commands: Vec<String>,
        /// Canned execute_command results, popped one per call.
        pub command_results: VecDeque<CommandOutput>,
        /// Commands whose string contains a needle return the paired output
        /// (checked before `command_results`).
        pub fail_on: Vec<(String, CommandOutput)>,
        /// (id, node_id, command, run_after_id) daemon tasks, in creation order.
        pub created_tasks: Vec<(u64, u64, String, Option<u64>)>,
        pub task_states: BTreeMap<u64, DaemonTaskInfo>,
        next_task_id: u64,
        /// (length, charset) per random_string call.
        pub random_calls: Vec<(i32, Option<String>)>,
        pub grants: Vec<(u64, u64, String)>,
        pub authz_down: bool,
        pub now: i64,
        pub logs: Vec<String>,
    }

    impl Default for MockHost {
        fn default() -> Self {
            Self {
                servers: BTreeMap::new(),
                nodes: BTreeMap::new(),
                storage: BTreeMap::new(),
                files: BTreeMap::new(),
                uploads: Vec::new(),
                commands: Vec::new(),
                command_results: VecDeque::new(),
                fail_on: Vec::new(),
                created_tasks: Vec::new(),
                task_states: BTreeMap::new(),
                next_task_id: 101,
                random_calls: Vec::new(),
                grants: Vec::new(),
                authz_down: false,
                now: 1_700_000_000,
                logs: Vec::new(),
            }
        }
    }

    impl MockHost {
        /// Node 1 ("node-1", linux, /srv/gameap) with server 3 ("cs", cstrike).
        /// The server dir is relative to the work path, as the panel stores it.
        pub fn standard() -> Self {
            let mut host = Self::default();
            host.nodes.insert(
                1,
                NodeInfo {
                    id: 1,
                    name: "node-1".into(),
                    ips: vec!["203.0.113.1".into()],
                    work_path: "/srv/gameap".into(),
                    os: "linux".into(),
                },
            );
            host.servers.insert(
                3,
                ServerInfo {
                    id: 3,
                    node_id: 1,
                    name: "cs".into(),
                    game_id: "cstrike".into(),
                    dir: "servers/cs".into(),
                    enabled: true,
                },
            );
            host
        }

        pub fn file(&self, node_id: u64, path: &str) -> Option<&[u8]> {
            self.files
                .get(&(node_id, path.to_string()))
                .map(Vec::as_slice)
        }

        fn storage_key(key: &str, entity: StorageEntity) -> StorageKey {
            (key.to_string(), (entity.entity_type, entity.entity_id))
        }
    }

    impl HostApi for MockHost {
        fn get_server(&mut self, id: u64) -> HostResult<Option<ServerInfo>> {
            Ok(self.servers.get(&id).cloned())
        }

        fn find_servers(&mut self, ids: &[u64], node_ids: &[u64]) -> HostResult<Vec<ServerInfo>> {
            Ok(self
                .servers
                .values()
                .filter(|server| ids.is_empty() || ids.contains(&server.id))
                .filter(|server| node_ids.is_empty() || node_ids.contains(&server.node_id))
                .cloned()
                .collect())
        }

        fn get_node(&mut self, id: u64) -> HostResult<Option<NodeInfo>> {
            Ok(self.nodes.get(&id).cloned())
        }

        fn find_nodes(&mut self) -> HostResult<Vec<NodeInfo>> {
            Ok(self.nodes.values().cloned().collect())
        }

        fn execute_command(&mut self, _node_id: u64, command: &str) -> HostResult<CommandOutput> {
            self.commands.push(command.to_string());

            for (needle, output) in &self.fail_on {
                if command.contains(needle.as_str()) {
                    return Ok(output.clone());
                }
            }

            Ok(self.command_results.pop_front().unwrap_or(CommandOutput {
                output: String::new(),
                exit_code: 0,
                error: None,
            }))
        }

        fn upload(
            &mut self,
            node_id: u64,
            path: &str,
            content: &[u8],
            permissions: u32,
        ) -> HostResult<()> {
            self.files
                .insert((node_id, path.to_string()), content.to_vec());
            self.uploads.push((node_id, path.to_string(), permissions));
            Ok(())
        }

        fn create_daemon_task(
            &mut self,
            node_id: u64,
            command: &str,
            run_after_id: Option<u64>,
        ) -> HostResult<u64> {
            let id = self.next_task_id;
            self.next_task_id += 1;
            self.created_tasks
                .push((id, node_id, command.to_string(), run_after_id));
            self.task_states.insert(
                id,
                DaemonTaskInfo {
                    id,
                    node_id,
                    status: TaskStatus::Waiting,
                    output: String::new(),
                },
            );
            Ok(id)
        }

        fn find_daemon_task(&mut self, task_id: u64) -> HostResult<Option<DaemonTaskInfo>> {
            Ok(self.task_states.get(&task_id).cloned())
        }

        fn storage_get(&mut self, key: &str, entity: StorageEntity) -> HostResult<Option<Vec<u8>>> {
            Ok(self.storage.get(&Self::storage_key(key, entity)).cloned())
        }

        fn storage_set(
            &mut self,
            key: &str,
            entity: StorageEntity,
            payload: &[u8],
        ) -> HostResult<()> {
            self.storage
                .insert(Self::storage_key(key, entity), payload.to_vec());
            Ok(())
        }

        fn storage_delete(&mut self, key: &str, entity: StorageEntity) -> HostResult<()> {
            self.storage.remove(&Self::storage_key(key, entity));
            Ok(())
        }

        fn storage_list(
            &mut self,
            key_prefix: &str,
            entity: StorageEntity,
        ) -> HostResult<Vec<(String, Vec<u8>)>> {
            let scope = (entity.entity_type, entity.entity_id);
            Ok(self
                .storage
                .iter()
                .filter(|((key, entity), _)| *entity == scope && key.starts_with(key_prefix))
                .map(|((key, _), payload)| (key.clone(), payload.clone()))
                .collect())
        }

        fn authz_can(&mut self, user_id: u64, _abilities: &[&str]) -> HostResult<bool> {
            if self.authz_down {
                return Err(HostApiError::Op("authorization unavailable".into()));
            }

            Ok(user_id == 1)
        }

        fn authz_can_any_for_entity(
            &mut self,
            user_id: u64,
            entity: StorageEntity,
            abilities: &[&str],
        ) -> HostResult<bool> {
            if self.authz_down {
                return Err(HostApiError::Op("authorization unavailable".into()));
            }

            Ok(self
                .grants
                .iter()
                .any(|(granted_user, granted_entity, ability)| {
                    *granted_user == user_id
                        && *granted_entity == entity.entity_id
                        && abilities.contains(&ability.as_str())
                }))
        }

        fn random_string(&mut self, length: i32, charset: Option<&str>) -> HostResult<String> {
            self.random_calls
                .push((length, charset.map(str::to_string)));
            Ok(format!("{:032x}", self.random_calls.len()))
        }

        fn now_unix(&mut self) -> i64 {
            self.now
        }

        fn log_error(&mut self, message: &str) {
            self.logs.push(format!("ERROR {message}"));
        }
    }
}
