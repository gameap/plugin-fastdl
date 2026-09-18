import type { NodeConfig, ServerFastDL, ServerSettings } from '../api';

const privateDirectories = new Set(['cfg', 'config', 'configs', 'log', 'logs', 'addons', 'plugins', 'bin', 'cache', 'data', 'download', 'downloads', 'backup', 'backups']);

export function isPublicUrl(value: string): boolean {
  if (!value || value.length > 2048 || !/^https?:\/\//.test(value) || /[^\x21-\x7e]|["'\\%?#@]/.test(value)) return false;
  if (value.split('/').some((part) => part === '.' || part === '..')) return false;
  try {
    const url = new URL(value);
    return ['http:', 'https:'].includes(url.protocol)
      && !url.username && !url.password && !url.search && !url.hash;
  } catch {
    return false;
  }
}

export function isDownloadUrl(value: string): boolean {
  if (!isPublicUrl(value)) return false;
  try {
    const url = new URL(value);
    return url.protocol === 'https:' && !url.username && !url.password && !url.hash;
  } catch {
    return false;
  }
}

export function isGameDirectory(value: string): boolean {
  if (value === '') return true;
  if (new TextEncoder().encode(value).length > 512 || /[\x00-\x1f\x7f-\x9f\\:%"'<>|?*{}]/.test(value)) return false;
  return value.split('/').every((part) => part.length > 0 && !part.startsWith('.') && !part.endsWith('.') && !part.endsWith(' ') && !privateDirectories.has(part.toLowerCase()));
}

export function isListenAddress(value: string): boolean {
  const match = /^(\[[0-9a-fA-F:.]+\]|\d{1,3}(?:\.\d{1,3}){3}):(\d{1,5})$/.exec(value);
  if (!match || Number(match[2]) < 1 || Number(match[2]) > 65535) return false;
  try {
    const url = new URL(`http://${value}`);
    return url.hostname === match[1] || (match[1].startsWith('[') && url.hostname.startsWith('['));
  } catch {
    return false;
  }
}

export function normalizeNodeConfig(config: NodeConfig): NodeConfig {
  return { listen: config.listen.trim(), public_base_url: config.public_base_url.trim().replace(/\/+$/, '') };
}

export function editableSettings(value: ServerFastDL): ServerSettings {
  return {
    enabled: value.enabled,
    autoindex: value.autoindex,
    engine: value.engine,
    game_dir: value.game_dir,
    manage_game_config: value.manage_game_config,
    generate_bz2: value.generate_bz2,
  };
}

export function settingsChanged(current: ServerSettings, original: ServerFastDL): boolean {
  return Object.entries(editableSettings(original)).some(([key, value]) => current[key as keyof ServerSettings] !== value);
}

export function needsApply(current: ServerSettings, original: ServerFastDL): boolean {
  return original.synced === false || settingsChanged(current, original);
}

const warningKeys: Record<string, string> = {
  'FastDL is not installed on this node': 'node_not_ready',
  'A public FastDL address must be configured by an administrator': 'public_url_required',
  'Changes have not been applied successfully. Save again or ask an administrator to synchronize the node.': 'sync_required',
};

export function serverWarnings(server: ServerFastDL, trans: (key: string) => string): string[] {
  const warnings = server.warnings.map((message) => warningKeys[message] ? trans(warningKeys[message]) : message);
  if (!server.node_ready) warnings.unshift(trans('node_not_ready'));
  if (server.synced === false) warnings.push(trans('sync_required'));
  return [...new Set(warnings)];
}
