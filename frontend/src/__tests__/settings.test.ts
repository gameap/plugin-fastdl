import { describe, expect, it } from 'vitest';
import { editableSettings, isDownloadUrl, isGameDirectory, isListenAddress, isPublicUrl, needsApply, normalizeNodeConfig, serverWarnings, settingsChanged } from '../lib/settings';
import type { ServerFastDL } from '../api';
import { translations } from '../translations';

describe('game content directory', () => {
  it.each(['', 'cstrike', 'orangebox/tf', 'garrysmod', 'cs-1.6/cstrike', 'game files/cstrike', 'Игры/cstrike'])('accepts a relative game directory: %s', (path) => {
    expect(isGameDirectory(path)).toBe(true);
  });
  it.each(['.', '..', '../cstrike', 'cstrike/../cfg', '/etc', 'C:\\servers', '.ssh', 'cstrike/.config', 'cstrike//maps', 'cstrike/', 'cstrike\\maps', 'cstrike/%2e%2e', 'cstrike\n/maps', 'cstrike./maps', 'cstrike /maps'])('rejects an unsafe or ambiguous directory: %s', (path) => {
    expect(isGameDirectory(path)).toBe(false);
  });
  it.each(['cfg', 'cstrike/CONFIGS', 'addons', 'game/plugins', 'logs', 'backup', 'downloads'])('rejects private directory %s', (path) => {
    expect(isGameDirectory(path)).toBe(false);
  });
  it('uses the backend UTF-8 byte limit and rejects Unicode control characters', () => {
    expect(isGameDirectory('я'.repeat(256))).toBe(true);
    expect(isGameDirectory('я'.repeat(257))).toBe(false);
    expect(isGameDirectory('game\u0085/cstrike')).toBe(false);
  });
});

describe('public download address', () => {
  it.each(['http://cdn.example.com:8080', 'https://cdn.example.com/fastdl', 'http://[::1]:8080'])('accepts %s', (url) => {
    expect(isPublicUrl(url)).toBe(true);
  });
  it.each(['javascript:alert(1)', 'ftp://example.com', 'https://user:password@example.com', 'https://example.com/?key=secret', 'https://example.com/#file', 'https://example.com/"\nsv_password', 'https://example.com\\evil'])('rejects %s', (url) => {
    expect(isPublicUrl(url)).toBe(false);
  });
  it('normalizes surrounding whitespace and trailing slashes', () => {
    expect(normalizeNodeConfig({ listen: ' 0.0.0.0:8080 ', public_base_url: ' https://cdn.example.com/// ' })).toEqual({ listen: '0.0.0.0:8080', public_base_url: 'https://cdn.example.com' });
  });
  it.each(['http://example.com/?', 'http://example.com/#', 'http://example.com/a@b', 'http://example.com/../files', 'HTTP://example.com'])('rejects a URL rejected by the backend: %s', (url) => {
    expect(isPublicUrl(url)).toBe(false);
  });
});

describe('installation fields', () => {
  it.each(['0.0.0.0:8080', '[::]:8080', '[::1]:9000', '127.0.0.1:8080'])('accepts listen address %s', (address) => {
    expect(isListenAddress(address)).toBe(true);
  });
  it.each(['0.0.0.0', 'localhost:9000', ':8080', 'localhost:0', '127.0.0.1:65536', 'localhost:abc', '::1:8080', 'host:8080\ncommand', '999.1.2.3:80', '127.00.0.1:80'])('rejects invalid listen address %s', (address) => {
    expect(isListenAddress(address)).toBe(false);
  });
  it('requires HTTPS binaries without credentials or fragments', () => {
    expect(isDownloadUrl('https://example.com/gameap-fastdl')).toBe(true);
    expect(isDownloadUrl('https://example.com/gameap-fastdl?version=1')).toBe(false);
    expect(isDownloadUrl('http://example.com/gameap-fastdl')).toBe(false);
    expect(isDownloadUrl('https://user:pass@example.com/gameap-fastdl')).toBe(false);
    expect(isDownloadUrl('https://example.com/gameap-fastdl#checksum')).toBe(false);
  });
});

describe('server settings', () => {
  const server: ServerFastDL = {
    server_name: 'Counter-Strike', enabled: true, engine: 'goldsource', autoindex: false,
    game_dir: 'cstrike', manage_game_config: true, generate_bz2: true,
    download_url: 'https://example.com/opaque-route/', configuration: [],
    can_manage: true, supported: true, node_ready: true, synced: true, warnings: [],
  };
  it('sends only writable fields, excluding permissions and generated paths', () => {
    expect(Object.keys(editableSettings(server)).sort()).toEqual(['autoindex', 'enabled', 'engine', 'game_dir', 'generate_bz2', 'manage_game_config']);
  });
  it('detects configuration management and compression edits', () => {
    const form = editableSettings(server);
    expect(settingsChanged(form, server)).toBe(false);
    expect(settingsChanged({ ...form, manage_game_config: false }, server)).toBe(true);
    expect(settingsChanged({ ...form, generate_bz2: false }, server)).toBe(true);
  });
  it('allows applying an unchanged configuration again after a partial backend failure', () => {
    const form = editableSettings(server);
    expect(needsApply(form, server)).toBe(false);
    expect(needsApply(form, { ...server, synced: false })).toBe(true);
  });
  it('localizes and deduplicates backend warnings while retaining unknown warnings', () => {
    const trans = (key: string) => `translated:${key}`;
    expect(serverWarnings({ ...server, node_ready: false, synced: false, warnings: [
      'FastDL is not installed on this node',
      'A public FastDL address must be configured by an administrator',
      'Changes have not been applied successfully. Save again or ask an administrator to synchronize the node.',
      'New server warning',
    ] }, trans)).toEqual(['translated:node_not_ready', 'translated:public_url_required', 'translated:sync_required', 'New server warning']);
  });
});

describe('translations', () => {
  it('has the same non-empty translations and placeholders in both languages', () => {
    expect(Object.keys(translations.ru).sort()).toEqual(Object.keys(translations.en).sort());
    for (const key of Object.keys(translations.en) as (keyof typeof translations.en)[]) {
      expect(translations.ru[key].trim()).not.toBe('');
      expect(translations.ru[key].match(/\{\w+\}/g) ?? []).toEqual(translations.en[key].match(/\{\w+\}/g) ?? []);
    }
  });
});
