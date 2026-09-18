<template>
  <div class="fastdl-tab" :class="{ 'fastdl-tab-with-save': data?.can_manage }">
    <div class="fastdl-actions">
      <GButton type="button" color="white" :loading="loading" :disabled="saving || applyingConfiguration || dirty" @click="load">
        <GIcon name="refresh" class="mr-1" />{{ trans('refresh') }}
      </GButton>
    </div>
    <NAlert v-if="error" type="error">{{ error }}</NAlert>
    <div v-if="loading && !data" class="fastdl-loading"><NSpin /></div>
    <template v-if="data">
      <NAlert v-if="!data.supported" type="warning">{{ trans('unsupported') }}</NAlert>
      <NAlert v-if="!data.can_manage" type="info">{{ trans('read_only') }}</NAlert>
      <NAlert v-for="warning in warnings" :key="warning" type="warning">{{ warning }}</NAlert>
      <NCard :title="trans('settings')" size="small">
        <NForm class="fastdl-form" label-placement="top" size="medium" :show-feedback="false" @submit.prevent="save">
          <NCheckbox v-model:checked="form.enabled" :disabled="locked || (!data.node_ready && !form.enabled)">
            {{ trans('enabled') }}
          </NCheckbox>
          <NFormItem :label="trans('game_dir')" :label-props="{ for: 'fastdl-game-dir' }">
            <div class="fastdl-field">
              <NInput v-model:value="form.game_dir" :disabled="locked" placeholder="cstrike" :input-props="{ id: 'fastdl-game-dir', 'aria-describedby': 'fastdl-game-dir-hint' }" />
              <p id="fastdl-game-dir-hint" class="fastdl-hint">{{ trans('game_dir_hint') }}</p>
            </div>
          </NFormItem>
          <div>
            <NCheckbox v-model:checked="form.autoindex" :disabled="locked" aria-describedby="fastdl-autoindex-hint">
              {{ trans('autoindex') }}
            </NCheckbox>
            <p id="fastdl-autoindex-hint" class="fastdl-hint fastdl-option-hint">{{ trans('autoindex_hint') }}</p>
          </div>
          <div v-if="data.engine === 'source'">
            <NCheckbox v-model:checked="form.generate_bz2" :disabled="locked" aria-describedby="fastdl-bz2-hint">
              {{ trans('generate_bz2') }}
            </NCheckbox>
            <p id="fastdl-bz2-hint" class="fastdl-hint fastdl-option-hint">{{ trans('generate_bz2_hint') }}</p>
          </div>
          <p v-if="dirty" class="fastdl-hint">{{ trans('unsaved') }}</p>
          <div v-if="data.can_manage" class="fastdl-save-bar">
            <GButton type="submit" color="green" :loading="saving" :disabled="locked || !pendingApply || (form.enabled && !data.node_ready)">
              <GIcon name="save" />
              <span class="inline">{{ trans('save') }}</span>
            </GButton>
          </div>
        </NForm>
      </NCard>
      <NCard v-if="applied && data.download_url && data.enabled" :title="trans('download_url')" size="small">
        <div class="fastdl-copyable">
          <code class="fastdl-code">{{ data.download_url }}</code>
          <button type="button" class="fastdl-copy-button" :title="trans('copy')" :aria-label="trans('copy')" @click="copy(data.download_url)">
            <Transition name="fastdl-copy-icon" mode="out-in">
              <GIcon v-if="copiedValue === data.download_url" key="check" name="check" class="fastdl-copied-icon" />
              <GIcon v-else key="copy" name="copy" />
            </Transition>
          </button>
        </div>
      </NCard>
      <NCard v-if="applied && data.enabled && data.configuration.length" :title="trans('game_configuration')" size="small">
        <div class="fastdl-result">
          <p class="fastdl-hint">{{ trans('config_apply_hint') }}</p>
          <div class="fastdl-copyable">
            <pre class="fastdl-code fastdl-configuration" tabindex="0">{{ configurationText }}</pre>
            <button type="button" class="fastdl-copy-button" :title="trans('copy')" :aria-label="trans('copy')" @click="copy(configurationText)">
              <Transition name="fastdl-copy-icon" mode="out-in">
                <GIcon v-if="copiedValue === configurationText" key="check" name="check" class="fastdl-copied-icon" />
                <GIcon v-else key="copy" name="copy" />
              </Transition>
            </button>
          </div>
          <div v-if="data.can_manage" class="fastdl-actions">
            <GButton type="button" size="small" color="black" :loading="applyingConfiguration" :disabled="!canApplyConfiguration" @click="applyConfiguration">
              {{ trans('apply_configuration') }}
            </GButton>
          </div>
          <NAlert v-if="configurationError" type="error">{{ configurationError }}</NAlert>
          <NAlert v-if="configurationFeedback" :type="configurationFeedback">
            {{ trans(configurationFeedback === 'success' ? 'configuration_applied' : 'configuration_rcon_failed') }}
          </NAlert>
        </div>
      </NCard>
      <NAlert v-if="copyError" type="warning">{{ trans('copy_failed') }}</NAlert>
    </template>
  </div>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, reactive, ref, watch } from 'vue';
