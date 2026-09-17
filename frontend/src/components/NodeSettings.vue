<template>
  <GModal :show="true" :title="`${trans('settings')} · ${node.name}`" style="width: 600px; max-width: 94vw" @update:show="close">
    <form class="space-y-4" @submit.prevent="save">
      <NAlert v-if="error" type="error">{{ error }}</NAlert>
      <NFormItem :label="trans('listen')" :show-feedback="false">
        <div class="w-full">
          <NInput v-model:value="config.listen" :disabled="saving" placeholder="0.0.0.0:8080" />
          <p class="mt-1 text-sm text-stone-500">{{ trans('listen_hint') }}</p>
        </div>
      </NFormItem>
      <NFormItem :label="trans('public_url')" :show-feedback="false">
        <div class="w-full">
          <NInput v-model:value="config.public_base_url" :disabled="saving" placeholder="http://fastdl.example.com:8080" />
          <p class="mt-1 text-sm text-stone-500">{{ trans('public_url_hint') }}</p>
        </div>
      </NFormItem>
      <NAlert type="info">{{ trans('network_notice') }}</NAlert>
      <div class="flex flex-wrap gap-2">
        <GButton type="button" color="black" :loading="saving" @click="save"><GIcon name="save" class="mr-1" />{{ trans('save') }}</GButton>
        <GButton type="button" color="black" :disabled="saving" @click="close">{{ trans('cancel') }}</GButton>
      </div>
    </form>
  </GModal>
</template>

<script setup lang="ts">
import { reactive, ref } from 'vue';
import { NAlert, NFormItem, NInput } from 'naive-ui';
import { usePluginTrans } from '@gameap/plugin-sdk';
import { fastdlApi, errorMessage, type FastDLNode } from '../api';
import { isListenAddress, isPublicUrl, normalizeNodeConfig } from '../lib/settings';

const props = defineProps<{ node: FastDLNode }>();
const emit = defineEmits<{ close: []; saved: [] }>();
const { trans } = usePluginTrans();
const config = reactive({ ...props.node.config });
const saving = ref(false);
const error = ref('');

function close() {
  if (!saving.value) emit('close');
}

async function save() {
  if (saving.value) return;
  const value = normalizeNodeConfig(config);
  error.value = !isListenAddress(value.listen) ? trans('listen_invalid')
    : !isPublicUrl(value.public_base_url) ? trans('public_url_invalid') : '';
  if (error.value) return;
  saving.value = true;
  try {
    await fastdlApi.saveNode(props.node.id, value);
    window.$message?.success(trans('saved'));
    emit('saved');
  } catch (e) {
    error.value = errorMessage(e, trans('save_failed'));
  } finally {
    saving.value = false;
  }
}
</script>
