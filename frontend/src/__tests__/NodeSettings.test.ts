import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { h, type App } from 'vue';
import { fastdlApi, type FastDLNode } from '../api';
import NodeSettings from '../components/NodeSettings.vue';
import { content, descendants, find, invoke, mountComponent, settle } from './memory-renderer';

vi.mock('@gameap/plugin-sdk', () => ({ usePluginTrans: () => ({ trans: (key: string) => key }) }));
vi.mock('naive-ui', async () => {
  const { defineComponent, h } = await import('vue');
  return Object.fromEntries(['NAlert', 'NFormItem', 'NInput'].map((name) => [name,
    defineComponent({ inheritAttrs: false, setup: (_, { attrs, slots }) => () => h(name, attrs, slots.default?.()) }),
  ]));
});

const node: FastDLNode = {
  id: 1, name: 'Test node', os: 'linux', status: 'installed', enabled_servers: 2,
  config: { listen: '0.0.0.0:8080', public_base_url: 'https://downloads.example.test' },
};
let app: App | undefined;

beforeEach(() => {
  vi.stubGlobal('window', { $message: { success: vi.fn() } });
});
afterEach(() => {
  app?.unmount();
  app = undefined;
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe('node settings application errors', () => {
  it.each([
    ['CONFIGURE_FAILED', undefined, 'node_save_configure_failed'],
    ['GAME_CONFIG_UPDATE_FAILED', 'Half-Life', 'node_save_game_config_update_failed'],
  ])('keeps the settings and modal open for retry after %s', async (code, serverName, expectedKey) => {
    const saved = vi.fn();
    const closed = vi.fn();
    const saveNode = vi.spyOn(fastdlApi, 'saveNode').mockRejectedValue({
      response: { data: { code, server_name: serverName, message: 'Game configuration update could not be verified' } },
    });
    const mounted = await mountComponent(() => h(NodeSettings, { node, onSaved: saved, onClose: closed }));
    app = mounted.app;
    const root = mounted.root;
    const input = descendants(root).find((item) => item.tag === 'NInput' && item.props.placeholder === 'http://fastdl.example.com:8080')!;
    invoke(input, 'onUpdate:value', 'https://new-downloads.example.test');
    await settle();
    await invoke(find(root, 'GButton', 'save'), 'onClick');
    await settle();

    expect(find(root, 'NAlert', expectedKey)).toBeDefined();
    expect(find(root, 'GModal').props.show).toBe(true);
    expect(input.props.value).toBe('https://new-downloads.example.test');
    expect(input.props.disabled).toBe(false);
    expect(find(root, 'GButton', 'save').props.loading).toBe(false);
    expect(saved).not.toHaveBeenCalled();
    expect(closed).not.toHaveBeenCalled();
    expect(window.$message?.success).not.toHaveBeenCalled();
    expect(content(root)).not.toContain('Game configuration update could not be verified');

    saveNode.mockResolvedValue({ ...node.config, public_base_url: 'https://new-downloads.example.test' });
    await invoke(find(root, 'GButton', 'save'), 'onClick');
    await settle();
    expect(saveNode).toHaveBeenCalledTimes(2);
    expect(saveNode).toHaveBeenLastCalledWith(node.id, { listen: node.config.listen, public_base_url: 'https://new-downloads.example.test' });
    expect(saved).toHaveBeenCalledOnce();
    expect(window.$message?.success).toHaveBeenCalledOnce();
  });
});