import { NAlert, NCard, NCheckbox, NForm, NFormItem, NInput, NSpin } from 'naive-ui';
import { providePluginTrans } from '@gameap/plugin-sdk';
import { errorMessage, fastdlApi, type ServerFastDL, type ServerSettings } from '../api';
import { editableSettings, isGameDirectory, needsApply, serverWarnings, settingsChanged } from '../lib/settings';

const props = defineProps<{ serverId: number; pluginId: string }>();
const { trans } = providePluginTrans(props.pluginId);
const data = ref<ServerFastDL | null>(null);
const configurationText = computed(() => data.value?.configuration.join('\n') ?? '');
const form = reactive<ServerSettings>({ enabled: false, autoindex: false, game_dir: '', generate_bz2: true });
const loading = ref(false);
const saving = ref(false);
const applyingConfiguration = ref(false);
const configurationError = ref('');
const configurationFeedback = ref<'success' | 'warning' | ''>('');
const error = ref('');
const copyError = ref(false);
const copiedValue = ref('');
let copyTimeout: ReturnType<typeof setTimeout> | undefined;
const saveFailed = ref(false);
const applied = computed(() => data.value?.synced === true && !saveFailed.value);
const dirty = computed(() => data.value !== null && settingsChanged(form, data.value));
const pendingApply = computed(() => data.value !== null && (saveFailed.value || needsApply(form, data.value)));
const warnings = computed(() => data.value ? serverWarnings(data.value, trans) : []);
const locked = computed(() => saving.value || loading.value || applyingConfiguration.value || !data.value?.can_manage || !data.value?.supported);
const canApplyConfiguration = computed(() => !locked.value && !dirty.value && applied.value
  && data.value?.enabled === true && data.value.node_ready && !!data.value.download_url && data.value.configuration.length > 0);
let generation = 0;
let disposed = false;

async function load() {
  if (saving.value || applyingConfiguration.value || dirty.value) return;
  const request = ++generation;
  const serverId = props.serverId;
  loading.value = true;
  error.value = '';
  resetConfigurationFeedback();
  try {
    const result = await fastdlApi.server(serverId);
    if (disposed || request !== generation) return;
    data.value = result;
    saveFailed.value = false;
    Object.assign(form, editableSettings(result));
  } catch (e) {
    if (!disposed && request === generation) error.value = errorMessage(e, trans('load_failed'), trans);
  } finally {
    if (!disposed && request === generation) loading.value = false;
  }
}

async function save() {
  if (locked.value || !pendingApply.value || (form.enabled && !data.value?.node_ready)) return;
  if (!isGameDirectory(form.game_dir.trim())) { error.value = trans('game_dir_invalid'); return; }
  const request = ++generation;
  const serverId = props.serverId;
  const submittedSettings = { ...form, game_dir: form.game_dir.trim() };
  saving.value = true;
  error.value = '';
  resetConfigurationFeedback();
  try {
    const result = await fastdlApi.saveServer(serverId, submittedSettings);
    if (disposed || request !== generation) return;
    data.value = result;
    saveFailed.value = false;
    Object.assign(form, editableSettings(result));
    window.$message?.success(trans('saved'));
  } catch (e) {
    if (disposed || request !== generation) return;
    error.value = errorMessage(e, trans('save_failed'), trans);
    saveFailed.value = true;
    try {
      const result = await fastdlApi.server(serverId);
      if (disposed || request !== generation) return;
      data.value = result;
      if (result.synced && !settingsChanged(submittedSettings, result)) {
        saveFailed.value = false;
        error.value = '';
        Object.assign(form, editableSettings(result));
        window.$message?.success(trans('synced'));
      }
    } catch {
      // Keep the save error and draft when the node is still unavailable.
    }
  } finally {
    if (!disposed && request === generation) saving.value = false;
  }
}

