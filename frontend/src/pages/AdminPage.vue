<template>
  <div v-if="isAdmin" class="fastdl-admin space-y-4">
    <GBreadcrumbs :items="breadcrumbs" />
    <p class="text-stone-500">{{ trans('introduction') }}</p>
    <div class="flex flex-wrap items-center gap-3">
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
          <div class="flex min-w-0 flex-wrap items-center gap-2">
            <GIcon name="server" class="flex-none" />
            <span class="fastdl-name">{{ node.name }}</span>
            <GStatusBadge :color="statusColor(node.status)" :text="trans(`status_${node.status}`)" />
          </div>
        </template>
        <div class="space-y-3">
          <p class="text-sm text-stone-500">{{ node.os }}<span v-if="node.version"> · {{ node.version }}</span></p>
          <p class="text-sm">{{ trans('enabled_servers').replace('{count}', String(node.enabled_servers)) }}</p>
          <p v-if="node.config.public_base_url" class="fastdl-url text-sm">{{ node.config.public_base_url }}</p>
          <NAlert v-if="node.error_message" type="error">{{ node.error_message }}</NAlert>
          <NAlert v-if="!node.config.public_base_url && node.status !== 'installing'" type="info">{{ trans('configure_first') }}</NAlert>
          <div class="flex flex-wrap gap-2">
            <GButton type="button" color="black" size="small" :disabled="loading || node.status === 'installing' || operatingId === node.id" @click="settingsNode = node"><GIcon name="settings" class="mr-1" />{{ trans('settings') }}</GButton>
            <GButton type="button" color="black" size="small" :disabled="loading || node.status === 'installing' || operatingId === node.id || !node.config.public_base_url" @click="installNode = node"><GIcon name="download" class="mr-1" />{{ trans(node.status === 'installed' ? 'update' : 'install') }}</GButton>
            <GButton type="button" v-if="node.status === 'installed'" color="black" size="small" :title="trans('sync_hint')" :loading="operatingId === node.id" :disabled="loading || operatingId !== null && operatingId !== node.id" @click="sync(node)">{{ trans('sync') }}</GButton>
          </div>
          <router-link v-if="node.task_id" :to="{ name: 'admin.gdaemon_tasks.output', params: { id: node.task_id } }" class="inline-block text-sm underline">{{ trans('task_output') }}</router-link>
        </div>
      </NCard>
    </div>
    <NodeSettings v-if="settingsNode" :key="settingsNode.id" :node="settingsNode" @close="onSettingsClosed" @saved="onSettingsClosed" />
    <NodeInstall v-if="installNode" :key="installNode.id" :node="installNode" @close="installNode = null" @started="onInstallStarted" />
  </div>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue';
import { NAlert, NCard, NInput, NSpin } from 'naive-ui';
import { useRouter } from 'vue-router';
import { useIsAdmin, usePluginTrans } from '@gameap/plugin-sdk';
import { errorMessage, fastdlApi, type FastDLNode, type InstallState, type NodeStatus } from '../api';
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
const installNode = ref<FastDLNode | null>(null);
const operatingId = ref<number | null>(null);
const filteredNodes = computed(() => nodes.value.filter((node) => node.name.toLocaleLowerCase().includes(search.value.trim().toLocaleLowerCase())));
let timer: ReturnType<typeof setTimeout> | undefined;
let disposed = false;
let generation = 0;

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

function onSettingsClosed() {
  settingsNode.value = null;
  void load();
}

function onInstallStarted(status: NodeStatus) {
  generation++;
  const node = nodes.value.find((item) => item.id === installNode.value?.id);
  if (node) Object.assign(node, status);
  installNode.value = null;
  schedulePoll();
}

async function sync(node: FastDLNode) {
  if (operatingId.value !== null) return;
  operatingId.value = node.id;
  error.value = '';
  try {
    await fastdlApi.sync(node.id);
    if (!disposed) window.$message?.success(trans('synced'));
  } catch (e) {
    if (!disposed) error.value = errorMessage(e, trans('sync_failed'), trans, 'node-sync');
  } finally {
    operatingId.value = null;
  }
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
.fastdl-nodes { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 320px), 1fr)); gap: 1rem; }
.fastdl-name { overflow-wrap: anywhere; }
.fastdl-url { overflow-wrap: anywhere; }
</style>
