import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { h, type App } from 'vue';
import { fastdlApi, type FastDLNode, type NodeConfig, type NodeStatus } from '../api';
import AdminPage from '../pages/AdminPage.vue';
import { content, descendants, find, invoke, mountComponent, settle, type Element } from './memory-renderer';

vi.mock('@gameap/plugin-sdk', async () => {
  const { ref } = await import('vue');
  return {
    usePluginTrans: () => ({ trans: (key: string) => key }),
    useIsAdmin: () => ref(true),
  };
});
vi.mock('vue-router', async () => {
  const { defineComponent, h } = await import('vue');
  return {
    useRouter: () => ({ replace: vi.fn() }),
    RouterLink: defineComponent({ inheritAttrs: false, setup: (_, { attrs, slots }) => () => h('RouterLink', attrs, slots.default?.()) }),
  };
});
vi.mock('naive-ui', async () => {
  const { defineComponent, h } = await import('vue');
  return Object.fromEntries(['NAlert', 'NCard', 'NInput', 'NSpin'].map((name) => [name,
    defineComponent({ inheritAttrs: false, setup: (_, { attrs, slots }) => () => h(name, attrs, [slots.header?.(), slots.default?.(), slots.footer?.()]) }),
  ]));
});
vi.mock('../components/NodeSettings.vue', async () => {
  const { defineComponent, h } = await import('vue');
  return { default: defineComponent({ inheritAttrs: false, setup: (_, { attrs }) => () => h('NodeSettings', attrs) }) };
});
vi.mock('../components/NodeInstall.vue', async () => {
  const { defineComponent, h } = await import('vue');
  return { default: defineComponent({ inheritAttrs: false, setup: (_, { attrs }) => () => h('NodeInstall', attrs) }) };
});

const nodeFixtures = (): FastDLNode[] => [
  { id: 7, name: 'New node', os: 'linux', status: 'not_installed', enabled_servers: 0, config: { listen: '0.0.0.0:8080', public_base_url: '' } },
  { id: 21, name: 'Retry node', os: 'windows', status: 'failed', enabled_servers: 0, config: { listen: '127.0.0.1:9090', public_base_url: 'https://retry.example.test' } },
  { id: 35, name: 'Existing node', os: 'linux', status: 'installed', enabled_servers: 2, config: { listen: '0.0.0.0:8080', public_base_url: 'https://existing.example.test' } },
];
const buttons = (card: Element) => descendants(card).filter((node) => node.tag === 'GButton');
const status = (card: Element) => find(card, 'GStatusBadge').props.text;
let app: App | undefined;

async function mountPage() {
  const mounted = await mountComponent(() => h(AdminPage));
  app = mounted.app;
  return mounted.root;
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.stubGlobal('window', { $message: { success: vi.fn() } });
  vi.spyOn(fastdlApi, 'nodes').mockResolvedValue(nodeFixtures());
  vi.spyOn(fastdlApi, 'status').mockResolvedValue({ status: 'installed', version: '0.2.3' });
});
afterEach(() => {
  app?.unmount();
  app = undefined;
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe('admin installation flow', () => {
  it.each([
    ['New node', 7],
    ['Retry node', 21],
  ])('opens installation settings for %s with only an Install action', async (name, id) => {
    const root = await mountPage();
    const card = find(root, 'NCard', name);

    expect(buttons(card).map(content)).toEqual(['install']);
    await invoke(find(card, 'GButton', 'install'), 'onClick');
    await settle();

    const settings = find(root, 'NodeSettings');
    expect(settings.props.mode).toBe('install');
    expect(settings.props.node).toMatchObject({ id, name });
    expect(descendants(root).some((node) => node.tag === 'NodeInstall')).toBe(false);
  });

  it('applies the started status and configuration only to the selected node and polls it', async () => {
    const root = await mountPage();
    await invoke(find(find(root, 'NCard', 'Retry node'), 'GButton', 'install'), 'onClick');
    await settle();
    const config = { listen: '0.0.0.0:8181', public_base_url: 'https://new-downloads.example.test' };
    const started = find(root, 'NodeSettings').props.onStarted as (status: NodeStatus, config: NodeConfig) => void;
    started({ status: 'installing', task_id: 123 }, config);
    await settle();

    const selected = find(root, 'NCard', 'Retry node');
    expect(descendants(root).some((node) => node.tag === 'NodeSettings' || node.tag === 'NodeInstall')).toBe(false);
    expect(status(selected)).toBe('status_installing');
    expect(find(selected, 'GButton', 'install').props.disabled).toBe(true);
    expect(status(find(root, 'NCard', 'New node'))).toBe('status_not_installed');
    expect(status(find(root, 'NCard', 'Existing node'))).toBe('status_installed');
    expect(fastdlApi.nodes).toHaveBeenCalledOnce();

    await vi.advanceTimersByTimeAsync(3999);
    expect(fastdlApi.status).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    await settle();

    expect(fastdlApi.status).toHaveBeenCalledExactlyOnceWith(21);
    expect(status(selected)).toBe('status_installed');
    expect(content(selected)).toContain('0.2.3');
    expect(buttons(selected).map(content)).toEqual(['settings', 'update']);
    expect(vi.getTimerCount()).toBe(0);

    await invoke(find(selected, 'GButton', 'settings'), 'onClick');
    await settle();
    expect(find(root, 'NodeSettings').props.node).toMatchObject({ id: 21, config });
  });

  it('keeps settings and update available for an installed node', async () => {
    const root = await mountPage();
    const card = find(root, 'NCard', 'Existing node');
    expect(buttons(card).map(content)).toEqual(['settings', 'update']);

    await invoke(find(card, 'GButton', 'settings'), 'onClick');
    await settle();
    const settings = find(root, 'NodeSettings');
    expect(settings.props.mode).toBe('settings');
    expect(settings.props.node).toMatchObject({ id: 35, name: 'Existing node' });
    await invoke(settings, 'onClose');
    await settle();

    await invoke(find(find(root, 'NCard', 'Existing node'), 'GButton', 'update'), 'onClick');
    await settle();
    const update = find(root, 'NodeInstall');
    expect(update.props.node).toMatchObject({ id: 35, name: 'Existing node', config: nodeFixtures()[2].config });
    expect(descendants(root).some((node) => node.tag === 'NodeSettings')).toBe(false);

    invoke(update, 'onStarted', { status: 'installing' });
    await settle();
    expect(status(find(root, 'NCard', 'Existing node'))).toBe('status_installing');
    expect(descendants(root).some((node) => node.tag === 'NodeInstall')).toBe(false);
    expect(vi.getTimerCount()).toBe(1);
  });
});
