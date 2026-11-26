<script setup lang="ts">
import { useI18n } from "vue-i18n";
import AudiencePanel from "./AudiencePanel.vue";

const { t } = useI18n();
</script>

<template>
  <div class="audience-view">
    <h2 class="view-title">{{ t("audience.pageTitle") }}</h2>
    <section class="card audience-full">
      <AudiencePanel />
    </section>
  </div>
</template>

<style scoped>
.audience-view {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 18px 20px;
}

.audience-full {
  flex: 1;
  min-height: 0;
}
</style>
