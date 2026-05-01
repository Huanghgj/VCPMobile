<script setup lang="ts">
// ToolInteractionOverlay.vue
// Container for Interactive tool UIs (camera, biometric, etc.)
// Phase 3 skeleton — will be populated when Interactive tools are added.

import { ref, onMounted, onUnmounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

interface ToolUiRequest {
  tool: string;
  id: string;
  args?: Record<string, any>;
}

const activeRequest = ref<ToolUiRequest | null>(null);
interface MobilePromptRequest {
  requestId: string;
  title: string;
  prompt: string;
  placeholder?: string;
  options?: string[];
  timeout?: number;
}

const promptRequest = ref<MobilePromptRequest | null>(null);
const promptResponse = ref("");
let unlisten: UnlistenFn | null = null;

onMounted(async () => {
  // Listen for tool UI requests from the Rust backend
  unlisten = await listen<ToolUiRequest>("tool-ui-request", (event) => {
    activeRequest.value = event.payload;
  });
});

onUnmounted(() => {
  unlisten?.();
});

let unlistenMobilePrompt: UnlistenFn | null = null;

onMounted(async () => {
  unlistenMobilePrompt = await listen<MobilePromptRequest>(
    "distributed-mobile-prompt",
    (event) => {
      promptRequest.value = event.payload;
      promptResponse.value = event.payload.placeholder || "";
    },
  );
});

onUnmounted(() => {
  unlistenMobilePrompt?.();
});

async function submitPrompt(response?: string, cancelled = false) {
  if (!promptRequest.value) return;
  const requestId = promptRequest.value.requestId;
  const finalResponse = response ?? promptResponse.value;
  promptRequest.value = null;
  promptResponse.value = "";
  try {
    await invoke("submit_distributed_prompt_response", {
      requestId,
      response: finalResponse,
      cancelled,
    });
  } catch (e) {
    console.error("[Distributed Prompt] Failed to submit response:", e);
  }
}

// Notification handler — listens for distributed-notification events
let unlistenNotification: UnlistenFn | null = null;

onMounted(async () => {
  unlistenNotification = await listen<{ title: string; body: string }>(
    "distributed-notification",
    (event) => {
      // Use browser Notification API or fallback
      if ("Notification" in window && Notification.permission === "granted") {
        new Notification(event.payload.title, { body: event.payload.body });
      } else {
        console.log(
          `[Distributed Notification] ${event.payload.title}: ${event.payload.body}`,
        );
      }
    },
  );
});

onUnmounted(() => {
  unlistenNotification?.();
});

// Clipboard write handler
let unlistenClipboard: UnlistenFn | null = null;

onMounted(async () => {
  unlistenClipboard = await listen<{ content: string }>(
    "distributed-clipboard-write",
    async (event) => {
      try {
        await navigator.clipboard.writeText(event.payload.content);
        console.log("[Distributed Clipboard] Content written.");
      } catch (e) {
        console.error("[Distributed Clipboard] Write failed:", e);
      }
    },
  );
});

onUnmounted(() => {
  unlistenClipboard?.();
});
</script>

<template>
  <!-- Interactive tool UI overlay — shown when a tool needs user interaction -->
  <Teleport to="#vcp-feature-overlays">
    <Transition name="fade">
      <div
        v-if="activeRequest"
        class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
      >
        <div
          class="bg-secondary-bg rounded-2xl p-6 mx-4 max-w-sm w-full shadow-2xl"
        >
          <div class="text-center space-y-3">
            <div class="text-lg font-bold text-primary-text">
              工具请求: {{ activeRequest.tool }}
            </div>
            <div class="text-sm opacity-60">
              此工具需要您的操作才能完成。
            </div>
            <!-- Phase 3+: tool-specific UI components will be rendered here -->
            <!-- e.g. <CameraCapture v-if="activeRequest.tool === 'camera'" /> -->
            <!-- e.g. <BiometricPrompt v-if="activeRequest.tool === 'biometric'" /> -->
            <div class="pt-4">
              <button
                class="px-4 py-2 bg-white/10 rounded-lg text-sm active:scale-95 transition-transform"
                @click="activeRequest = null"
              >
                取消
              </button>
            </div>
          </div>
        </div>
      </div>
    </Transition>

    <Transition name="fade">
      <div
        v-if="promptRequest"
        class="fixed inset-0 z-[260] flex items-center justify-center bg-black/65 backdrop-blur-md pointer-events-auto px-4"
      >
        <div
          class="w-full max-w-md rounded-[1.75rem] border border-white/10 bg-secondary-bg shadow-2xl overflow-hidden"
        >
          <div class="p-5 border-b border-white/10">
            <div class="text-xs uppercase tracking-[0.24em] opacity-45 font-mono">
              Mobile Tool Request
            </div>
            <div class="text-xl font-black text-primary-text mt-2">
              {{ promptRequest.title || "等待用户回复" }}
            </div>
            <div class="text-sm opacity-65 mt-2 leading-relaxed whitespace-pre-wrap">
              {{ promptRequest.prompt }}
            </div>
          </div>

          <div class="p-5 space-y-4">
            <div v-if="promptRequest.options?.length" class="grid gap-2">
              <button
                v-for="option in promptRequest.options"
                :key="option"
                class="w-full text-left rounded-2xl border border-white/10 bg-white/8 px-4 py-3 text-sm font-bold active:scale-[0.98] transition-transform"
                @click="submitPrompt(option)"
              >
                {{ option }}
              </button>
            </div>

            <textarea
              v-model="promptResponse"
              rows="4"
              class="w-full resize-none rounded-2xl border border-white/10 bg-black/10 px-4 py-3 text-sm text-primary-text outline-none focus:border-primary/60"
              :placeholder="promptRequest.placeholder || '输入回复...'"
            ></textarea>

            <div class="flex gap-3">
              <button
                class="flex-1 rounded-2xl bg-white/10 px-4 py-3 text-sm font-bold active:scale-[0.98] transition-transform"
                @click="submitPrompt('', true)"
              >
                取消
              </button>
              <button
                class="flex-1 rounded-2xl bg-primary text-white px-4 py-3 text-sm font-black active:scale-[0.98] transition-transform"
                @click="submitPrompt()"
              >
                提交
              </button>
            </div>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>
