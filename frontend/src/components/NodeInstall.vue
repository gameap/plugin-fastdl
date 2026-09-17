<template>
  <GModal :show="true" :title="`${trans(node.status === 'installed' ? 'update' : 'install')} · ${node.name}`" style="width: 640px; max-width: 94vw" @update:show="close">
    <form class="space-y-4" @submit.prevent="install">
      <NAlert v-if="error" type="error">{{ error }}</NAlert>
      <NAlert type="info">{{ trans('install_notice') }}</NAlert>
      <NFormItem :label="trans('download_binary')" :show-feedback="false">
        <div class="w-full">
          <NInput v-model:value="request.download_url" :disabled="saving" placeholder="https://" />
          <p class="mt-1 text-sm text-stone-500">{{ trans('download_binary_hint') }}</p>
        </div>
      </NFormItem>
      <NFormItem :label="trans('checksum')" :show-feedback="false">
        <div class="w-full">
          <NInput v-model:value="request.sha256" :disabled="saving" :maxlength="64" />
          <p class="mt-1 text-sm text-stone-500">{{ trans('checksum_hint') }}</p>
        </div>
      </NFormItem>
      <div class="flex flex-wrap gap-2">
        <GButton type="button" color="black" :loading="saving" @click="install"><GIcon name="download" class="mr-1" />{{ trans(node.status === 'installed' ? 'update' : 'install') }}</GButton>
        <GButton type="button" color="black" :disabled="saving" @click="close">{{ trans('cancel') }}</GButton>
      </div>
    </form>
  </GModal>
</template>

<script setup lang="ts">
import { reactive, ref } from 'vue';
import { NAlert, NFormItem, NInput } from 'naive-ui';
import { usePluginTrans } from '@gameap/plugin-sdk';
import { fastdlApi, errorMessage, type FastDLNode, type NodeStatus } from '../api';
import { isDownloadUrl } from '../lib/settings';

const props = defineProps<{ node: FastDLNode }>();
const emit = defineEmits<{ close: []; started: [status: NodeStatus] }>();
const { trans } = usePluginTrans();
const request = reactive({ download_url: '', sha256: '' });
const saving = ref(false);
const error = ref('');

function close() {
  if (!saving.value) emit('close');
}

async function install() {
  if (saving.value) return;
  const value = { download_url: request.download_url.trim(), sha256: request.sha256.trim().toLowerCase() };
  error.value = !isDownloadUrl(value.download_url) ? trans('binary_invalid')
    : !/^[0-9a-f]{64}$/.test(value.sha256) ? trans('checksum_invalid') : '';
  if (error.value) return;
  saving.value = true;
  try {
    const status = await fastdlApi.setup(props.node.id, value);
    window.$message?.success(trans('install_started'));
    emit('started', status);
  } catch (e) {
    error.value = errorMessage(e, trans('install_failed'));
  } finally {
    saving.value = false;
  }
}
</script>