async function applyConfiguration() {
  if (!canApplyConfiguration.value) return;
  const request = ++generation;
  const serverId = props.serverId;
  applyingConfiguration.value = true;
  resetConfigurationFeedback();
  try {
    const result = await fastdlApi.applyConfiguration(serverId);
    if (disposed || request !== generation) return;
    if (!result.configured) {
      configurationError.value = trans('apply_configuration_failed');
      return;
    }
    configurationFeedback.value = result.rcon_applied ? 'success' : 'warning';
  } catch (e) {
    if (!disposed && request === generation) {
      configurationError.value = errorMessage(e, trans('apply_configuration_failed'), trans, 'game-configure');
    }
  } finally {
    if (!disposed && request === generation) applyingConfiguration.value = false;
  }
}

function resetConfigurationFeedback() {
  configurationError.value = '';
  configurationFeedback.value = '';
}

function resetCopyFeedback() {
  if (copyTimeout !== undefined) clearTimeout(copyTimeout);
  copyTimeout = undefined;
  copiedValue.value = '';
}

async function copy(value: string) {
  const serverId = props.serverId;
  copyError.value = false;
  try {
    await navigator.clipboard.writeText(value);
    if (disposed || serverId !== props.serverId) return;
    resetCopyFeedback();
    copiedValue.value = value;
    copyTimeout = setTimeout(resetCopyFeedback, 2000);
  } catch {
    if (!disposed && serverId === props.serverId) copyError.value = true;
  }
}

watch(form, resetConfigurationFeedback, { flush: 'sync' });
watch(() => props.serverId, () => {
  data.value = null;
  saving.value = false;
  applyingConfiguration.value = false;
  saveFailed.value = false;
  copyError.value = false;
  resetCopyFeedback();
  void load();
}, { immediate: true });
onBeforeUnmount(() => { disposed = true; generation++; resetCopyFeedback(); });
</script>

<style scoped>
.fastdl-tab,
.fastdl-form,
.fastdl-result {
  display: flex;
  flex-direction: column;
  min-width: 0;
}
.fastdl-tab { margin-top: .5rem; gap: 1rem; }
.fastdl-tab-with-save { padding-bottom: 5rem; }
.fastdl-form { gap: 1.25rem; }
.fastdl-result { gap: .75rem; }
.fastdl-loading { padding: 2rem 0; text-align: center; }
.fastdl-actions { display: flex; flex-wrap: wrap; align-items: center; gap: .5rem; }
.fastdl-field { width: 100%; min-width: 0; }
.fastdl-copyable { position: relative; min-width: 0; }
.fastdl-code {
  display: block;
  margin: .25rem 0;
  padding: .5rem 2.5rem .5rem .5rem;
  border-radius: .25rem;
  background: var(--gameap-stone-50, #fafaf9);
  color: var(--gameap-red-800, #991b1b);
  font-family: monospace;
  font-size: .875rem;
  line-height: 1.25rem;
  word-break: break-all;
  overflow-wrap: anywhere;
}
.fastdl-copy-button {
  position: absolute;
  right: .5rem;
  top: 50%;
  transform: translateY(-50%);
  padding: .25rem;
  border: 0;
  border-radius: .25rem;
  background: transparent;
  color: var(--gameap-stone-500, #78716c);
  cursor: pointer;
  transition: color .15s ease, background-color .15s ease;
}
.fastdl-copy-button:hover { background: var(--gameap-stone-200, #e7e5e4); }
.fastdl-copy-button:focus-visible { outline: 2px solid var(--gameap-primary, #84cc16); outline-offset: 2px; }
.fastdl-copied-icon { color: var(--gameap-success, #84cc16); }
.dark .fastdl-code {
  background: var(--gameap-stone-600, #57534e);
  color: var(--gameap-red-300, #fca5a5);
}
.dark .fastdl-copy-button { color: var(--gameap-stone-300, #d6d3d1); }
.dark .fastdl-copy-button:hover { background: var(--gameap-stone-500, #78716c); }
.fastdl-copy-icon-enter-active,
.fastdl-copy-icon-leave-active { transition: opacity .15s ease, transform .15s ease; }
.fastdl-copy-icon-enter-from { opacity: 0; transform: scale(.8); }
.fastdl-copy-icon-leave-to { opacity: 0; transform: scale(.8); }
.fastdl-save-bar {
  position: fixed;
  bottom: 0;
  left: 0;
  right: 0;
  z-index: 30;
  display: flex;
  justify-content: flex-end;
  padding: .75rem 1rem;
  border-top: 1px solid var(--gameap-border, #e7e5e4);
  background: var(--gameap-surface-raised, #fff);
}
.fastdl-hint {
  margin: .25rem 0 0;
  font-size: .75rem;
  line-height: 1rem;
  color: var(--gameap-text-muted, #78716c);
}
.fastdl-option-hint { margin-left: 1.5rem; }
.fastdl-configuration { white-space: pre-wrap; word-break: normal; }
</style>
