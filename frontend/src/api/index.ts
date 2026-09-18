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
  engine: Engine;
  game_dir: string;
  manage_game_config: boolean;
  generate_bz2: boolean;
}

export interface ServerFastDL extends ServerSettings {
  server_name: string;
  download_url: string;
  configuration: string[];
  can_manage: boolean;
  supported: boolean;
  node_ready: boolean;
  synced: boolean;
  warnings: string[];
}

const BASE = '/api/plugins/fastdla';

export const fastdlApi = {
  nodes: async () => (await axios.get<{ nodes: FastDLNode[] }>(`${BASE}/admin/nodes`)).data.nodes,
  status: async (id: number) => (await axios.get<NodeStatus>(`${BASE}/nodes/${id}/status`)).data,
  saveNode: async (id: number, config: NodeConfig) => (await axios.put<NodeConfig>(`${BASE}/nodes/${id}/config`, config)).data,
  setup: async (id: number) => (await axios.post<NodeStatus>(`${BASE}/nodes/${id}/setup`, {})).data,
  sync: async (id: number) => (await axios.post<{ synced: boolean }>(`${BASE}/nodes/${id}/sync`)).data,
  server: async (id: number) => (await axios.get<ServerFastDL>(`${BASE}/servers/${id}/fastdl`)).data,
  saveServer: async (id: number, settings: ServerSettings) => (await axios.put<ServerFastDL>(`${BASE}/servers/${id}/fastdl`, settings)).data,
};

export function errorMessage(error: unknown, fallback: string): string {
  const response = (error as { response?: { data?: { message?: unknown } } } | null)?.response;
  return typeof response?.data?.message === 'string' ? response.data.message : fallback;
}
