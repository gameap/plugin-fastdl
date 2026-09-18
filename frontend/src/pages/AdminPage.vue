<template>
  <div v-if="isAdmin" class="fastdl-admin">
    <GBreadcrumbs :items="breadcrumbs" />
    <div class="fastdl-toolbar">
      <GButton type="button" color="white" :loading="loading" @click="load"><GIcon name="refresh" class="mr-1" />{{ trans('refresh') }}</GButton>
      <NInput v-model:value="search" clearable :placeholder="trans('search_nodes')" :input-props="{ 'aria-label': trans('search_nodes') }" style="width: min(100%, 360px)" />
    </div>
    <NAlert v-if="error" type="error">{{ error }}</NAlert>
    <NAlert v-if="pollError" type="warning">{{ trans('polling_failed') }}</NAlert>
    <div v-if="loading && nodes.length === 0" class="py-10 text-center"><NSpin /></div>
    <GEmpty v-else-if="filteredNodes.length === 0" :description="trans('no_nodes')" />
    <div v-else class="fastdl-nodes">
      <NCard v-for="node in filteredNodes" :key="node.id" size="small">
        <template #header>
          <div class="fastdl-node-header">
            <GIcon name="server" class="flex-none" />
            <span class="fastdl-name">{{ node.name }}</span>
            <GStatusBadge :color="statusColor(node.status)" :text="trans(`status_${node.status}`)" />
          </div>
        </template>
        <div class="fastdl-node-content">
          <p class="fastdl-node-os text-sm text-stone-500">
            <GIcon :name="osIcon(node.os)" aria-hidden="true" />
            <span>{{ node.os }}<span v-if="node.version"> · {{ node.version }}</span></span>
          </p>
          <p class="text-sm">{{ trans('enabled_servers').replace('{count}', String(node.enabled_servers)) }}</p>
          <NAlert v-if="node.error_message" type="error">
            <div class="fastdl-install-error">
              <p>{{ node.error_message }}</p>
              <RouterLink
                v-if="node.status === 'failed' && (node.task_id ?? 0) > 0"
                :to="{ name: 'admin.gdaemon_tasks.output', params: { id: node.task_id } }"
                class="text-info underline hover:no-underline"
              >{{ trans('open_daemon_task') }}</RouterLink>
            </div>
          </NAlert>
        </div>
        <template #footer>
          <div class="fastdl-node-actions">
            <GButton v-if="node.status === 'installed'" type="button" color="black" size="small" :disabled="loading" @click="openSettings(node)"><GIcon name="settings" class="mr-1" />{{ trans('settings') }}</GButton>
            <GButton type="button" color="black" size="small" :disabled="loading || node.status === 'installing'" @click="openInstall(node)"><GIcon name="download" class="mr-1" />{{ trans(node.status === 'installed' ? 'update' : 'install') }}</GButton>
          </div>
        </template>
      </NCard>
    </div>
    <NodeSettings v-if="settingsNode" :key="settingsNode.id" :node="settingsNode" :mode="settingsMode" @close="onSettingsClosed" @saved="onSettingsClosed" @started="onInstallStarted" />
    <NodeInstall v-if="installNode" :key="installNode.id" :node="installNode" @close="installNode = null" @started="onInstallStarted" />
  </div>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue';
import { NAlert, NCard, NInput, NSpin } from 'naive-ui';
import { RouterLink, useRouter } from 'vue-router';
import { useIsAdmin, usePluginTrans } from '@gameap/plugin-sdk';
import { errorMessage, fastdlApi, type FastDLNode, type InstallState, type NodeConfig, type NodeStatus } from '../api';
import NodeSettings from '../components/NodeSettings.vue';
import NodeInstall from '../components/NodeInstall.vue';

