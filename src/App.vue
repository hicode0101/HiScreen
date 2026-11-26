<script setup lang="ts">
import { computed, onMounted } from "vue";
import {
  NConfigProvider,
  NMessageProvider,
  darkTheme,
  dateEnUS,
  dateZhCN,
  enUS,
  zhCN,
} from "naive-ui";
import { useI18n } from "vue-i18n";
import { fluentDark, fluentLight } from "@/theme/fluent";
import { resolvedTheme } from "@/lib/theme";
import TitleBar from "@/components/TitleBar.vue";
import Sidebar from "@/components/Sidebar.vue";
import ShareView from "@/components/ShareView.vue";
import AudienceView from "@/components/AudienceView.vue";
import NetworkView from "@/components/NetworkView.vue";
import QualityView from "@/components/QualityView.vue";
import LogView from "@/components/LogView.vue";
import StatusBar from "@/components/StatusBar.vue";
import AboutDialog from "@/components/AboutDialog.vue";
import { ui } from "@/stores/ui";
import { initShare, disposeShare } from "@/stores/share";

const { locale } = useI18n();

const isDark = computed(() => resolvedTheme.value === "dark");
const naiveLocale = computed(() => (locale.value === "en-US" ? enUS : zhCN));
const naiveDateLocale = computed(() =>
  locale.value === "en-US" ? dateEnUS : dateZhCN,
);

onMounted(() => {
  void initShare();
});

// The app is a single-window desktop tool; no view unmount lifecycle exists,
// so the poller only needs explicit cleanup on hot-reload of this module.
if (import.meta.hot) {
  import.meta.hot.dispose(() => disposeShare());
}
</script>

<template>
  <NConfigProvider
    :theme="isDark ? darkTheme : null"
    :theme-overrides="isDark ? fluentDark : fluentLight"
    :locale="naiveLocale"
    :date-locale="naiveDateLocale"
  >
    <!-- Toasts pop in the bottom-right corner, Windows 11 notification style -->
    <NMessageProvider placement="bottom-right" :duration="4000" :max="3" keep-alive-on-hover>
      <div class="app-shell">
        <TitleBar />
        <div class="app-body">
          <Sidebar />
          <main class="app-main">
            <ShareView v-if="ui.view === 'share'" />
            <AudienceView v-else-if="ui.view === 'audience'" />
            <NetworkView v-else-if="ui.view === 'network'" />
            <QualityView v-else-if="ui.view === 'quality'" />
            <LogView v-else />
          </main>
        </div>
        <StatusBar />
      </div>
      <AboutDialog />
    </NMessageProvider>
  </NConfigProvider>
</template>
