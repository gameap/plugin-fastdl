import axios from 'axios';

export type Engine = 'goldsource' | 'source';
export type InstallState = 'not_installed' | 'installing' | 'installed' | 'failed';

export interface NodeConfig {
  listen: string;
  public_base_url: string;
}

export interface NodeStatus {
  status: InstallState;
  version?: string;
  error_message?: string;
  task_id?: number;
}

export interface FastDLNode extends NodeStatus {
  id: number;
  name: string;
  os: string;
  enabled_servers: number;
  config: NodeConfig;
}

export interface ServerSettings {
  enabled: boolean;
  autoindex: boolean;
  game_dir: string;
  manage_game_config: boolean;
  generate_bz2: boolean;
}

export interface ServerFastDL extends ServerSettings {
  engine: Engine;
  server_name: string;
  download_url: string;
  configuration: string[];
  can_manage: boolean;
  supported: boolean;
  node_ready: boolean;
  synced: boolean;
  warnings: string[];
}

export interface GameConfigurationResult {
  configured: boolean;
  rcon_applied: boolean;
}

interface SavedGameConfiguration {
  configured: boolean;
  configuration: string[];
}

const BASE = '/api/plugins/i3z7ix336msd4';

export const fastdlApi = {
  nodes: async () => (await axios.get<{ nodes: FastDLNode[] }>(`${BASE}/admin/nodes`)).data.nodes,
  status: async (id: number) => (await axios.get<NodeStatus>(`${BASE}/nodes/${id}/status`)).data,
  saveNode: async (id: number, config: NodeConfig) => (await axios.put<NodeConfig>(`${BASE}/nodes/${id}/config`, config)).data,
  setup: async (id: number) => (await axios.post<NodeStatus>(`${BASE}/nodes/${id}/setup`, {})).data,
  server: async (id: number) => (await axios.get<ServerFastDL>(`${BASE}/servers/${id}/fastdl`)).data,
  saveServer: async (id: number, settings: ServerSettings) => (await axios.put<ServerFastDL>(`${BASE}/servers/${id}/fastdl`, settings)).data,
  applyConfiguration: async (id: number): Promise<GameConfigurationResult> => {
    const { data } = await axios.post<SavedGameConfiguration>(`${BASE}/servers/${id}/fastdl/configure`, {});
    if (!data.configured) return { configured: false, rcon_applied: false };
    try {
      for (const command of data.configuration) await axios.post(`/api/servers/${id}/rcon`, { command });
      return { configured: true, rcon_applied: true };
    } catch {
      return { configured: true, rcon_applied: false };
    }
  },
};

const errorKeys = new Map([
  ['CONFIGURE_FAILED', 'configure_failed'],
  ['GAME_CONFIG_UPDATE_FAILED', 'game_config_update_failed'],
  ['CONFIGURATION_NOT_READY', 'configuration_not_ready'],
  ['NODE_UNAVAILABLE', 'node_unavailable'],
  ['RESTART_FAILED', 'restart_failed'],
  ['FORBIDDEN', 'access_denied'],
  ['UNAUTHENTICATED', 'authentication_required'],
  ['AUTHZ_UNAVAILABLE', 'authorization_unavailable'],
  ['NOT_FOUND', 'not_found'],
]);

const validationKeys = new Map([
  ['FastDL only supports GoldSource and Source games', 'unsupported'],
  ['Listen address must be an IP address and port', 'listen_invalid'],
  ['Listen port must not be zero', 'listen_invalid'],
  ['A valid HTTP URL is required (HTTPS for downloads)', 'public_url_invalid'],
  ['Invalid public or download URL', 'public_url_invalid'],
  ['Game directory must be a relative path inside the game server', 'game_dir_invalid'],
  ['Internal server directories cannot be used as a game directory', 'game_dir_invalid'],
  ['Only Linux and Windows nodes are supported', 'unsupported_node_os'],
]);

const conflictKeys = new Map([
  ['Install FastDL on this node first', 'node_not_ready'],
  ['Configure the public FastDL address first', 'public_url_required'],
  ['Installation is already in progress', 'installation_in_progress'],
  ["Disable FastDL on this node's game servers before clearing its public address", 'disable_before_clearing_url'],
  ['Cannot safely update server.cfg. Ensure the configuration exists, is a regular file inside this server, and the FastDL service can access it.', 'game_config_update_failed'],
]);

type ErrorContext = 'node-save' | 'game-configure';

export function errorMessage(error: unknown, fallback: string, trans: (key: string) => string, context?: ErrorContext): string {
  const body = (error as { response?: { data?: { code?: unknown; message?: unknown; server_name?: unknown } } } | null)?.response?.data;
  if (typeof body?.code !== 'string') return fallback;

  let key = errorKeys.get(body.code);
  if (typeof body.message === 'string') {
    if (body.code === 'INVALID_INPUT') key = validationKeys.get(body.message);
    if (body.code === 'CONFLICT') key = conflictKeys.get(body.message);
  }
  if (context === 'game-configure' && (key === 'configure_failed' || key === 'game_config_update_failed')) {
    return trans(`game_apply_${key}`);
  }
  if (context === 'node-save' && (key === 'configure_failed' || key === 'game_config_update_failed')) {
    const message = trans(`node_save_${key}`);
    const name = typeof body.server_name === 'string' ? body.server_name.trim() : '';
    return name && name.length <= 200 && !/[\x00-\x1f\x7f<>{}]/.test(name)
      ? `${trans('affected_game_server').replace('{name}', () => name)} ${message}`
      : message;
  }
  return key ? trans(key) : fallback;
}
