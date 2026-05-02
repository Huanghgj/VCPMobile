<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import {
  ArrowLeft,
  ArrowRight,
  Bot,
  Globe2,
  MousePointerClick,
  Play,
  RefreshCw,
  ScrollText,
  Server,
  SquareMousePointer,
  StopCircle,
  UserRoundCheck,
  X,
} from "lucide-vue-next";
import { useBrowserStore } from "./browserStore";
import type { BrowserAction } from "../../core/api/browserBridge";

const props = withDefaults(defineProps<{ isOpen?: boolean; zIndex?: number }>(), {
  isOpen: false,
  zIndex: 50,
});
const emit = defineEmits<{ close: [] }>();

const browserStore = useBrowserStore();
const url = ref("https://www.google.com");
const selector = ref("");
const text = ref("");
const value = ref("");
const script = ref("return document.title;");
const bindLan = ref(false);

const snapshot = computed(() => browserStore.snapshot);
const assistUrl = computed(() => browserStore.assist?.url || "");
const controlLabel = computed(() => {
  if (!snapshot.value) return "idle";
  return `${snapshot.value.controlMode} / ${snapshot.value.status}`;
});
const pageSummary = computed(() => ({
  title: snapshot.value?.title || "",
  url: snapshot.value?.url || "",
  lastAction: snapshot.value?.lastAction || "",
  waitingReason: snapshot.value?.waitingReason || "",
  bridgeKind: snapshot.value?.bridgeKind || "",
}));

watch(
  () => props.isOpen,
  (isOpen) => {
    if (isOpen) void browserStore.refreshSnapshot();
  },
);

onMounted(() => {
  if (props.isOpen) void browserStore.refreshSnapshot();
});

const runAction = async (action: BrowserAction) => {
  try {
    await browserStore.executeAction(action);
  } catch {
    // Store state already carries the user-facing error.
  }
};

const navigate = () => {
  const target = url.value.trim();
  if (!target) return;
  void runAction({ action: "navigate", url: target });
};

const clickTarget = () => {
  void runAction({
    action: "click",
    selector: selector.value.trim() || undefined,
    text: text.value.trim() || undefined,
  });
};

const typeTarget = () => {
  if (!selector.value.trim()) return;
  void runAction({
    action: "type",
    selector: selector.value.trim(),
    value: value.value,
  });
};

const startAssist = () => {
  void browserStore.startAssist(bindLan.value);
};
</script>

