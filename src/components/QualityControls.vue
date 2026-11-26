<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { NSelect, NSlider, NSwitch, useMessage } from "naive-ui";
import { useI18n } from "vue-i18n";
import { api, isTauri } from "@/lib/tauri";
import {
  applyMicParams,
  applyShareParams,
  clampFps,
  clampMicVolume,
  clampQuality,
  persistSettings,
  settings,
} from "@/stores/settings";
import Icon from "./Icon.vue";

/**
 * Shared audio & quality controls (video quality slider, target fps select,
 * audio toggle, mic enable/device/volume/level) used by the quality page.
 */
const { t } = useI18n();
const message = useMessage();

const fpsOptions = computed(() =>
  [10, 15, 20, 24, 30, 60].map((v) => ({ label: `${v} FPS`, value: v })),
);

let applyTimer: number | null = null;

function scheduleApply(): void {
  if (applyTimer !== null) clearTimeout(applyTimer);
  applyTimer = window.setTimeout(() => {
    applyTimer = null;
    void applyShareParams();
  }, 300);
}

function onQuality(value: number): void {
  settings.quality = clampQuality(value);
  void persistSettings();
  scheduleApply();
}

function onFps(value: number): void {
  settings.fps = clampFps(value);
  void persistSettings();
  void applyShareParams();
}

function onAudio(value: boolean): void {
  settings.audio = value;
  void persistSettings();
  void applyShareParams();
}

function onShowCursor(value: boolean): void {
  settings.showCursor = value;
  void persistSettings();
  void applyShareParams();
}

// ---------------------------------------------------------------- microphone

const micDevices = ref<string[]>([]);
const micLoading = ref(false);
/** Latest level from the backend, in dBFS × 10 (-900..=0). */
const micLevelDb10 = ref(-900);
let micLoaded = false;
let micPollTimer: number | null = null;
let micApplyTimer: number | null = null;

const micOptions = computed(() =>
  micDevices.value.map((name) => ({ label: name, value: name })),
);

/** Meter fill percent: -60 dB..=0 dB maps to 0..100. */
const micLevelPct = computed(() => {
  const db = micLevelDb10.value / 10;
  return Math.min(100, Math.max(0, Math.round((db + 60) * (100 / 60))));
});
const micLevelText = computed(() => `${Math.round(micLevelDb10.value / 10)} dB`);

async function loadMics(verbose: boolean): Promise<void> {
  if (!isTauri()) {
    if (verbose) message.info(t("share.browserHint"));
    return;
  }
  micLoading.value = true;
  try {
    micDevices.value = await api.listMics();
    // A vanished device can't be reopened; fall back to the default one.
    if (settings.micDevice && !micDevices.value.includes(settings.micDevice)) {
      settings.micDevice = "";
      void persistSettings();
      void applyMicParams();
    }
    if (verbose) {
      if (micDevices.value.length === 0) {
        message.warning(t("quality.micNone"));
      } else {
        message.success(t("quality.micsRefreshed"));
      }
    }
  } catch {
    if (verbose) message.error(t("quality.micRefreshFailed"));
  } finally {
    micLoading.value = false;
  }
}

function startMicPoll(): void {
  if (micPollTimer !== null || !isTauri()) return;
  micPollTimer = window.setInterval(() => {
    void api
      .getMicLevel()
      .then((v) => {
        micLevelDb10.value = v;
      })
      .catch(() => {});
  }, 150);
}

function stopMicPoll(): void {
  if (micPollTimer !== null) {
    clearInterval(micPollTimer);
    micPollTimer = null;
  }
  micLevelDb10.value = -900;
}

function scheduleMicApply(): void {
  if (micApplyTimer !== null) clearTimeout(micApplyTimer);
  micApplyTimer = window.setTimeout(() => {
    micApplyTimer = null;
    void applyMicParams();
  }, 300);
}

function onMicEnabled(value: boolean): void {
  // Session-only state: the mic always starts OFF each launch, so the
  // on/off choice is never persisted — only device/volume are.
  settings.micEnabled = value;
  if (value && !micLoaded) {
    micLoaded = true;
    void loadMics(false);
  }
  if (value) startMicPoll();
  else stopMicPoll();
  void applyMicParams();
}

function onMicDevice(value: string): void {
  settings.micDevice = value;
  void persistSettings();
  void applyMicParams();
}

