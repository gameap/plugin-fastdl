<template>
  <div class="mt-2 space-y-4">
    <div class="flex flex-wrap items-center gap-3">
      <GButton type="button" color="white" :loading="loading" :disabled="saving || dirty" @click="load"><GIcon name="refresh" class="mr-1" />{{ trans('refresh') }}</GButton>
      <GStatusBadge v-if="data" :color="!applied ? 'orange' : data.enabled ? 'green' : 'stone'" :text="trans(!applied ? 'not_applied' : data.enabled ? 'active' : 'disabled')" />
    </div>
    <NAlert v-if="error" type="error">{{ error }}</NAlert>
    <div v-if="loading && !data" class="py-8 text-center"><NSpin /></div>
    <template v-if="data">
      <NAlert v-if="!data.supported" type="warning">{{ trans('unsupported') }}</NAlert>
      <NAlert v-if="!data.can_manage" type="info">{{ trans('read_only') }}</NAlert>
      <NAlert v-for="warning in warnings" :key="warning" type="warning">{{ warning }}</NAlert>
      <NCard size="small">
        <form class="space-y-5" @submit.prevent="save">
          <NCheckbox v-model:checked="form.enabled" :disabled="locked || (!data.node_ready && !form.enabled)">{{ trans('enabled') }}</NCheckbox>
          <div class="fastdl-fields">
            <NFormItem :label="trans('engine')" :show-feedback="false">
              <NSelect v-model:value="form.engine" :options="engines" :disabled="locked" />
            </NFormItem>
            <NFormItem :label="trans('game_dir')" :show-feedback="false">
              <div class="w-full">
                <NInput v-model:value="form.game_dir" :disabled="locked" placeholder="cstrike" />
                <p class="mt-1 text-sm text-stone-500">{{ trans('game_dir_hint') }}</p>
              </div>
            </NFormItem>
          </div>
          <div>
            <NCheckbox v-model:checked="form.autoindex" :disabled="locked">{{ trans('autoindex') }}</NCheckbox>
            <p class="mt-1 text-sm text-stone-500">{{ trans('autoindex_hint') }}</p>
          </div>
          <div v-if="form.engine === 'source'">
            <NCheckbox v-model:checked="form.generate_bz2" :disabled="locked">{{ trans('generate_bz2') }}</NCheckbox>
            <p class="mt-1 text-sm text-stone-500">{{ trans('generate_bz2_hint') }}</p>
          </div>
          <div>
            <NCheckbox v-model:checked="form.manage_game_config" :disabled="locked">{{ trans('manage_game_config') }}</NCheckbox>
            <p class="mt-1 text-sm text-stone-500">{{ trans('manage_game_config_hint') }}</p>
          </div>
          <NAlert type="info">{{ trans('security') }}</NAlert>
          <NAlert v-if="dirty" type="info">{{ trans('unsaved') }}</NAlert>
          <GButton type="button" v-if="data.can_manage" color="black" :loading="saving" :disabled="locked || !pendingApply || (form.enabled && !data.node_ready)" @click="save"><GIcon name="save" class="mr-1" />{{ trans('save') }}</GButton>
        </form>
      </NCard>
      <NCard v-if="applied && data.download_url && data.enabled" :title="trans('download_url')" size="small">
        <div class="flex flex-wrap items-start gap-2">
          <NInput :value="data.download_url" readonly :input-props="{ 'aria-label': trans('download_url') }" style="flex: 1; min-width: min(100%, 260px)" />
          <GButton type="button" color="black" @click="copy(data.download_url)"><GIcon name="copy" class="mr-1" />{{ trans('copy') }}</GButton>
        </div>
      </NCard>
      <NCard v-if="applied && data.enabled && data.configuration.length" :title="trans('game_configuration')" size="small">
        <div class="space-y-3">
          <NAlert type="info">{{ trans(data.manage_game_config ? 'config_apply_hint' : 'manual_config_hint') }}</NAlert>
          <pre class="fastdl-configuration" tabindex="0">{{ data.configuration.join('\n') }}</pre>
          <GButton type="button" color="black" @click="copy(data.configuration.join('\n'))"><GIcon name="copy" class="mr-1" />{{ trans('copy') }}</GButton>
        </div>
      </NCard>
      <NAlert v-if="copyError" type="warning">{{ trans('copy_failed') }}</NAlert>
    </template>
  </div>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, reactive, ref, watch } from 'vue';
import { NAlert, NCard, NCheckbox, NFormItem, NInput, NSelect, NSpin } from 'naive-ui';
import { providePluginTrans } from '@gameap/plugin-sdk';
import { errorMessage, fastdlApi, type ServerFastDL, type ServerSettings } from '../api';
import { editableSettings, isGameDirectory, needsApply, serverWarnings, settingsChanged } from '../lib/settings';

const props = defineProps<{ serverId: number; pluginId: string }>();
const { trans } = providePluginTrans(props.pluginId);
const data = ref<ServerFastDL | null>(null);
const form = reactive<ServerSettings>({ enabled: false, autoindex: false, engine: 'goldsource', game_dir: '', manage_game_config: true, generate_bz2: true });
const engines = [{ label: 'GoldSource', value: 'goldsource' }, { label: 'Source', value: 'source' }];
const loading = ref(false);
const saving = ref(false);
const error = ref('');
const copyError = ref(false);
const saveFailed = ref(false);
const applied = computed(() => data.value?.synced === true && !saveFailed.value);
const dirty = computed(() => data.value !== null && settingsChanged(form, data.value));
const pendingApply = computed(() => data.value !== null && (saveFailed.value || needsApply(form, data.value)));
const warnings = computed(() => data.value ? serverWarnings(data.value, trans) : []);
const locked = computed(() => saving.value || loading.value || !data.value?.can_manage);
let generation = 0;
let disposed = false;

async function load() {
  const request = ++generation;
  const serverId = props.serverId;
  loading.value = true;
  error.value = '';
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

async function copy(value: string) {
  copyError.value = false;
  try {
    await navigator.clipboard.writeText(value);
    window.$message?.success(trans('copied'));
  } catch {
    copyError.value = true;
  }
}

watch(() => props.serverId, () => {
  data.value = null;
  saving.value = false;
  saveFailed.value = false;
  copyError.value = false;
  void load();
}, { immediate: true });
onBeforeUnmount(() => { disposed = true; generation++; });
</script>

<style scoped>
.fastdl-fields { display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 2fr); gap: 1rem; }
.fastdl-configuration { margin: 0; overflow-x: auto; padding: .75rem; border-radius: .375rem; background: rgba(127, 127, 127, .1); white-space: pre; }
@media (max-width: 640px) { .fastdl-fields { grid-template-columns: minmax(0, 1fr); } }
</style>
