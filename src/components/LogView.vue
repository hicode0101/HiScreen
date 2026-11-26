<script setup lang="ts">
import { computed } from "vue";
import { NButton } from "naive-ui";
import { useI18n } from "vue-i18n";
import { api, isTauri } from "@/lib/tauri";
import { share } from "@/stores/share";

const { t } = useI18n();

const logs = computed(() => [...share.logs].reverse());

async function clearLogs(): Promise<void> {
  if (isTauri()) {
    try {
      await api.clearLogs();
      share.logs = [];
    } catch {
      // Outside Tauri the in-memory list is cleared below anyway.
    }
  } else {
    share.logs = [];
  }
}
</script>

<template>
  <div class="log-view">
    <div class="head-row">
      <h2 class="view-title">{{ t("log.title") }}</h2>
      <NButton size="small" @click="void clearLogs()">
        {{ t("log.clear") }}
      </NButton>
    </div>

    <section class="card log-card">
      <ul class="log-list">
        <li v-if="logs.length === 0" class="log-empty">{{ t("log.empty") }}</li>
        <li v-for="(log, i) in logs" :key="`${log.ts}-${i}`" class="log-item">
          <span class="log-dot" :class="log.level"></span>
          <span class="log-ts mono">{{ log.ts }}</span>
          <span class="log-msg">{{ log.msg }}</span>
        </li>
      </ul>
    </section>
  </div>
</template>

<style scoped>
.log-view {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 18px 20px;
}

.head-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.log-card {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 10px 14px;
}

.log-list {
  margin: 0;
  padding: 0;
  list-style: none;
  display: flex;
  flex-direction: column;
}

.log-item {
  display: flex;
  align-items: baseline;
  gap: 10px;
  padding: 7px 2px;
  border-bottom: 1px solid var(--stroke);
  font-size: 12.5px;
  line-height: 1.55;
}

.log-item:last-child {
  border-bottom: none;
}

.log-empty {
  padding: 24px 0;
  text-align: center;
  color: var(--text-tertiary);
}

.log-dot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  flex-shrink: 0;
  background: var(--text-tertiary);
  transform: translateY(-1px);
}

.log-dot.success {
  background: var(--success);
}

.log-dot.info {
  background: var(--accent);
}

.log-dot.warn {
  background: var(--warning);
}

.log-dot.error {
  background: var(--danger);
}

.log-ts {
  flex-shrink: 0;
  color: var(--text-tertiary);
  font-variant-numeric: tabular-nums;
}

.log-msg {
  color: var(--text-primary);
  word-break: break-all;
  user-select: text;
}

.mono {
  font-family: "Cascadia Code", Consolas, monospace;
}
</style>
