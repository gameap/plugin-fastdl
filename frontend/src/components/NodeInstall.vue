<template>
  <GModal :show="true" :title="`${trans(node.status === 'installed' ? 'update' : 'install')} · ${node.name}`" style="width: 640px; max-width: 94vw" @update:show="close">
    <form class="space-y-4" @submit.prevent="install">
      <NAlert v-if="error" type="error">{{ error }}</NAlert>
      <NAlert type="info">{{ trans('install_notice') }}</NAlert>
      <div class="flex flex-wrap gap-2">
        <GButton type="button" color="black" :loading="saving" @click="install"><GIcon name="download" class="mr-1" />{{ trans(node.status === 'installed' ? 'update' : 'install') }}</GButton>
        <GButton type="button" color="black" :disabled="saving" @click="close">{{ trans('cancel') }}</GButton>
      </div>
    </form>
  </GModal>
</template>

<script setup lang="ts">
import { ref } from 'vue';
import { NAlert } from 'naive-ui';
import { usePluginTrans } from '@gameap/plugin-sdk';
import { fastdlApi, errorMessage, type FastDLNode, type NodeStatus } from '../api';

const props = defineProps<{ node: FastDLNode }>();
const emit = defineEmits<{ close: []; started: [status: NodeStatus] }>();
const { trans } = usePluginTrans();
const saving = ref(false);
const error = ref('');

function close() {
  if (!saving.value) emit('close');
}

async function install() {
  if (saving.value) return;
  error.value = '';
  saving.value = true;
  try {
    const status = await fastdlApi.setup(props.node.id);
    window.$message?.success(trans('install_started'));
    emit('started', status);
  } catch (e) {
    error.value = errorMessage(e, trans('install_failed'), trans);
  } finally {
    saving.value = false;
  }
}
</script>
