<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { share } from "@/stores/share";
import { settings } from "@/stores/settings";
import QualityControls from "./QualityControls.vue";
import Icon from "./Icon.vue";

const { t } = useI18n();

const audioStatus = computed(() => {
  if (!share.status.audioOn) return t("quality.audioOff");
  return settings.audio ? t("quality.audioConnected") : t("quality.audioOff");
});

const fpsStatus = computed(() => {
  const live = share.status.sharing;
  return live
    ? t("quality.fpsLive", { fps: share.status.fpsTarget })
    : t("quality.fpsIdle");
});
</script>

<template>
  <div class="quality-view">
    <h2 class="view-title">{{ t("quality.pageTitle") }}</h2>

    <section class="card">
      <h3 class="section-title">{{ t("quality.videoSection") }}</h3>
      <QualityControls />
    </section>

    <section class="card">
      <h3 class="section-title">{{ t("quality.deviceSection") }}</h3>
      <div class="status-list">
        <div class="status-item">
          <Icon name="speaker" :size="15" />
          <span>{{ t("quality.audioStatus") }}</span>
          <span class="value">{{ audioStatus }}</span>
        </div>
        <div class="status-item">
          <Icon name="share" :size="15" />
          <span>{{ t("quality.captureStatus") }}</span>
          <span class="value">{{ fpsStatus }}</span>
        </div>
      </div>
    </section>
  </div>
</template>

<style scoped>
.quality-view {
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
  margin: 0 0 14px;
  font-size: 15px;
  font-weight: 650;
}

.status-list {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.status-item {
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: 13.5px;
  color: var(--text-secondary);
}

.status-item .value {
  color: var(--text-primary);
}
</style>
