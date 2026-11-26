<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from "vue";
import { NButton, NInput, NInputNumber, NSwitch } from "naive-ui";
import { useI18n } from "vue-i18n";
import { useMessage } from "naive-ui";
import { openExternal, api, isTauri } from "@/lib/tauri";
import {
  applyPassword,
  clampPort,
  persistSettings,
  settings,
} from "@/stores/settings";
import { applyPort, share } from "@/stores/share";
import Icon from "./Icon.vue";

const { t } = useI18n();
const message = useMessage();

const portDraft = ref<number>(settings.port);
const saving = ref(false);

const addresses = computed(() =>
  share.status.running ? share.status.addresses : [],
);

function onPort(value: number | null): void {
  portDraft.value = clampPort(Number(value) || settings.port);
}

async function savePort(): Promise<void> {
  settings.port = clampPort(portDraft.value);
  portDraft.value = settings.port;
  saving.value = true;
  try {
    await persistSettings();
    await applyPort();
    message.success(t("network.portApplied"));
  } catch {
    message.error(t("network.portFailed"));
  } finally {
    saving.value = false;
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

// -------------------------------------------------------- connection password

/** Draft being edited; the applied password lives in settings.pw. */
const pwDraft = ref<string>(settings.pw);
/** Value the backend currently holds, so no-op edits skip the toast. */
let appliedPw = settings.pw;
let pwApplyTimer: number | null = null;

/** Same rule the backend enforces: 4–6 ASCII letters/digits. */
function validPw(pw: string): boolean {
  return /^[A-Za-z0-9]{4,6}$/.test(pw);
}

const pwInvalid = computed(
  () => settings.pwEnabled && pwDraft.value !== settings.pw && !validPw(pwDraft.value),
);

function randomPw(): string {
  // Random 6-char mix (upper + lower + digit, one of each guaranteed);
  // look-alikes (0/O/o, 1/l/I) are left out so the code reads clearly.
  const upper = "ABCDEFGHJKLMNPQRSTUVWXYZ";
  const lower = "abcdefghijkmnpqrstuvwxyz";
  const digits = "23456789";
  const all = upper + lower + digits;
  const rand = (set: string): string => {
    const buf = new Uint32Array(1);
    crypto.getRandomValues(buf);
    return set[buf[0] % set.length];
  };
  const chars = [rand(upper), rand(lower), rand(digits)];
  while (chars.length < 6) chars.push(rand(all));
  for (let i = chars.length - 1; i > 0; i--) {
    const buf = new Uint32Array(1);
    crypto.getRandomValues(buf);
    const j = buf[0] % (i + 1);
    [chars[i], chars[j]] = [chars[j], chars[i]];
  }
  return chars.join("");
}

function onPwEnabled(value: boolean): void {
  // Session-only switch: enabling always mints a fresh random password,
  // and nothing is written to config.json.
  settings.pwEnabled = value;
  settings.pw = value ? randomPw() : "";
  pwDraft.value = settings.pw;
  appliedPw = settings.pw;
  void applyPassword();
}

function onPwEdit(value: string): void {
  pwDraft.value = value;
  if (!validPw(value)) return; // invalid drafts are never applied
  settings.pw = value;
  schedulePwApply();
}

function schedulePwApply(): void {
  if (pwApplyTimer !== null) clearTimeout(pwApplyTimer);
  pwApplyTimer = window.setTimeout(() => {
    pwApplyTimer = null;
    void applyEditedPw();
  }, 300);
}

/** Persist a manually edited password; confirm with a bottom-right toast. */
async function applyEditedPw(): Promise<void> {
  if (!isTauri() || !settings.loaded || settings.pw === appliedPw) return;
  try {
    await api.setPassword(settings.pwEnabled, settings.pw);
    appliedPw = settings.pw;
    message.success(t("network.pwChanged"));
  } catch {
    message.error(t("network.pwChangeFailed"));
  }
}

function regenPw(): void {
  settings.pw = randomPw();
  pwDraft.value = settings.pw;
  // Same path as a manual edit: apply + bottom-right toast (appliedPw is
  // updated inside applyEditedPw once the backend confirms).
  void applyEditedPw();
}

function copyPw(): void {
  if (!settings.pw) return;
  void navigator.clipboard
    .writeText(settings.pw)
    .then(() => message.success(t("network.pwCopied")))
    .catch(() => message.error(t("share.copyFailed")));
}

onBeforeUnmount(() => {
  // A pending edit is flushed, not dropped: navigating away right after
  // typing still applies (and toasts) the change.
  if (pwApplyTimer !== null) {
    clearTimeout(pwApplyTimer);
    pwApplyTimer = null;
    void applyEditedPw();
  }
});
</script>

<template>
  <div class="network-view">
    <h2 class="view-title">{{ t("network.title") }}</h2>

    <section class="card">
      <h3 class="section-title">{{ t("network.service") }}</h3>
      <div class="status-row">
        <span class="dot" :class="{ on: share.status.running }"></span>
        <span>{{ share.status.running ? t("network.running") : t("network.stopped") }}</span>
        <template v-if="share.status.running">
          <span class="mono addr">{{ share.status.url }}</span>
          <button class="icon-btn" :title="t('share.copy')" @click="copyUrl">
            <Icon name="copy" :size="14" />
          </button>
          <button class="icon-btn" :title="t('share.open')" @click="openInBrowser">
            <Icon name="globe" :size="14" />
          </button>
        </template>
      </div>
      <ul class="addr-list" v-if="addresses.length > 1">
        <li v-for="a in addresses.slice(1)" :key="a" class="mono">{{ a }}</li>
      </ul>
    </section>

    <section class="card">
      <h3 class="section-title">{{ t("network.portTitle") }}</h3>
      <div class="port-row">
        <NInputNumber
          class="port-input"
          size="small"
          :value="portDraft"
          :min="1024"
          :max="65535"
          :step="1"
          :show-button="false"
          @update:value="onPort"
        />
        <NButton size="small" type="primary" :loading="saving" @click="void savePort()">
          {{ t("network.apply") }}
        </NButton>
      </div>
      <p class="hint">{{ t("network.portHint") }}</p>
    </section>

    <section class="card">
      <h3 class="section-title">{{ t("network.pwTitle") }}</h3>
      <div class="pw-row">
        <label class="toggle-item">
          <NSwitch
            size="small"
            :value="settings.pwEnabled"
            @update:value="onPwEnabled"
          />
          <span>{{ t("network.pwEnable") }}</span>
        </label>
        <template v-if="settings.pwEnabled">
          <NInput
            class="pw-input"
            size="small"
            :value="pwDraft"
            :maxlength="6"
            :status="pwInvalid ? 'error' : undefined"
            @update:value="onPwEdit"
          />
          <button class="icon-btn" :title="t('network.pwRegen')" @click="regenPw">
            <Icon name="refresh" :size="14" />
          </button>
          <button class="icon-btn" :title="t('network.pwCopy')" @click="copyPw">
            <Icon name="copy" :size="14" />
          </button>
        </template>
      </div>
      <p class="hint" :class="{ danger: pwInvalid }">
        {{
          pwInvalid
            ? t("network.pwInvalid")
            : settings.pwEnabled
              ? t("network.pwOnHint")
              : t("network.pwHint")
        }}
      </p>
    </section>
  </div>
</template>

<style scoped>
.network-view {
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
  margin: 0 0 12px;
  font-size: 15px;
  font-weight: 650;
}

.status-row {
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: 13.5px;
}

.dot {
  width: 9px;
  height: 9px;
  border-radius: 50%;
  background: var(--text-tertiary);
}

.dot.on {
  background: var(--success);
  box-shadow: 0 0 6px rgba(15, 123, 15, 0.5);
}

.mono {
  font-variant-numeric: tabular-nums;
  font-family: "Cascadia Code", Consolas, monospace;
}

.addr {
  color: var(--accent);
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

.addr-list {
  margin: 10px 0 0;
  padding: 0 0 0 19px;
  list-style: none;
  font-size: 12.5px;
  color: var(--text-secondary);
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.port-row {
  display: flex;
  align-items: center;
  gap: 10px;
}

.port-input {
  width: 140px;
}

.pw-row {
  display: flex;
  align-items: center;
  gap: 12px;
}

.toggle-item {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 13px;
  cursor: pointer;
}

.pw-input {
  width: 130px;
}

.pw-input :deep(input) {
  font-family: "Cascadia Code", Consolas, monospace;
  letter-spacing: 2px;
}

.hint {
  margin: 12px 0 0;
  font-size: 12.5px;
  color: var(--text-tertiary);
  line-height: 1.6;
  white-space: pre-line;
}

.hint.danger {
  color: #d03050;
}
</style>
