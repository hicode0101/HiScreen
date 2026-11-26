<script setup lang="ts">
import { computed, ref } from "vue";
import { NButton, NInputNumber } from "naive-ui";
import { useI18n } from "vue-i18n";
import { useMessage } from "naive-ui";
import { openExternal } from "@/lib/tauri";
import { clampPort, persistSettings, settings } from "@/stores/settings";
import { applyPort, share } from "@/stores/share";
import Icon from "./Icon.vue";

const { t } = useI18n();
const message = useMessage();

const portDraft = ref<number>(settings.port);
const saving = ref(false);

const addresses = computed(() =>
  share.status.running ? share.status.addresses : [],
);

function onPort(value: number | null): void {
  portDraft.value = clampPort(Number(value) || settings.port);
}

async function savePort(): Promise<void> {
  settings.port = clampPort(portDraft.value);
  portDraft.value = settings.port;
  saving.value = true;
  try {
    await persistSettings();
    await applyPort();
    message.success(t("network.portApplied"));
  } catch {
    message.error(t("network.portFailed"));
  } finally {
    saving.value = false;
  }
}

function copyUrl(): void {
  const url = share.status.url;
  if (!url) return;
  void navigator.clipboard
    .writeText(url)
    .then(() => message.success(t("share.copied")))
    .catch(() => message.error(t("share.copyFailed")));
}

function openInBrowser(): void {
  if (share.status.url) void openExternal(share.status.url);
}
</script>

<template>
  <div class="network-view">
    <h2 class="view-title">{{ t("network.title") }}</h2>

    <section class="card">
      <h3 class="section-title">{{ t("network.service") }}</h3>
      <div class="status-row">
        <span class="dot" :class="{ on: share.status.running }"></span>
        <span>{{ share.status.running ? t("network.running") : t("network.stopped") }}</span>
        <template v-if="share.status.running">
          <span class="mono addr">{{ share.status.url }}</span>
          <button class="icon-btn" :title="t('share.copy')" @click="copyUrl">
            <Icon name="copy" :size="14" />
          </button>
          <button class="icon-btn" :title="t('share.open')" @click="openInBrowser">
            <Icon name="globe" :size="14" />
          </button>
        </template>
      </div>
      <ul class="addr-list" v-if="addresses.length > 1">
        <li v-for="a in addresses.slice(1)" :key="a" class="mono">{{ a }}</li>
      </ul>
    </section>

    <section class="card">
      <h3 class="section-title">{{ t("network.portTitle") }}</h3>
      <div class="port-row">
        <NInputNumber
          class="port-input"
          size="small"
          :value="portDraft"
          :min="1024"
          :max="65535"
          :step="1"
          :show-button="false"
          @update:value="onPort"
        />
        <NButton size="small" type="primary" :loading="saving" @click="void savePort()">
          {{ t("network.apply") }}
        </NButton>
      </div>
      <p class="hint">{{ t("network.portHint") }}</p>
    </section>
  </div>
</template>

<style scoped>
.network-view {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 18px 20px;
  overflow-y: auto;
  max-width: 720px;
}

.section-title {
  margin: 0 0 12px;
  font-size: 15px;
  font-weight: 650;
}

.status-row {
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: 13.5px;
}

.dot {
  width: 9px;
  height: 9px;
  border-radius: 50%;
  background: var(--text-tertiary);
}

.dot.on {
  background: var(--success);
  box-shadow: 0 0 6px rgba(15, 123, 15, 0.5);
}

.mono {
  font-variant-numeric: tabular-nums;
  font-family: "Cascadia Code", Consolas, monospace;
}

.addr {
  color: var(--accent);
}

.icon-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  flex-shrink: 0;
  border: 1px solid var(--stroke);
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
}

.icon-btn:hover {
  background: var(--bg-hover);
  color: var(--text-primary);
}

.addr-list {
  margin: 10px 0 0;
  padding: 0 0 0 19px;
  list-style: none;
  font-size: 12.5px;
  color: var(--text-secondary);
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.port-row {
  display: flex;
  align-items: center;
  gap: 10px;
}

.port-input {
  width: 140px;
}

.hint {
  margin: 12px 0 0;
  font-size: 12.5px;
  color: var(--text-tertiary);
  line-height: 1.6;
  white-space: pre-line;
}
</style>
