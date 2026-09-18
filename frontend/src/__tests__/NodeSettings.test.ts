import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { h, type App } from 'vue';
import { fastdlApi, type FastDLNode, type NodeConfig, type NodeStatus } from '../api';
import NodeSettings from '../components/NodeSettings.vue';
import { content, deferred, descendants, find, invoke, mountComponent, settle } from './memory-renderer';

vi.mock('@gameap/plugin-sdk', () => ({ usePluginTrans: () => ({ trans: (key: string) => key }) }));
vi.mock('naive-ui', async () => {
  const { defineComponent, h } = await import('vue');
  return Object.fromEntries(['NAlert', 'NForm', 'NFormItem', 'NInput'].map((name) => [name,
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
  vi.spyOn(fastdlApi, 'setup').mockResolvedValue({ status: 'installing', task_id: 12 });
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
    await invoke(find(root, 'NForm'), 'onSubmit', { preventDefault() {} });
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
    await invoke(find(root, 'NForm'), 'onSubmit', { preventDefault() {} });
    await settle();
    expect(saveNode).toHaveBeenCalledTimes(2);
    expect(saveNode).toHaveBeenLastCalledWith(node.id, { listen: node.config.listen, public_base_url: 'https://new-downloads.example.test' });
    expect(saved).toHaveBeenCalledOnce();
    expect(window.$message?.success).toHaveBeenCalledOnce();
    expect(fastdlApi.setup).not.toHaveBeenCalled();
  });
});

describe('installation settings', () => {
  it.each(['', '   ', 'downloads.example.test', 'https://downloads.example.test?private=1'])(
    'disables installation and prevents requests for invalid public address %j',
    async (publicUrl) => {
      const saveNode = vi.spyOn(fastdlApi, 'saveNode').mockResolvedValue(node.config);
      const mounted = await mountComponent(() => h(NodeSettings, {
        node: { ...node, config: { ...node.config, public_base_url: publicUrl } }, mode: 'install',
      }));
      app = mounted.app;
      const { root } = mounted;

      expect(find(root, 'GButton', 'install').props.disabled).toBe(true);
      await invoke(find(root, 'NForm'), 'onSubmit', { preventDefault() {} });
      expect(find(root, 'NAlert', 'public_url_invalid')).toBeDefined();
      expect(saveNode).not.toHaveBeenCalled();
      expect(fastdlApi.setup).not.toHaveBeenCalled();

      const input = descendants(root).find((item) => item.tag === 'NInput' && item.props.placeholder === 'http://fastdl.example.com:8080')!;
      invoke(input, 'onUpdate:value', ' https://downloads.example.test/ ');
      await settle();
      expect(find(root, 'GButton', 'install').props.disabled).toBe(false);
    },
  );

  it('disables installation when the listen address is invalid', async () => {
    const saveNode = vi.spyOn(fastdlApi, 'saveNode').mockResolvedValue(node.config);
    const mounted = await mountComponent(() => h(NodeSettings, {
      node: { ...node, config: { ...node.config, listen: '0.0.0.0:0' } }, mode: 'install',
    }));
    app = mounted.app;
    const { root } = mounted;

    expect(find(root, 'GButton', 'install').props.disabled).toBe(true);
    await invoke(find(root, 'NForm'), 'onSubmit', { preventDefault() {} });
    expect(find(root, 'NAlert', 'listen_invalid')).toBeDefined();
    expect(saveNode).not.toHaveBeenCalled();
    expect(fastdlApi.setup).not.toHaveBeenCalled();
  });

  it('saves normalized settings before starting installation and keeps the modal pending through both steps', async () => {
    const saved = vi.fn();
    const started = vi.fn();
    const closed = vi.fn();
    const saving = deferred<NodeConfig>();
    const installing = deferred<NodeStatus>();
    const saveNode = vi.spyOn(fastdlApi, 'saveNode').mockReturnValue(saving.promise);
    vi.mocked(fastdlApi.setup).mockReturnValue(installing.promise);
    const mounted = await mountComponent(() => h(NodeSettings, {
      node: { ...node, status: 'not_installed', config: { listen: ' 0.0.0.0:8080 ', public_base_url: ' https://downloads.example.test/// ' } },
      mode: 'install', onSaved: saved, onStarted: started, onClose: closed,
    }));
    app = mounted.app;
    const { root } = mounted;

    expect(find(root, 'GModal').props.title).toBe('install · Test node');
    expect(content(root)).toContain('install_notice');
    expect(content(root)).toContain('network_notice');
    expect(content(root)).not.toContain('node_save_hint');
    const submission = invoke(find(root, 'NForm'), 'onSubmit', { preventDefault() {} });
    await settle();

    expect(saveNode).toHaveBeenCalledWith(node.id, node.config);
    expect(fastdlApi.setup).not.toHaveBeenCalled();
    expect(find(root, 'GButton', 'install').props.disabled).toBe(true);
    expect(find(root, 'GButton', 'cancel').props.disabled).toBe(true);
    expect(descendants(root).filter((item) => item.tag === 'NInput').every((item) => item.props.disabled)).toBe(true);
    await invoke(find(root, 'NForm'), 'onSubmit', { preventDefault() {} });
    invoke(find(root, 'GModal'), 'onUpdate:show', false);
    expect(saveNode).toHaveBeenCalledOnce();
    expect(closed).not.toHaveBeenCalled();

    saving.resolve(node.config);
    await settle();
    expect(fastdlApi.setup).toHaveBeenCalledExactlyOnceWith(node.id);
    expect(find(root, 'GButton', 'install').props.disabled).toBe(true);
    expect(saved).not.toHaveBeenCalled();
    expect(started).not.toHaveBeenCalled();
    expect(window.$message?.success).not.toHaveBeenCalled();

    const status: NodeStatus = { status: 'installing', task_id: 12 };
    installing.resolve(status);
    await submission;
    await settle();
    expect(started).toHaveBeenCalledExactlyOnceWith(status, node.config);
    expect(saved).not.toHaveBeenCalled();
    expect(closed).not.toHaveBeenCalled();
    expect(window.$message?.success).toHaveBeenCalledExactlyOnceWith('install_started');
  });

  it.each([
    [new Error('connection lost'), 'save_failed'],
    [{ response: { data: { code: 'CONFIGURE_FAILED' } } }, 'node_save_configure_failed'],
  ])('keeps the draft and does not install when saving fails (%s)', async (failure, expectedKey) => {
    const saved = vi.fn();
    const started = vi.fn();
    const closed = vi.fn();
    vi.spyOn(fastdlApi, 'saveNode').mockRejectedValue(failure);
    const mounted = await mountComponent(() => h(NodeSettings, {
      node, mode: 'install', onSaved: saved, onStarted: started, onClose: closed,
    }));
    app = mounted.app;
    const { root } = mounted;
    const input = descendants(root).find((item) => item.tag === 'NInput' && item.props.placeholder === 'http://fastdl.example.com:8080')!;
    invoke(input, 'onUpdate:value', 'https://new-downloads.example.test');
    await settle();
    await invoke(find(root, 'NForm'), 'onSubmit', { preventDefault() {} });
    await settle();

    expect(find(root, 'NAlert', expectedKey)).toBeDefined();
    expect(input.props.value).toBe('https://new-downloads.example.test');
    expect(input.props.disabled).toBe(false);
    expect(find(root, 'GButton', 'install').props.disabled).toBe(false);
    expect(fastdlApi.setup).not.toHaveBeenCalled();
    expect(saved).not.toHaveBeenCalled();
    expect(started).not.toHaveBeenCalled();
    expect(closed).not.toHaveBeenCalled();
    expect(window.$message?.success).not.toHaveBeenCalled();
  });

  it.each([
    [new Error('connection lost'), 'install_failed'],
    [{ response: { data: { code: 'NODE_UNAVAILABLE' } } }, 'node_unavailable'],
  ])('keeps the draft and retries both steps when installation fails (%s)', async (failure, expectedKey) => {
    const saved = vi.fn();
    const started = vi.fn();
    const closed = vi.fn();
    const config: NodeConfig = { ...node.config, public_base_url: 'https://new-downloads.example.test' };
    const saveNode = vi.spyOn(fastdlApi, 'saveNode').mockResolvedValue(config);
    vi.mocked(fastdlApi.setup).mockRejectedValueOnce(failure);
    const mounted = await mountComponent(() => h(NodeSettings, {
      node, mode: 'install', onSaved: saved, onStarted: started, onClose: closed,
    }));
    app = mounted.app;
    const { root } = mounted;
    const input = descendants(root).find((item) => item.tag === 'NInput' && item.props.placeholder === 'http://fastdl.example.com:8080')!;
    invoke(input, 'onUpdate:value', ' https://new-downloads.example.test/ ');
    await settle();
    await invoke(find(root, 'NForm'), 'onSubmit', { preventDefault() {} });
    await settle();

    expect(find(root, 'NAlert', expectedKey)).toBeDefined();
    expect(input.props.value).toBe(' https://new-downloads.example.test/ ');
    expect(find(root, 'GButton', 'install').props.disabled).toBe(false);
    expect(saved).not.toHaveBeenCalled();
    expect(started).not.toHaveBeenCalled();
    expect(closed).not.toHaveBeenCalled();
    expect(window.$message?.success).not.toHaveBeenCalled();

    await invoke(find(root, 'NForm'), 'onSubmit', { preventDefault() {} });
    await settle();
    expect(saveNode).toHaveBeenCalledTimes(2);
    expect(saveNode).toHaveBeenLastCalledWith(node.id, config);
    expect(fastdlApi.setup).toHaveBeenCalledTimes(2);
    expect(started).toHaveBeenCalledExactlyOnceWith({ status: 'installing', task_id: 12 }, config);
    expect(saved).not.toHaveBeenCalled();
    expect(window.$message?.success).toHaveBeenCalledExactlyOnceWith('install_started');
  });
});
