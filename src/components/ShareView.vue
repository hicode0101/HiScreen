<script setup lang="ts">
import { computed, onMounted, onUnmounted, watch } from "vue";
import { NSelect, NTooltip } from "naive-ui";
import { useI18n } from "vue-i18n";
import { useMessage } from "naive-ui";
import {
  share,
  loadPreview,
  PREVIEW_INTERVAL_MS,
  refreshDisplays,
  selectDisplay,
  startPreviewLoop,
  stopPreviewLoop,
  startShare,
  stopShare,
} from "@/stores/share";
import { settings } from "@/stores/settings";
import { isTauri, openExternal } from "@/lib/tauri";
import AudiencePanel from "./AudiencePanel.vue";
import Icon from "./Icon.vue";

const { t } = useI18n();
const message = useMessage();

const selected = computed(() => share.displays[share.selected] ?? null);
const sharing = computed(() => share.status.sharing);
const preview = computed(() => share.previews[share.selected] ?? "");

const displayOptions = computed(() =>
  share.displays.map((d) => ({
    label: `${d.name} (${d.width}×${d.height}${d.primary ? " · " + t("share.primary") : ""})`,
    value: d.id,
  })),
);

const recentLogs = computed(() => share.logs.slice(-5).reverse());

onMounted(() => {
  void loadPreview(share.selected);
  startPreviewLoop();
});

// The view is conditionally rendered, so unmounting stops the preview loop
// together with the view's visibility.
onUnmounted(() => {
  stopPreviewLoop();
});

watch(
  () => share.selected,
  (id) => void loadPreview(id),
);