function onMicVolume(value: number): void {
  settings.micVolume = clampMicVolume(value);
  void persistSettings();
  scheduleMicApply();
}

// The cursor overlay is implemented for Windows captures only.
const cursorSupported = navigator.userAgent.includes("Windows");

onMounted(() => {
  if (settings.micEnabled) {
    micLoaded = true;
    void loadMics(false);
    startMicPoll();
  }
});

onBeforeUnmount(() => {
  if (applyTimer !== null) clearTimeout(applyTimer);
  if (micApplyTimer !== null) clearTimeout(micApplyTimer);
  stopMicPoll();
});
</script>

<template>
  <div class="quality-controls">
    <div class="row">
      <span class="field-label">{{ t("quality.videoQuality") }}</span>
      <NSlider
        class="slider"
        :value="settings.quality"
        :min="10"
        :max="100"
        :step="1"
        :format-tooltip="(v: number) => `${v}%`"
        @update:value="onQuality"
      />
      <span class="pct">{{ settings.quality }}%</span>
    </div>

    <div class="row">
      <span class="field-label">{{ t("quality.targetFps") }}</span>
      <NSelect
        class="fps-select"
        size="small"
        :value="settings.fps"
        :options="fpsOptions"
        @update:value="onFps"
      />
    </div>

    <div class="row toggles">
      <label class="toggle-item">
        <NSwitch
          size="small"
          :value="settings.audio"
          @update:value="onAudio"
        />
        <span>{{ t("quality.shareAudio") }}</span>
      </label>
      <label v-if="cursorSupported" class="toggle-item">
        <NSwitch
          size="small"
          :value="settings.showCursor"
          @update:value="onShowCursor"
        />
        <span>{{ t("quality.showCursor") }}</span>
      </label>
      <label class="toggle-item">
        <NSwitch
          size="small"
          :value="settings.micEnabled"
          @update:value="onMicEnabled"
        />
        <span>{{ t("quality.mic") }}</span>
      </label>
    </div>

    <template v-if="settings.micEnabled">
      <div class="row">
        <span class="field-label">{{ t("quality.micDevice") }}</span>
        <NSelect
          class="mic-select"
          size="small"
          :value="settings.micDevice"
          :options="micOptions"
          :loading="micLoading"
          :placeholder="t('quality.micDevicePlaceholder')"
          @update:value="onMicDevice"
        />
        <button class="icon-btn" :title="t('share.refreshSources')" @click="void loadMics(true)">
          <Icon name="refresh" :size="14" />
        </button>
      </div>

      <div class="row">
        <span class="field-label">{{ t("quality.micVolume") }}</span>
        <NSlider
          class="slider"
          :value="settings.micVolume"
          :min="0"
          :max="200"
          :step="1"
          :format-tooltip="(v: number) => `${v}%`"
          @update:value="onMicVolume"
        />
        <span class="pct">{{ settings.micVolume }}%</span>
      </div>

      <div class="row">
        <span class="field-label">{{ t("quality.micLevel") }}</span>
        <div class="level-track">
          <div class="level-fill" :style="{ width: `${micLevelPct}%` }" />
        </div>
        <span class="pct level-db">{{ micLevelText }}</span>
      </div>
    </template>
  </div>
</template>

<style scoped>
.quality-controls {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.row {
  display: flex;
  align-items: center;
  gap: 14px;
}

.field-label {
  width: 72px;
  flex-shrink: 0;
  font-size: 13px;
  color: var(--text-secondary);
}

.slider {
  flex: 1;
}

.pct {
  width: 44px;
  text-align: right;
  font-variant-numeric: tabular-nums;
  color: var(--text-primary);
}

.fps-select {
  width: 140px;
}

.mic-select {
  width: 240px;
}

.row.toggles {
  gap: 26px;
}

.toggle-item {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 13px;
  cursor: pointer;
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

.level-track {
  flex: 1;
  height: 10px;
  border-radius: 5px;
  background: var(--bg-hover);
  overflow: hidden;
}

.level-fill {
  height: 100%;
  border-radius: 5px;
  /* green -> yellow -> red; clipping by width keeps quiet levels green */
  background: linear-gradient(90deg, #18a058 0%, #63c231 40%, #f0a020 70%, #d03050 100%);
  transition: width 0.12s linear;
}

.level-db {
  width: 56px;
  color: var(--text-secondary);
}
</style>
