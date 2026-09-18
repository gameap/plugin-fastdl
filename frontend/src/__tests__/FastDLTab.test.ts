import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { h, reactive, type App } from 'vue';
import { content, deferred, descendants, find, invoke, mountComponent, settle } from './memory-renderer';
import { fastdlApi, type GameConfigurationResult, type ServerFastDL } from '../api';
import FastDLTab from '../tabs/FastDLTab.vue';

vi.mock('@gameap/plugin-sdk', () => ({ providePluginTrans: () => ({ trans: (key: string) => key }) }));
vi.mock('naive-ui', async () => {
  const { defineComponent, h } = await import('vue');
  return Object.fromEntries(['NAlert', 'NCard', 'NCheckbox', 'NForm', 'NFormItem', 'NInput', 'NSpin'].map((name) => [name,
    defineComponent({ inheritAttrs: false, setup: (_, { attrs, slots }) => () => h(name, attrs, slots.default?.()) }),
  ]));
});


const server: ServerFastDL = {
  server_name: 'Half-Life', enabled: true, autoindex: false, engine: 'goldsource', game_dir: 'valve',
  generate_bz2: true, can_manage: true, supported: true, node_ready: true,
  synced: true, warnings: [], download_url: 'https://downloads.example.test/files/', configuration: ['sv_allowdownload 1'],
};
const nodeError = { response: { data: { code: 'NODE_UNAVAILABLE', message: 'publication failed' } } };
const apps: App[] = [];
async function mount() {
  const props = reactive({ serverId: 4, pluginId: 'i3z7ix336msd4' });
  const { root, app } = await mountComponent(() => h(FastDLTab, props));
  apps.push(app);
  return { root, props, app };
}

