import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { h, reactive, type App } from 'vue';
import { content, deferred, descendants, find, invoke, mountComponent, settle } from './memory-renderer';
import { fastdlApi, type ServerFastDL } from '../api';
import FastDLTab from '../tabs/FastDLTab.vue';

vi.mock('@gameap/plugin-sdk', () => ({ providePluginTrans: () => ({ trans: (key: string) => key }) }));
vi.mock('naive-ui', async () => {
  const { defineComponent, h } = await import('vue');
  return Object.fromEntries(['NAlert', 'NCard', 'NCheckbox', 'NFormItem', 'NInput', 'NSelect', 'NSpin'].map((name) => [name,
    defineComponent({ inheritAttrs: false, setup: (_, { attrs, slots }) => () => h(name, attrs, slots.default?.()) }),
  ]));
});


const server: ServerFastDL = {
  server_name: 'Half-Life', enabled: true, autoindex: false, engine: 'goldsource', game_dir: 'valve',
  manage_game_config: true, generate_bz2: true, can_manage: true, supported: true, node_ready: true,
  synced: true, warnings: [], download_url: 'https://downloads.example.test/files/', configuration: ['sv_allowdownload 1'],
};
const configureError = { response: { data: { code: 'CONFIGURE_FAILED', message: 'unverified output' } } };
const apps: App[] = [];
async function mount() {
  const props = reactive({ serverId: 4, pluginId: 'fastdla' });
  const { root, app } = await mountComponent(() => h(FastDLTab, props));
  apps.push(app);
  return { root, props, app };
}

beforeEach(() => {
  vi.stubGlobal('window', { $message: { success: vi.fn() } });
  vi.spyOn(fastdlApi, 'server').mockResolvedValue({ ...server });
  vi.spyOn(fastdlApi, 'saveServer').mockRejectedValue(configureError);
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
    await invoke(find(root, 'GButton', 'save'), 'onClick');
    await settle();

    expect(fastdlApi.server).toHaveBeenCalledTimes(2);
    expect(find(root, 'NAlert', 'configure_failed')).toBeDefined();
    expect(find(root, 'GStatusBadge').props).toMatchObject({ color: 'orange', text: 'not_applied' });
    expect(find(root, 'NCheckbox', 'autoindex').props.checked).toBe(true);
    expect(find(root, 'GButton', 'save').props.disabled).toBe(false);
    expect(descendants(root).filter((node) => node.tag === 'NCard' && node.props.title)).toHaveLength(0);
    expect(content(root)).not.toContain('unsaved');

    vi.mocked(fastdlApi.saveServer).mockResolvedValue({ ...server, autoindex: true });
    await invoke(find(root, 'GButton', 'save'), 'onClick');
    await settle();
    expect(find(root, 'GStatusBadge').props.text).toBe('active');
    expect(find(root, 'GButton', 'save').props.disabled).toBe(true);
    expect(descendants(root).filter((node) => node.tag === 'NCard' && node.props.title)).toHaveLength(2);
  });

  it('keeps the draft when refreshing persisted settings after a failed save', async () => {
    const { root } = await mount();
    const input = descendants(root).find((node) => node.tag === 'NInput' && node.props.placeholder === 'cstrike')!;
    invoke(input, 'onUpdate:value', ' valve ');
    await settle();
    vi.mocked(fastdlApi.server).mockResolvedValue({ ...server, synced: false });
    await invoke(find(root, 'GButton', 'save'), 'onClick');
    await settle();
    expect(fastdlApi.saveServer).toHaveBeenCalledWith(4, expect.objectContaining({ game_dir: 'valve' }));
    expect(input.props.value).toBe(' valve ');
  });

  it('trusts a refreshed confirmation when the save succeeded but its response was lost', async () => {
    const { root } = await mount();
    invoke(find(root, 'NCheckbox', 'autoindex'), 'onUpdate:checked', true);
    await settle();
    vi.mocked(fastdlApi.server).mockResolvedValue({ ...server, autoindex: true });
    await invoke(find(root, 'GButton', 'save'), 'onClick');
    await settle();
    expect(find(root, 'GStatusBadge').props.text).toBe('active');
    expect(find(root, 'GButton', 'save').props.disabled).toBe(true);
    expect(content(root)).not.toContain('configure_failed');
    expect(window.$message?.success).toHaveBeenCalledWith('synced');
    expect(descendants(root).filter((node) => node.tag === 'NCard' && node.props.title)).toHaveLength(2);
  });

  it('keeps retry available and hides stale success when recovery cannot load state', async () => {
    const { root } = await mount();
    invoke(find(root, 'NCheckbox', 'autoindex'), 'onUpdate:checked', true);
    await settle();
    vi.mocked(fastdlApi.server).mockRejectedValue(new Error('connection lost'));
    await invoke(find(root, 'GButton', 'save'), 'onClick');
    await settle();
    expect(find(root, 'NAlert', 'configure_failed')).toBeDefined();
    expect(find(root, 'GButton', 'save').props.disabled).toBe(false);
    expect(find(root, 'GStatusBadge').props.text).toBe('not_applied');
  });

  it('does not replace a different server with a late recovery response', async () => {
    const { root, props } = await mount();
    invoke(find(root, 'NCheckbox', 'autoindex'), 'onUpdate:checked', true);
    await settle();
    const recovery = deferred<ServerFastDL>();
    vi.mocked(fastdlApi.server).mockReturnValueOnce(recovery.promise).mockResolvedValue({ ...server, game_dir: 'cstrike' });
    const saving = invoke(find(root, 'GButton', 'save'), 'onClick');
    await settle();
    props.serverId = 8;
    await settle();
    recovery.resolve({ ...server, synced: false });
    await saving;
    await settle();
    expect(descendants(root).find((node) => node.tag === 'NInput' && node.props.placeholder === 'cstrike')?.props.value).toBe('cstrike');
    expect(find(root, 'GStatusBadge').props.text).toBe('active');
    expect(content(root)).not.toContain('configure_failed');
  });

  it('does not update or notify a closed tab after recovery finishes', async () => {
    const { root, app } = await mount();
    invoke(find(root, 'NCheckbox', 'autoindex'), 'onUpdate:checked', true);
    await settle();
    const recovery = deferred<ServerFastDL>();
    vi.mocked(fastdlApi.server).mockReturnValueOnce(recovery.promise);
    const saving = invoke(find(root, 'GButton', 'save'), 'onClick');
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
