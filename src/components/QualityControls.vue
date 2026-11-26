<script setup lang="ts">
import { computed, onBeforeUnmount } from "vue";
import { NSelect, NSlider, NSwitch } from "naive-ui";
import { useI18n } from "vue-i18n";
import {
  applyShareParams,
  clampFps,
  clampQuality,
  persistSettings,
  settings,
} from "@/stores/settings";

/**
 * Shared audio & quality controls (video quality slider, target fps select,
 * audio toggle, auto-start toggle) used on both the share page and the
 * dedicated quality page.
 */
const { t } = useI18n();

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

function onAutoStart(value: boolean): void {
  settings.autoStart = value;
  void persistSettings();
}

onBeforeUnmount(() => {
  if (applyTimer !== null) clearTimeout(applyTimer);
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
      <label class="toggle-item">
        <NSwitch
          size="small"
          :value="settings.autoStart"
          @update:value="onAutoStart"
        />
        <span>{{ t("quality.autoStart") }}</span>
      </label>
    </div>
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
</style>
