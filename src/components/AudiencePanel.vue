<script setup lang="ts">
import { computed, ref } from "vue";
import { NButton, NSelect, NTooltip, useMessage } from "naive-ui";
import { useI18n } from "vue-i18n";
import { api, isTauri } from "@/lib/tauri";
import { share } from "@/stores/share";
import Icon from "./Icon.vue";

/**
 * Viewer table + broadcast input. `compact` renders without the browser and
 * kick columns (used on the share page); the dedicated audience page has them.
 */
const props = defineProps<{ compact?: boolean }>();

const { t } = useI18n();
const message = useMessage();

const notice = ref("");
const noticeDuration = ref(10);
const viewers = computed(() => share.status.viewers ?? []);

const durationOptions = [
  { label: () => t("audience.durSec", { n: 5 }), value: 5 },
  { label: () => t("audience.durSec", { n: 10 }), value: 10 },
  { label: () => t("audience.durMin", { n: 1 }), value: 60 },
  { label: () => t("audience.durMin", { n: 10 }), value: 600 },
  { label: () => t("audience.durMin", { n: 30 }), value: 1800 },
  { label: () => t("audience.durMin", { n: 60 }), value: 3600 },
];

function formatFor(seconds: number): string {
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  const s = Math.floor(seconds % 60);
  if (h > 0) return `${h}h ${m}m`;
  if (m > 0) return `${m}m ${s}s`;
  return `${s}s`;
}

async function sendNotice(): Promise<void> {
  const text = notice.value.trim();
  if (!text) return;
  if (!isTauri()) {
    message.info(t("audience.browserHint"));
    return;
  }
  try {
    await api.broadcastNotice(text, noticeDuration.value);
    message.success(t("audience.noticeSent"));
    notice.value = "";
  } catch {
    message.error(t("audience.noticeFailed"));
  }
}

async function kick(id: number): Promise<void> {
  if (!isTauri()) return;
  try {
    await api.kickViewer(id);
    message.success(t("audience.kicked"));
  } catch {
    message.error(t("audience.kickFailed"));
  }
}
</script>

<template>
  <div class="audience-panel" :class="{ compact: props.compact }">
    <div class="panel-head">
      <h3 class="panel-title">{{ t("audience.title") }}</h3>
      <span class="count">{{ viewers.length }}</span>
    </div>

    <div class="table-wrap">
      <table class="aud-table">
        <thead>
          <tr>
            <th>{{ t("audience.ip") }}</th>
            <th>{{ t("audience.device") }}</th>
            <th v-if="!props.compact">{{ t("audience.browser") }}</th>
            <th>{{ t("audience.latency") }}</th>
            <th>{{ t("audience.connected") }}</th>
            <th>{{ t("audience.audio") }}</th>
            <th v-if="!props.compact" class="op-col"></th>
          </tr>
        </thead>
        <tbody>
          <tr v-if="viewers.length === 0">
            <td :colspan="props.compact ? 5 : 7" class="empty">
              {{ t("audience.empty") }}
            </td>
          </tr>
          <tr v-for="v in viewers" :key="v.id">
            <td class="mono">{{ v.addr || "—" }}</td>
            <td>
              <div class="device-cell">
                <Icon name="display" :size="14" />
                <span>{{ v.os || t("audience.unknown") }}</span>
              </div>
            </td>
            <td v-if="!props.compact">{{ v.browser || "—" }}</td>
            <td class="mono">{{ v.latencyMs }} ms</td>
            <td class="mono">{{ formatFor(v.connectedForS) }}</td>
            <td>
              <span class="audio-pill" :class="{ on: v.audioOn }">
                {{ v.audioOn ? t("audience.audioOn") : t("audience.audioOff") }}
              </span>
            </td>
            <td v-if="!props.compact" class="op-col">
              <button
                class="kick-btn"
                :title="t('audience.kick')"
                @click="void kick(v.id)"
              >
                <Icon name="dismiss" :size="14" />
              </button>
            </td>
          </tr>
        </tbody>
      </table>
    </div>

    <div class="notice-row">
      <input
        v-model="notice"
        class="notice-input"
        type="text"
        :placeholder="t('audience.noticePlaceholder')"
        @keydown.enter="void sendNotice()"
      />
      <NTooltip trigger="hover" placement="top">
        <template #trigger>
          <NSelect
            class="notice-duration"
            size="small"
            :value="noticeDuration"
            :options="durationOptions"
            @update:value="(v: number) => (noticeDuration = v)"
          />
        </template>
        {{ t("audience.durationTip") }}
      </NTooltip>
      <NButton type="primary" size="small" @click="void sendNotice()">
        {{ t("audience.send") }}
      </NButton>
    </div>
  </div>
</template>

<style scoped>
.audience-panel {
  display: flex;
  flex-direction: column;
  gap: 12px;
  height: 100%;
}

.panel-head {
  display: flex;
  align-items: center;
  gap: 8px;
}

.panel-title {
  margin: 0;
  font-size: 15px;
  font-weight: 650;
}

.count {
  min-width: 22px;
  height: 20px;
  padding: 0 6px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: 10px;
  background: var(--accent-subtle-bg);
  color: var(--accent);
  font-size: 12px;
  font-weight: 600;
}

.table-wrap {
  flex: 1;
  min-height: 0;
  overflow: auto;
  border: 1px solid var(--stroke);
  border-radius: var(--radius-sm);
}

.aud-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 12.5px;
}

.aud-table th {
  position: sticky;
  top: 0;
  padding: 8px 10px;
  text-align: left;
  font-weight: 600;
  color: var(--text-secondary);
  background: var(--bg-subtle);
  border-bottom: 1px solid var(--stroke);
  white-space: nowrap;
}

.aud-table td {
  padding: 9px 10px;
  border-bottom: 1px solid var(--stroke);
  white-space: nowrap;
}

.aud-table tbody tr:last-child td {
  border-bottom: none;
}

.aud-table tbody tr:hover {
  background: var(--bg-hover);
}

.mono {
  font-variant-numeric: tabular-nums;
}

/* The compact panel is ~330px wide; tighten cells so the columns fit. */
.compact .aud-table th,
.compact .aud-table td {
  padding-inline: 6px;
}

.empty {
  text-align: center;
  color: var(--text-tertiary);
  padding: 26px 10px !important;
}

.device-cell {
  display: flex;
  align-items: center;
  gap: 6px;
  color: var(--text-primary);
}

.audio-pill {
  display: inline-flex;
  padding: 2px 8px;
  border-radius: 999px;
  font-size: 11.5px;
  background: var(--bg-hover);
  color: var(--text-secondary);
}

.audio-pill.on {
  background: rgba(15, 123, 15, 0.1);
  color: var(--success);
}

[data-theme="dark"] .audio-pill.on {
  background: rgba(108, 203, 95, 0.14);
}

.op-col {
  width: 34px;
  text-align: center;
}

.kick-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  border: none;
  border-radius: 4px;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
}

.kick-btn:hover {
  background: rgba(196, 43, 28, 0.12);
  color: var(--danger);
}

.notice-row {
  display: flex;
  gap: 8px;
}

.notice-input {
  flex: 1;
  min-width: 0;
  height: 32px;
  padding: 0 10px;
  border: 1px solid var(--stroke-control);
  border-radius: var(--radius-sm);
  background: var(--bg-card);
  color: var(--text-primary);
  font-family: inherit;
  font-size: 13px;
  outline: none;
}

.notice-input:focus {
  border-color: var(--accent);
}

.notice-input::placeholder {
  color: var(--text-tertiary);
}

.notice-duration {
  width: 108px;
  flex-shrink: 0;
}
</style>