<template>
  <Teleport to="#vcp-feature-overlays" :disabled="!props.isOpen">
    <Transition name="fade">
      <div
        v-if="props.isOpen"
        class="browser-runtime fixed inset-0 flex flex-col bg-secondary-bg text-primary-text pointer-events-auto"
        :style="{ zIndex: props.zIndex }"
      >
        <header class="shrink-0 border-b border-white/10 px-4 pt-[calc(var(--vcp-safe-top,24px)+16px)] pb-3">
          <div class="flex items-center justify-between gap-3">
            <div class="min-w-0">
              <div class="flex items-center gap-2">
                <Globe2 :size="20" />
                <h2 class="truncate text-lg font-black">Mobile Browser</h2>
              </div>
              <div class="mt-1 truncate text-xs opacity-55">{{ controlLabel }}</div>
            </div>
            <button
              class="icon-btn"
              title="Close"
              @click="emit('close')"
            >
              <X :size="20" />
            </button>
          </div>
        </header>

        <div class="flex-1 overflow-y-auto p-4 pb-safe space-y-4">
          <section class="panel space-y-3">
            <div class="flex gap-2">
              <input
                v-model="url"
                class="field min-w-0 flex-1"
                placeholder="https://example.com"
                inputmode="url"
                @keydown.enter="navigate"
              />
              <button class="icon-btn primary" title="Open" :disabled="browserStore.loading" @click="navigate">
                <Play :size="18" />
              </button>
            </div>

            <div class="grid grid-cols-5 gap-2">
              <button class="icon-btn" title="Back" :disabled="browserStore.loading" @click="runAction({ action: 'back' })">
                <ArrowLeft :size="17" />
              </button>
              <button class="icon-btn" title="Forward" :disabled="browserStore.loading" @click="runAction({ action: 'forward' })">
                <ArrowRight :size="17" />
              </button>
              <button class="icon-btn" title="Reload" :disabled="browserStore.loading" @click="runAction({ action: 'reload' })">
                <RefreshCw :size="17" />
              </button>
              <button class="icon-btn" title="Snapshot" :disabled="browserStore.loading" @click="runAction({ action: 'snapshot' })">
                <ScrollText :size="17" />
              </button>
              <button class="icon-btn" title="Resume AI" :disabled="browserStore.loading" @click="runAction({ action: 'resume' })">
                <Bot :size="17" />
              </button>
            </div>
          </section>

          <section class="panel space-y-3">
            <div class="grid grid-cols-1 gap-2">
              <input v-model="selector" class="field" placeholder="CSS selector" />
              <input v-model="text" class="field" placeholder="Visible text" />
              <input v-model="value" class="field" placeholder="Input value" />
            </div>
            <div class="grid grid-cols-2 gap-2">
              <button class="action-btn" :disabled="browserStore.loading" @click="clickTarget">
                <MousePointerClick :size="16" />
                <span>Click</span>
              </button>
              <button class="action-btn" :disabled="browserStore.loading" @click="typeTarget">
                <SquareMousePointer :size="16" />
                <span>Type</span>
              </button>
              <button class="action-btn" :disabled="browserStore.loading" @click="runAction({ action: 'scroll', direction: 'down', amount: 720 })">
                <ScrollText :size="16" />
                <span>Scroll</span>
              </button>
              <button class="action-btn" :disabled="browserStore.loading" @click="runAction({ action: 'screenshot' })">
                <Globe2 :size="16" />
                <span>Shot</span>
              </button>
              <button class="action-btn" :disabled="browserStore.loading" @click="runAction({ action: 'handoff', reason: 'Manual verification requested.' })">
                <UserRoundCheck :size="16" />
                <span>Handoff</span>
              </button>
            </div>
          </section>

          <section class="panel space-y-3">
            <textarea v-model="script" class="field min-h-28 font-mono text-xs" spellcheck="false"></textarea>
            <button class="action-btn w-full justify-center" :disabled="browserStore.loading" @click="runAction({ action: 'eval', script })">
              <Play :size="16" />
              <span>Evaluate</span>
            </button>
          </section>

          <section class="panel space-y-3">
            <div class="flex items-center justify-between gap-3">
              <label class="flex items-center gap-2 text-sm font-bold">
                <input v-model="bindLan" type="checkbox" class="h-4 w-4 accent-emerald-500" />
                LAN
              </label>
              <div class="flex gap-2">
                <button class="icon-btn" title="Start assist server" :disabled="browserStore.loading" @click="startAssist">
                  <Server :size="17" />
                </button>
                <button class="icon-btn" title="Stop assist server" :disabled="browserStore.loading" @click="browserStore.stopAssist()">
                  <StopCircle :size="17" />
                </button>
              </div>
            </div>
            <a v-if="assistUrl" class="block break-all rounded-lg bg-black/10 px-3 py-2 text-xs dark:bg-white/10" :href="assistUrl">
              {{ assistUrl }}
            </a>
          </section>

          <section class="panel space-y-2">
            <img
              v-if="snapshot?.screenshot"
              :src="snapshot.screenshot"
              class="max-h-96 w-full rounded-lg border border-white/10 object-contain"
              alt=""
            />
            <pre class="snapshot">{{ JSON.stringify(pageSummary, null, 2) }}</pre>
            <pre v-if="snapshot?.text" class="snapshot max-h-64">{{ snapshot.text }}</pre>
          </section>

          <section v-if="browserStore.lastError" class="error-panel">
            {{ browserStore.lastError }}
          </section>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.browser-runtime {
  background-color: color-mix(in srgb, var(--primary-bg) 94%, transparent);
  backdrop-filter: blur(36px) saturate(180%);
}

.panel {
  border: 1px solid rgba(255, 255, 255, 0.1);
  border-radius: 8px;
  background: rgba(255, 255, 255, 0.06);
  padding: 12px;
}

.field {
  width: 100%;
  border-radius: 8px;
  border: 1px solid rgba(255, 255, 255, 0.1);
  background: rgba(0, 0, 0, 0.16);
  padding: 10px 12px;
  font-size: 14px;
  outline: none;
}

.icon-btn,
.action-btn {
  display: inline-flex;
  min-height: 42px;
  align-items: center;
  justify-content: center;
  gap: 8px;
  border-radius: 8px;
  background: rgba(255, 255, 255, 0.1);
  padding: 0 12px;
  font-weight: 700;
  transition: transform 0.15s ease, background-color 0.15s ease;
}

.icon-btn {
  width: 42px;
  padding: 0;
}

.icon-btn.primary,
.action-btn:active,
.icon-btn:active {
  background: rgba(34, 197, 94, 0.24);
}

.icon-btn:disabled,
.action-btn:disabled {
  opacity: 0.5;
}

.action-btn:active,
.icon-btn:active {
  transform: scale(0.96);
}

.snapshot {
  overflow: auto;
  white-space: pre-wrap;
  word-break: break-word;
  font-size: 11px;
  line-height: 1.5;
  opacity: 0.78;
}

.error-panel {
  border-radius: 8px;
  background: rgba(239, 68, 68, 0.14);
  color: rgb(254, 202, 202);
  padding: 12px;
  font-size: 12px;
}

.fade-enter-active,
.fade-leave-active {
  transition: opacity 0.25s ease;
}

.fade-enter-from,
.fade-leave-to {
  opacity: 0;
}
</style>
