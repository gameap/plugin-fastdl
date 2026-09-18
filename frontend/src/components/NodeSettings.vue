<template>
  <GModal :show="true" :title="`${trans(isInstall ? 'install' : 'settings')} · ${node.name}`" style="width: 600px; max-width: 94vw" @update:show="close">
    <NForm class="node-settings-form" label-placement="top" size="medium" :show-feedback="false" @submit.prevent="save">
      <NAlert v-if="error" type="error">{{ error }}</NAlert>
      <NFormItem :label="trans('listen')" :label-props="{ for: 'fastdl-node-listen' }">
        <div class="node-settings-field">
          <NInput v-model:value="config.listen" :disabled="saving" placeholder="0.0.0.0:8080" :input-props="{ id: 'fastdl-node-listen', 'aria-describedby': 'fastdl-node-listen-hint' }" />
          <p id="fastdl-node-listen-hint" class="node-settings-hint">{{ trans('listen_hint') }}</p>
        </div>
      </NFormItem>
      <NFormItem :label="trans('public_url')" :label-props="{ for: 'fastdl-node-public-url' }">
        <div class="node-settings-field">
          <NInput v-model:value="config.public_base_url" :disabled="saving" placeholder="http://fastdl.example.com:8080" :input-props="{ id: 'fastdl-node-public-url', 'aria-describedby': 'fastdl-node-public-url-hint' }" />
          <p id="fastdl-node-public-url-hint" class="node-settings-hint">{{ trans('public_url_hint') }}</p>
        </div>
      </NFormItem>
      <NAlert type="info">{{ trans('network_notice') }}</NAlert>
      <p class="node-settings-hint">{{ trans(isInstall ? 'install_notice' : 'node_save_hint') }}</p>
      <div class="node-settings-actions">
        <GButton type="submit" color="black" :loading="saving" :disabled="saving || (isInstall && !validConfig)"><GIcon :name="isInstall ? 'download' : 'save'" class="mr-1" />{{ trans(isInstall ? 'install' : 'save') }}</GButton>
        <GButton type="button" color="black" :disabled="saving" @click="close">{{ trans('cancel') }}</GButton>
      </div>
    </NForm>
  </GModal>
</template>

<script setup lang="ts">
import { computed, reactive, ref } from 'vue';
import { NAlert, NForm, NFormItem, NInput } from 'naive-ui';
import { usePluginTrans } from '@gameap/plugin-sdk';
import { fastdlApi, errorMessage, type FastDLNode, type NodeConfig, type NodeStatus } from '../api';
import { isListenAddress, isPublicUrl, normalizeNodeConfig } from '../lib/settings';

const props = withDefaults(defineProps<{ node: FastDLNode; mode?: 'settings' | 'install' }>(), { mode: 'settings' });
const emit = defineEmits<{ close: []; saved: []; started: [status: NodeStatus, config: NodeConfig] }>();
const { trans } = usePluginTrans();
const config = reactive({ ...props.node.config });
const saving = ref(false);
const error = ref('');
const isInstall = computed(() => props.mode === 'install');
const validConfig = computed(() => {
  const value = normalizeNodeConfig(config);
  return isListenAddress(value.listen) && isPublicUrl(value.public_base_url);
});

function close() {
  if (!saving.value) emit('close');
}

async function save() {
  if (saving.value) return;
  const value = normalizeNodeConfig(config);
  error.value = !isListenAddress(value.listen) ? trans('listen_invalid')
    : !isPublicUrl(value.public_base_url) ? trans('public_url_invalid') : '';
  if (error.value) return;
  const nodeId = props.node.id;
  const install = isInstall.value;
  let stage: 'save' | 'install' = 'save';
  saving.value = true;
  try {
    const savedConfig = await fastdlApi.saveNode(nodeId, value);
    if (install) {
      stage = 'install';
      const status = await fastdlApi.setup(nodeId);
      window.$message?.success(trans('install_started'));
      emit('started', status, savedConfig);
    } else {
      window.$message?.success(trans('saved'));
      emit('saved');
    }
  } catch (e) {
    error.value = stage === 'save'
      ? errorMessage(e, trans('save_failed'), trans, 'node-save')
      : errorMessage(e, trans('install_failed'), trans);
  } finally {
    saving.value = false;
  }
}
</script>

<style scoped>
.node-settings-form { display: flex; flex-direction: column; gap: 1.25rem; min-width: 0; }
.node-settings-field { width: 100%; min-width: 0; }
.node-settings-hint { margin: .25rem 0 0; font-size: .75rem; line-height: 1rem; color: var(--gameap-text-muted, #78716c); }
.node-settings-actions { display: flex; flex-wrap: wrap; align-items: center; gap: .5rem; }
</style>
