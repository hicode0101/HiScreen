<script setup lang="ts">
import { useI18n } from "vue-i18n";
import Icon from "./Icon.vue";
import { ui, switchView, type ViewId } from "@/stores/ui";

const { t } = useI18n();

const items: { id: ViewId; icon: string; key: string }[] = [
  { id: "share", icon: "share", key: "nav.share" },
  { id: "audience", icon: "users", key: "nav.audience" },
  { id: "network", icon: "lan", key: "nav.network" },
  { id: "quality", icon: "speaker", key: "nav.quality" },
  { id: "log", icon: "log", key: "nav.log" },
  { id: "about", icon: "info", key: "nav.about" },
];
</script>

<template>
  <nav class="sidebar">
    <button
      v-for="item in items"
      :key="item.id"
      class="nav-item"
      :class="{ active: ui.view === item.id }"
      @click="switchView(item.id)"
    >
      <Icon :name="item.icon" :size="18" />
      <span>{{ t(item.key) }}</span>
    </button>
  </nav>
</template>

<style scoped>
.sidebar {
  width: 172px;
  flex-shrink: 0;
  padding: 10px 8px;
  display: flex;
  flex-direction: column;
  gap: 4px;
  border-right: 1px solid var(--stroke);
  background: var(--bg-subtle);
}

.nav-item {
  position: relative;
  display: flex;
  align-items: center;
  gap: 11px;
  padding: 10px 12px;
  border: none;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-primary);
  font-family: inherit;
  font-size: 13.5px;
  text-align: left;
  cursor: pointer;
  transition: background 0.15s ease;
}

.nav-item:hover {
  background: var(--bg-hover);
}

.nav-item.active {
  background: var(--accent-subtle-bg);
  color: var(--accent);
  font-weight: 600;
}

.nav-item.active::before {
  content: "";
  position: absolute;
  left: 0;
  top: 8px;
  bottom: 8px;
  width: 3px;
  border-radius: 2px;
  background: var(--accent);
}

.nav-item span {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
</style>