async function toggleShare(): Promise<void> {
  if (sharing.value) {
    await stopShare();
  } else {
    await startShare(share.selected);
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

async function refreshSources(): Promise<void> {
  if (!isTauri()) {
    message.info(t("share.browserHint"));
    return;
  }
  const ok = await refreshDisplays();
  if (ok) message.success(t("share.sourcesRefreshed"));
  else message.error(t("share.refreshFailed"));
  // The preview normally refreshes every 2 s; sync it right away so the
  // thumbnail matches the freshly enumerated display list.
  void loadPreview(share.selected, true);
}
</script>

<template>
  <div class="share-view">
    <h2 class="view-title">{{ t("share.title") }}</h2>

    <div class="columns">
      <div class="left-col">
        <!-- ------- share card ------- -->
        <section class="card share-card">
          <NTooltip trigger="hover" placement="top">
            <template #trigger>
              <div class="preview-box">
                <img v-if="preview" :src="preview" alt="display preview" />
                <div v-else class="preview-empty">
                  <Icon name="display" :size="44" />
                </div>
              </div>
            </template>
            {{ t("share.previewTip", { n: PREVIEW_INTERVAL_MS / 1000 }) }}
          </NTooltip>

          <div class="meta-row">
            <div class="fields">
              <div class="field">
                <span class="field-label">{{ t("share.source") }}</span>
                <NSelect
                  class="source-select"
                  size="small"
                  :value="share.selected"
                  :options="displayOptions"
                  @update:value="(v: number) => selectDisplay(v)"
                />
                <button class="icon-btn" :title="t('share.refreshSources')" @click="void refreshSources()">
                  <Icon name="refresh" :size="14" />
                </button>
              </div>
              <div class="field">
                <span class="field-label">{{ t("share.resolution") }}</span>
                <span class="value mono">
                  {{ selected ? `${selected.width}×${selected.height}` : "—" }}
                </span>
              </div>
              <div class="field">
                <span class="field-label">{{ t("share.fps") }}</span>
                <span class="value mono">{{ settings.fps }} FPS</span>
                <span class="badge" :class="{ live: sharing }">
                  {{ sharing ? t("share.live") : t("share.idle") }}
                </span>
              </div>
              <div class="field">
                <span class="field-label">{{ t("quality.videoQuality") }}</span>
                <span class="value mono">{{ settings.quality }}%</span>
              </div>
            </div>
            <div class="action-col">
              <button class="share-btn" :class="{ danger: sharing }" :disabled="share.busy" @click="void toggleShare()">
                <Icon :name="sharing ? 'stop' : 'play'" :size="16" />
                {{ sharing ? t("share.stop") : t("share.start") }}
              </button>
            </div>
          </div>

          <div class="net-row">
            <div class="net-info">
              <div class="field-label">{{ t("share.address") }}</div>
              <div class="url-row">
                <code class="url" :class="{ off: !share.status.running }">
                  {{ share.status.running ? share.status.url : t("share.serverOff") }}
                </code>
                <template v-if="share.status.running">
                  <button class="icon-btn" :title="t('share.copy')" @click="copyUrl">
                    <Icon name="copy" :size="14" />
                  </button>
                  <button class="icon-btn" :title="t('share.open')" @click="openInBrowser">
                    <Icon name="globe" :size="14" />
                  </button>
                </template>
              </div>
            </div>
            <div v-if="share.status.qr" class="qr-box" v-html="share.status.qr"></div>
          </div>
        </section>
      </div>

      <aside class="right-col">
        <section class="card audience-card">
          <AudiencePanel compact />
        </section>
        <section class="card events-card">
          <h3 class="section-title">{{ t("share.events") }}</h3>
          <ul class="event-list">
            <li v-if="recentLogs.length === 0" class="event-empty">
              {{ t("share.noEvents") }}
            </li>
            <li v-for="(log, i) in recentLogs" :key="`${log.ts}-${i}`" class="event-item">
              <span class="event-dot" :class="log.level"></span>
              <span class="event-msg">{{ log.msg }}</span>
            </li>
          </ul>
        </section>
      </aside>
    </div>
  </div>
</template>

<style scoped>
.share-view {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 18px 20px;
  overflow-y: auto;
}

.columns {
  flex: 1;
  min-height: 0;
  display: flex;
  gap: 14px;
  align-items: flex-start;
}

.left-col {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.right-col {
  width: 360px;
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.share-card {
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.preview-box {
  height: 280px;
  border-radius: var(--radius-sm);
  border: 1px solid var(--stroke);
  background: var(--bg-subtle);
  overflow: hidden;
  display: flex;
  align-items: center;
  justify-content: center;
}

.preview-box img {
  width: 100%;
  height: 100%;
  object-fit: contain;
}

.preview-empty {
  color: var(--text-tertiary);
}

.meta-row {
  display: flex;
  align-items: flex-end;
  justify-content: space-between;
  gap: 16px;
}

.fields {
  display: flex;
  flex-direction: column;
  gap: 10px;
  min-width: 0;
}

.field {
  display: flex;
  align-items: center;
  gap: 10px;
}

.source-select {
  width: 260px;
}

.value {
  font-size: 13px;
}

.mono {
  font-variant-numeric: tabular-nums;
}

.badge {
  padding: 2px 10px;
  border-radius: 999px;
  font-size: 12px;
  background: var(--bg-hover);
  color: var(--text-secondary);
}

.badge.live {
  background: rgba(15, 123, 15, 0.12);
  color: var(--success);
}

[data-theme="dark"] .badge.live {
  background: rgba(108, 203, 95, 0.16);
}

.share-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  min-width: 150px;
  height: 40px;
  padding: 0 22px;
  border: none;
  border-radius: var(--radius-sm);
  background: var(--success);
  color: #fff;
  font-family: inherit;
  font-size: 14.5px;
  font-weight: 600;
  cursor: pointer;
  transition: filter 0.15s ease;
}

.share-btn:hover:not(:disabled) {
  filter: brightness(1.12);
}

.share-btn:disabled {
  opacity: 0.55;
  cursor: default;
}

.share-btn.danger {
  background: var(--danger);
}

.net-row {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 16px;
  border-top: 1px solid var(--stroke);
  padding-top: 14px;
}

.net-info {
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-width: 0;
}

.url-row {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}

.url {
  font-size: 13.5px;
  color: var(--accent);
  font-family: "Cascadia Code", Consolas, monospace;
  word-break: break-all;
}

.url.off {
  color: var(--text-tertiary);
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

.qr-box {
  width: 108px;
  height: 108px;
  flex-shrink: 0;
  padding: 6px;
  background: #fff;
  border-radius: var(--radius-sm);
}

.qr-box :deep(svg) {
  width: 100%;
  height: 100%;
  display: block;
}

.section-title {
  margin: 0 0 14px;
  font-size: 15px;
  font-weight: 650;
}

.audience-card {
  height: 400px;
}

.events-card {
  display: flex;
  flex-direction: column;
}

.event-list {
  margin: 0;
  padding: 0;
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.event-item {
  display: flex;
  align-items: baseline;
  gap: 8px;
  font-size: 12.5px;
  color: var(--text-secondary);
  line-height: 1.5;
}

.event-empty {
  color: var(--text-tertiary);
  font-size: 12.5px;
}

.event-dot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  flex-shrink: 0;
  background: var(--text-tertiary);
  transform: translateY(-1px);
}

.event-dot.success {
  background: var(--success);
}

.event-dot.info {
  background: var(--accent);
}

.event-dot.warn {
  background: var(--warning);
}

.event-dot.error {
  background: var(--danger);
}
</style>
