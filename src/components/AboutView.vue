<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { NSelect } from "naive-ui";
import { useI18n } from "vue-i18n";
import { api, openExternal } from "@/lib/tauri";
import { APP_AUTHOR, APP_HOMEPAGE } from "@/lib/appInfo";
import { settings, updateLanguage, updateTheme } from "@/stores/settings";
import type { LangOption } from "@/lib/i18n";
import type { ThemeOption } from "@/lib/theme";
import Icon from "./Icon.vue";

const { t } = useI18n();
const version = ref("");

onMounted(async () => {
  try {
    version.value = await api.appVersion();
  } catch {
    // Running outside Tauri (e.g. plain browser) — leave the version empty.
  }
});

const themeOptions = computed(() => [
  { label: t("settings.light"), value: "light" },
  { label: t("settings.dark"), value: "dark" },
]);

const languageOptions = computed(() => [
  { label: t("settings.followSystem"), value: "system" },
  { label: "简体中文", value: "zh-CN" },
  { label: "English", value: "en-US" },
]);

function onThemeChange(value: string | number): void {
  void updateTheme(String(value) as ThemeOption);
}

function onLanguageChange(value: string | number): void {
  void updateLanguage(String(value) as LangOption);
}

function openHomepage(): void {
  void openExternal(APP_HOMEPAGE);
}
</script>

<template>
  <div class="about-view">
    <h2 class="view-title">{{ t("about.title") }}</h2>

    <section class="card about-card">
      <div class="about-head">
        <svg class="about-logo" viewBox="0 0 24 24" width="64" height="64" aria-hidden="true">
          <rect x="1.5" y="1.5" width="21" height="21" rx="5" fill="var(--accent)" />
          <rect x="5" y="5.5" width="14" height="9.5" rx="1.6" stroke="var(--accent-text)" stroke-width="1.7" fill="none" stroke-linejoin="round" />
          <path d="M10.3 8.1 14.2 10.25 10.3 12.4z" fill="var(--accent-text)" stroke="var(--accent-text)" stroke-width="0.8" stroke-linejoin="round" />
          <path d="M9.5 18.6h5M12 15v3.6" stroke="var(--accent-text)" stroke-width="1.7" fill="none" stroke-linecap="round" />
        </svg>

        <div class="about-name">{{ t("app.name") }}</div>
        <div class="about-version">{{ version ? `v${version}` : "-" }}</div>
        <p class="about-desc">{{ t("about.desc") }}</p>

        <div class="about-meta">
          <div class="about-row">
            <span class="about-label">{{ t("about.homepage") }}</span>
            <a href="#" class="about-link" @click.prevent="openHomepage">{{ APP_HOMEPAGE }}</a>
          </div>
          <div class="about-row">
            <span class="about-label">{{ t("about.author") }}</span>
            <span>{{ APP_AUTHOR }}</span>
          </div>
        </div>
      </div>

      <!-- Preference rows: theme / language, same wiring as the scaffold settings dialog -->
      <div class="about-prefs">
        <div class="pref-row">
          <span class="row-icon"><Icon name="moon" :size="20" /></span>
          <div class="row-text">
            <div class="row-label">{{ t("settings.theme") }}</div>
            <div class="row-hint">{{ t("settings.themeHint") }}</div>
          </div>
          <NSelect
            class="row-select"
            :value="settings.theme"
            :options="themeOptions"
            :consistent-menu-width="false"
            @update:value="onThemeChange"
          />
        </div>

        <div class="pref-row">
          <span class="row-icon"><Icon name="globe" :size="20" /></span>
          <div class="row-text">
            <div class="row-label">{{ t("settings.language") }}</div>
            <div class="row-hint">{{ t("settings.languageHint") }}</div>
          </div>
          <NSelect
            class="row-select"
            :value="settings.language"
            :options="languageOptions"
            :consistent-menu-width="false"
            @update:value="onLanguageChange"
          />
        </div>
      </div>
    </section>
  </div>
</template>

<style scoped>
.about-view {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 18px 20px;
  overflow-y: auto;
  max-width: 640px;
}

.about-card {
  padding: 20px 22px 8px;
}

.about-head {
  display: flex;
  flex-direction: column;
  align-items: center;
  text-align: center;
}

.about-logo {
  border-radius: 14px;
  margin-bottom: 12px;
}

.about-name {
  font-size: 21px;
  font-weight: 700;
  line-height: 1.3;
}

.about-version {
  margin-top: 5px;
  font-size: 14px;
  font-weight: 600;
  letter-spacing: 2px;
  color: var(--accent);
  font-variant-numeric: tabular-nums;
}

.about-desc {
  margin: 8px 0 12px;
  font-size: 13.5px;
  color: var(--text-secondary);
  line-height: 1.6;
}

/* Label/value rows stay left-aligned inside the centered column */
.about-meta {
  width: fit-content;
  max-width: 100%;
  text-align: left;
  font-size: 13.5px;
}

.about-row {
  display: flex;
  gap: 4px;
  margin-top: 6px;
}

.about-label {
  flex-shrink: 0;
  color: var(--text-secondary);
}

.about-link {
  color: var(--accent);
  text-decoration: none;
  word-break: break-all;
}
.about-link:hover {
  text-decoration: underline;
}

/* Preference rows (theme / language) span the full card width, Fluent settings style */
.about-prefs {
  margin-top: 14px;
  text-align: left;
}

.pref-row {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 13px 2px;
  border-top: 1px solid var(--stroke);
}

.row-icon {
  display: inline-flex;
  color: var(--text-secondary);
}

.row-text {
  flex: 1;
  min-width: 0;
}

.row-label {
  font-size: 14px;
}

.row-hint {
  margin-top: 2px;
  font-size: 12px;
  color: var(--text-secondary);
}

.row-select {
  width: 190px;
}
</style>