const { trans } = usePluginTrans();
const isAdmin = useIsAdmin();
const router = useRouter();
const breadcrumbs = computed(() => [{ route: '/', text: 'GameAP', icon: 'gicon gicon-gameap' }, { text: trans('fastdl') }, { text: trans('nodes') }]);
const nodes = ref<FastDLNode[]>([]);
const search = ref('');
const loading = ref(false);
const error = ref('');
const pollError = ref(false);
const settingsNode = ref<FastDLNode | null>(null);
const settingsMode = ref<'settings' | 'install'>('settings');
const installNode = ref<FastDLNode | null>(null);
const filteredNodes = computed(() => nodes.value.filter((node) => node.name.toLocaleLowerCase().includes(search.value.trim().toLocaleLowerCase())));
let timer: ReturnType<typeof setTimeout> | undefined;
let disposed = false;
let generation = 0;

function osIcon(os: string): string {
  const name = String(os || '').trim().toLowerCase();
  if (name.startsWith('w')) return 'windows';
  if (name.startsWith('m')) return 'apple';
  return 'linux';
}

function statusColor(status: InstallState): string {
  return { installed: 'green', installing: 'blue', failed: 'red', not_installed: 'stone' }[status];
}

function schedulePoll() {
  clearTimeout(timer);
  if (!disposed && nodes.value.some((node) => node.status === 'installing')) timer = setTimeout(poll, 4000);
}

async function poll() {
  const request = generation;
  const installing = nodes.value.filter((node) => node.status === 'installing');
  const results = await Promise.allSettled(installing.map((node) => fastdlApi.status(node.id)));
  if (disposed || request !== generation) return;
  pollError.value = results.some((result) => result.status === 'rejected');
  results.forEach((result, index) => {
    if (result.status !== 'fulfilled') return;
    const node = nodes.value.find((item) => item.id === installing[index].id);
    if (node) Object.assign(node, result.value);
  });
  schedulePoll();
}

async function load() {
  if (loading.value) return;
  clearTimeout(timer);
  const request = ++generation;
  loading.value = true;
  error.value = '';
  pollError.value = false;
  try {
    const result = await fastdlApi.nodes();
    if (disposed || request !== generation) return;
    nodes.value = result;
    schedulePoll();
  } catch (e) {
    if (!disposed && request === generation) {
      error.value = errorMessage(e, trans('load_failed'), trans);
      schedulePoll();
    }
  } finally {
    if (!disposed && request === generation) loading.value = false;
  }
}

function openSettings(node: FastDLNode) {
  settingsMode.value = 'settings';
  settingsNode.value = node;
}

function openInstall(node: FastDLNode) {
  if (node.status === 'installed') {
    installNode.value = node;
  } else {
    settingsMode.value = 'install';
    settingsNode.value = node;
  }
}

function onSettingsClosed() {
  settingsNode.value = null;
  void load();
}

function onInstallStarted(status: NodeStatus, config?: NodeConfig) {
  generation++;
  const nodeId = installNode.value?.id ?? settingsNode.value?.id;
  const node = nodes.value.find((item) => item.id === nodeId);
  if (node) {
    Object.assign(node, status);
    if (config) node.config = config;
  }
  installNode.value = null;
  settingsNode.value = null;
  schedulePoll();
}

onMounted(() => {
  if (!isAdmin.value) {
    void router.replace({ name: 'error403' });
    return;
  }
  void load();
});
onBeforeUnmount(() => { disposed = true; generation++; clearTimeout(timer); });
</script>

<style scoped>
.fastdl-admin,
.fastdl-node-content { display: flex; flex-direction: column; min-width: 0; }
.fastdl-admin { gap: 1rem; }
.fastdl-node-content { gap: .75rem; }
.fastdl-install-error { display: flex; flex-direction: column; align-items: flex-start; gap: .5rem; }
.fastdl-node-content p { margin: 0; }
.fastdl-node-os { display: flex; align-items: center; gap: .375rem; }
.fastdl-toolbar,
.fastdl-node-header,
.fastdl-node-actions { display: flex; flex-wrap: wrap; align-items: center; gap: .5rem; min-width: 0; }
.fastdl-toolbar { gap: .75rem; }
.fastdl-nodes { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 320px), 1fr)); gap: 1rem; }
.fastdl-name { overflow-wrap: anywhere; }
</style>