beforeEach(() => {
  vi.stubGlobal('window', { $message: { success: vi.fn() } });
  vi.spyOn(fastdlApi, 'server').mockResolvedValue({ ...server });
  vi.spyOn(fastdlApi, 'saveServer').mockRejectedValue(nodeError);
  vi.spyOn(fastdlApi, 'applyConfiguration').mockResolvedValue({ configured: true, rcon_applied: true });
});
afterEach(() => {
  for (const app of apps.splice(0)) app.unmount();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe('FastDL save recovery', () => {
  it('shows unapplied settings and allows an unchanged retry after a partial save', async () => {
    const { root } = await mount();
    invoke(find(root, 'NCheckbox', 'autoindex'), 'onUpdate:checked', true);
    await settle();
    vi.mocked(fastdlApi.server).mockResolvedValue({ ...server, autoindex: true, synced: false });
    await invoke(find(root, 'NForm'), 'onSubmit', { preventDefault() {} });
    await settle();

    expect(fastdlApi.server).toHaveBeenCalledTimes(2);
    expect(find(root, 'NAlert', 'node_unavailable')).toBeDefined();
    expect(find(root, 'NAlert', 'sync_required')).toBeDefined();
    expect(find(root, 'NCheckbox', 'autoindex').props.checked).toBe(true);
    expect(find(root, 'GButton', 'save').props.disabled).toBe(false);
    expect(descendants(root).filter((node) => node.tag === 'NCard' && ['download_url', 'game_configuration'].includes(String(node.props.title)))).toHaveLength(0);
    expect(content(root)).not.toContain('unsaved');

    vi.mocked(fastdlApi.saveServer).mockResolvedValue({ ...server, autoindex: true });
    await invoke(find(root, 'NForm'), 'onSubmit', { preventDefault() {} });
    await settle();
    expect(find(root, 'GButton', 'save').props.disabled).toBe(true);
    expect(descendants(root).filter((node) => node.tag === 'NCard' && ['download_url', 'game_configuration'].includes(String(node.props.title)))).toHaveLength(2);
  });

  it('keeps the draft when refreshing persisted settings after a failed save', async () => {
    const { root } = await mount();
    const input = descendants(root).find((node) => node.tag === 'NInput' && node.props.placeholder === 'cstrike')!;
    invoke(input, 'onUpdate:value', ' valve ');
    await settle();
    vi.mocked(fastdlApi.server).mockResolvedValue({ ...server, synced: false });
    await invoke(find(root, 'NForm'), 'onSubmit', { preventDefault() {} });
    await settle();
    expect(fastdlApi.saveServer).toHaveBeenCalledWith(4, expect.objectContaining({ game_dir: 'valve' }));
    expect(input.props.value).toBe(' valve ');
  });

  it('trusts a refreshed confirmation when the save succeeded but its response was lost', async () => {
    const { root } = await mount();
    invoke(find(root, 'NCheckbox', 'autoindex'), 'onUpdate:checked', true);
    await settle();
    vi.mocked(fastdlApi.server).mockResolvedValue({ ...server, autoindex: true });
    await invoke(find(root, 'NForm'), 'onSubmit', { preventDefault() {} });
    await settle();
    expect(find(root, 'GButton', 'save').props.disabled).toBe(true);
    expect(content(root)).not.toContain('node_unavailable');
    expect(window.$message?.success).toHaveBeenCalledWith('synced');
    expect(descendants(root).filter((node) => node.tag === 'NCard' && ['download_url', 'game_configuration'].includes(String(node.props.title)))).toHaveLength(2);
  });

  it('keeps retry available and hides stale success when recovery cannot load state', async () => {
    const { root } = await mount();
    invoke(find(root, 'NCheckbox', 'autoindex'), 'onUpdate:checked', true);
    await settle();
    vi.mocked(fastdlApi.server).mockRejectedValue(new Error('connection lost'));
    await invoke(find(root, 'NForm'), 'onSubmit', { preventDefault() {} });
    await settle();
    expect(find(root, 'NAlert', 'node_unavailable')).toBeDefined();
    expect(find(root, 'GButton', 'save').props.disabled).toBe(false);
  });

  it('does not replace a different server with a late recovery response', async () => {
    const { root, props } = await mount();
    invoke(find(root, 'NCheckbox', 'autoindex'), 'onUpdate:checked', true);
    await settle();
    const recovery = deferred<ServerFastDL>();
    vi.mocked(fastdlApi.server).mockReturnValueOnce(recovery.promise).mockResolvedValue({ ...server, game_dir: 'cstrike' });
    const saving = invoke(find(root, 'NForm'), 'onSubmit', { preventDefault() {} });
    await settle();
    props.serverId = 8;
    await settle();
    recovery.resolve({ ...server, synced: false });
    await saving;
    await settle();
    expect(descendants(root).find((node) => node.tag === 'NInput' && node.props.placeholder === 'cstrike')?.props.value).toBe('cstrike');
    expect(content(root)).not.toContain('node_unavailable');
  });

  it('does not update or notify a closed tab after recovery finishes', async () => {
    const { root, app } = await mount();
    invoke(find(root, 'NCheckbox', 'autoindex'), 'onUpdate:checked', true);
    await settle();
    const recovery = deferred<ServerFastDL>();
    vi.mocked(fastdlApi.server).mockReturnValueOnce(recovery.promise);
    const saving = invoke(find(root, 'NForm'), 'onSubmit', { preventDefault() {} });
    await settle();
    app.unmount();
    apps.splice(apps.indexOf(app), 1);
    recovery.resolve({ ...server, synced: false });
    await saving;
    await settle();
    expect(root.children).toHaveLength(0);
    expect(window.$message?.success).not.toHaveBeenCalled();
  });
});

describe('game configuration application', () => {
  it.each([true, false])('applies only on request regardless of the legacy automatic setting: %s', async (manageGameConfig) => {
    const legacyServer = { ...server, manage_game_config: manageGameConfig };
    vi.mocked(fastdlApi.server).mockResolvedValue(legacyServer);
    const { root } = await mount();
    expect(content(root)).not.toContain('manage_game_config');
    expect(fastdlApi.applyConfiguration).not.toHaveBeenCalled();
    const card = descendants(root).find((node) => node.tag === 'NCard' && node.props.title === 'game_configuration')!;
    const button = find(card, 'GButton', 'apply_configuration');
    expect(button.props.size).toBe('small');
    expect(button.props.disabled).toBe(false);
    await invoke(button, 'onClick');
    await settle();

    expect(fastdlApi.applyConfiguration).toHaveBeenCalledExactlyOnceWith(4);
    expect(fastdlApi.saveServer).not.toHaveBeenCalled();
    expect(find(card, 'NAlert', 'configuration_applied').props.type).toBe('success');
  });

  it('warns when files were updated but RCON could not apply the settings', async () => {
    vi.mocked(fastdlApi.applyConfiguration).mockResolvedValue({ configured: true, rcon_applied: false });
    const { root } = await mount();
    await invoke(find(root, 'GButton', 'apply_configuration'), 'onClick');
    await settle();
    expect(find(root, 'NAlert', 'configuration_rcon_failed').props.type).toBe('warning');
    expect(content(root)).not.toContain('configuration_applied');
  });

  it.each([
    ['GAME_CONFIG_UPDATE_FAILED', 'game_apply_game_config_update_failed'],
    ['CONFIGURE_FAILED', 'game_apply_configure_failed'],
    ['CONFIGURATION_NOT_READY', 'configuration_not_ready'],
    ['UNRECOGNIZED', 'apply_configuration_failed'],
  ])('explains %s without claiming a save failed and permits retry', async (code, key) => {
    vi.mocked(fastdlApi.applyConfiguration).mockRejectedValueOnce({ response: { data: { code } } });
    const { root } = await mount();
    await invoke(find(root, 'GButton', 'apply_configuration'), 'onClick');
    await settle();
    expect(find(root, 'NAlert', key).props.type).toBe('error');
    expect(content(root)).not.toContain('save_failed');
    expect(find(root, 'GButton', 'apply_configuration').props.disabled).toBe(false);

    await invoke(find(root, 'GButton', 'apply_configuration'), 'onClick');
    await settle();
    expect(content(root)).not.toContain(key);
    expect(find(root, 'NAlert', 'configuration_applied')).toBeDefined();
  });

  it('clears feedback and blocks application while there are unsaved changes', async () => {
    const { root } = await mount();
    const button = find(root, 'GButton', 'apply_configuration');
    await invoke(button, 'onClick');
    await settle();
    invoke(find(root, 'NCheckbox', 'autoindex'), 'onUpdate:checked', true);
    await settle();
    expect(content(root)).not.toContain('configuration_applied');
    expect(button.props.disabled).toBe(true);
    await invoke(button, 'onClick');
    expect(fastdlApi.applyConfiguration).toHaveBeenCalledTimes(1);
  });

  it.each([
    { supported: false }, { node_ready: false }, { download_url: '' },
  ])('blocks application when the server is unavailable: %j', async (settings) => {
    vi.mocked(fastdlApi.server).mockResolvedValue({ ...server, ...settings });
    const { root } = await mount();
    const button = find(root, 'GButton', 'apply_configuration');
    expect(button.props.disabled).toBe(true);
    await invoke(button, 'onClick');
    expect(fastdlApi.applyConfiguration).not.toHaveBeenCalled();
  });

  it.each([
    { synced: false }, { can_manage: false }, { enabled: false }, { configuration: [] },
  ])('does not expose the action when it cannot be used: %j', async (settings) => {
    vi.mocked(fastdlApi.server).mockResolvedValue({ ...server, ...settings });
    const { root } = await mount();
    expect(descendants(root).some((node) => node.tag === 'GButton' && content(node).includes('apply_configuration'))).toBe(false);
  });

  it('locks edits, save, refresh and repeated application until completion', async () => {
    const pending = deferred<GameConfigurationResult>();
    vi.mocked(fastdlApi.applyConfiguration).mockReturnValueOnce(pending.promise);
    const { root } = await mount();
    const button = find(root, 'GButton', 'apply_configuration');
    const applying = invoke(button, 'onClick');
    await settle();
    expect(button.props.loading).toBe(true);
    expect(button.props.disabled).toBe(true);
    expect(find(root, 'GButton', 'save').props.disabled).toBe(true);
    expect(find(root, 'GButton', 'refresh').props.disabled).toBe(true);
    expect(descendants(root).filter((node) => ['NCheckbox', 'NInput'].includes(node.tag)).every((node) => node.props.disabled === true)).toBe(true);
    await invoke(button, 'onClick');
    await invoke(find(root, 'GButton', 'refresh'), 'onClick');
    await invoke(find(root, 'NForm'), 'onSubmit', { preventDefault() {} });
    expect(fastdlApi.applyConfiguration).toHaveBeenCalledTimes(1);
    expect(fastdlApi.server).toHaveBeenCalledTimes(1);
    expect(fastdlApi.saveServer).not.toHaveBeenCalled();

    pending.resolve({ configured: true, rcon_applied: true });
    await applying;
    await settle();
    expect(button.props.loading).toBe(false);
    expect(button.props.disabled).toBe(false);
  });

  it('ignores an application result from a previously selected server', async () => {
    const pending = deferred<GameConfigurationResult>();
    vi.mocked(fastdlApi.applyConfiguration).mockReturnValueOnce(pending.promise);
    const { root, props } = await mount();
    const applying = invoke(find(root, 'GButton', 'apply_configuration'), 'onClick');
    await settle();
    props.serverId = 8;
    await settle();
    pending.resolve({ configured: true, rcon_applied: true });
    await applying;
    await settle();
    expect(content(root)).not.toContain('configuration_applied');
    expect(find(root, 'GButton', 'apply_configuration').props.loading).toBe(false);
  });

  it('ignores an application result after the tab is closed', async () => {
    const pending = deferred<GameConfigurationResult>();
    vi.mocked(fastdlApi.applyConfiguration).mockReturnValueOnce(pending.promise);
    const { root, app } = await mount();
    const applying = invoke(find(root, 'GButton', 'apply_configuration'), 'onClick');
    await settle();
    app.unmount();
    apps.splice(apps.indexOf(app), 1);
    pending.resolve({ configured: true, rcon_applied: true });
    await applying;
    await settle();
    expect(root.children).toHaveLength(0);
    expect(window.$message?.success).not.toHaveBeenCalled();
  });
});

describe('game-derived engine', () => {
  it.each(['goldsource', 'source'] as const)('uses the %s game engine for compression settings', async (engine) => {
    vi.mocked(fastdlApi.server).mockResolvedValue({ ...server, engine });
    vi.mocked(fastdlApi.saveServer).mockResolvedValue({ ...server, engine, autoindex: true });
    const { root } = await mount();
    expect(descendants(root).some((node) => node.tag === 'NCheckbox' && content(node) === 'generate_bz2')).toBe(engine === 'source');
    invoke(find(root, 'NCheckbox', 'autoindex'), 'onUpdate:checked', true);
    await settle();
    await invoke(find(root, 'NForm'), 'onSubmit', { preventDefault() {} });
    expect(fastdlApi.saveServer).toHaveBeenCalledWith(4, {
      enabled: true, autoindex: true, game_dir: 'valve', generate_bz2: true,
    });
    expect(fastdlApi.applyConfiguration).not.toHaveBeenCalled();
  });

  it('prevents saving when the game engine is unsupported', async () => {
    vi.mocked(fastdlApi.server).mockResolvedValue({ ...server, supported: false, synced: false });
    const { root } = await mount();
    expect(find(root, 'NAlert', 'unsupported')).toBeDefined();
    expect(find(root, 'GButton', 'save').props.disabled).toBe(true);
    await invoke(find(root, 'NForm'), 'onSubmit', { preventDefault() {} });
    expect(fastdlApi.saveServer).not.toHaveBeenCalled();
  });
});
