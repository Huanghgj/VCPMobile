<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import {
  Clapperboard,
  Library,
  LoaderCircle,
  LogOut,
  Menu,
  Play,
  Search,
  Server,
  X,
} from "lucide-vue-next";
import ChatView from "../chat/ChatView.vue";
import { useLayoutStore } from "../../core/stores/layout";
import { useWatchTogetherStore } from "../../core/stores/watchTogether";
import {
  authenticateEmby,
  EmbyClient,
  normalizeEmbyServerUrl,
  type EmbyMediaItem,
} from "./embyClient";
import EmbyPlayer from "./EmbyPlayer.vue";

const layoutStore = useLayoutStore();
const watchStore = useWatchTogetherStore();
const router = useRouter();
const serverUrl = ref(watchStore.connection?.serverUrl || "");
const username = ref(watchStore.connection?.userName || "");
const password = ref("");
const connectionError = ref("");
const connecting = ref(false);
const sessionReady = ref(false);
const mediaLoading = ref(false);
const mediaError = ref("");
const mediaItems = ref<EmbyMediaItem[]>([]);
const selectedItem = ref<EmbyMediaItem | null>(null);
const libraryOpen = ref(true);
const searchTerm = ref("");

const client = computed(() =>
  watchStore.connection ? new EmbyClient(watchStore.connection) : null,
);

const serverLabel = computed(
  () => watchStore.connection?.serverName || watchStore.connection?.serverUrl || "Emby",
);

const itemSubtitle = (item: EmbyMediaItem) => {
  if (item.Type === "Episode") {
    const parts = [item.SeriesName];
    if (item.ParentIndexNumber !== undefined && item.IndexNumber !== undefined) {
      parts.push(
        `S${String(item.ParentIndexNumber).padStart(2, "0")}E${String(item.IndexNumber).padStart(2, "0")}`,
      );
    }
    return parts.filter(Boolean).join(" · ");
  }
  return item.ProductionYear ? String(item.ProductionYear) : "电影";
};

const loadMedia = async () => {
  if (!client.value) return;
  mediaLoading.value = true;
  mediaError.value = "";
  try {
    mediaItems.value = await client.value.getMediaItems(searchTerm.value);
  } catch (error) {
    mediaError.value = error instanceof Error ? error.message : String(error);
  } finally {
    mediaLoading.value = false;
  }
};

const restoreSession = async () => {
  if (!client.value) return;
  connecting.value = true;
  connectionError.value = "";
  try {
    await client.value.validateSession();
    sessionReady.value = true;
    await loadMedia();
  } catch (error) {
    sessionReady.value = false;
    connectionError.value = `会话已失效：${error instanceof Error ? error.message : String(error)}`;
  } finally {
    connecting.value = false;
  }
};

const connect = async () => {
  if (!serverUrl.value.trim() || !username.value.trim()) return;
  connecting.value = true;
  connectionError.value = "";
  try {
    const normalizedUrl = normalizeEmbyServerUrl(serverUrl.value);
    const session = await authenticateEmby(
      normalizedUrl,
      username.value,
      password.value,
      watchStore.deviceId,
    );
    watchStore.setConnection({
      serverUrl: normalizedUrl,
      accessToken: session.accessToken,
      userId: session.userId,
      userName: session.userName,
      serverName: session.serverName,
    });
    password.value = "";
    sessionReady.value = true;
    await loadMedia();
  } catch (error) {
    connectionError.value = error instanceof Error ? error.message : String(error);
  } finally {
    connecting.value = false;
  }
};

const disconnect = () => {
  selectedItem.value = null;
  mediaItems.value = [];
  sessionReady.value = false;
  watchStore.clearConnection();
  watchStore.setContextProvider(null);
};

const selectItem = (item: EmbyMediaItem) => {
  selectedItem.value = item;
  libraryOpen.value = false;
};

const closeWatchTogether = async () => {
  await router.push("/chat");
};

onMounted(() => {
  watchStore.active = true;
  if (watchStore.connected) void restoreSession();
});

onBeforeUnmount(() => {
  watchStore.active = false;
  watchStore.setContextProvider(null);
});
</script>

<template>
  <div class="watch-view">
    <section class="watch-stage" aria-label="Emby 播放器">
      <header class="watch-toolbar">
        <div class="toolbar-leading">
          <button
            type="button"
            class="icon-button"
            title="打开侧栏"
            aria-label="打开侧栏"
            @click="layoutStore.toggleLeftDrawer()"
          >
            <Menu :size="20" />
          </button>
          <Clapperboard :size="20" class="toolbar-mark" />
          <div class="toolbar-title">
            <strong>{{ selectedItem?.Name || "一起看" }}</strong>
            <span>{{ selectedItem?.SeriesName || serverLabel }}</span>
          </div>
        </div>

        <div class="toolbar-actions">
          <button
            v-if="sessionReady && selectedItem"
            type="button"
            class="icon-button"
            :class="{ active: libraryOpen }"
            title="媒体库"
            aria-label="媒体库"
            @click="libraryOpen = !libraryOpen"
          >
            <Library :size="19" />
          </button>
          <button
            v-if="sessionReady"
            type="button"
            class="icon-button"
            title="退出 Emby"
            aria-label="退出 Emby"
            @click="disconnect"
          >
            <LogOut :size="18" />
          </button>
          <button
            type="button"
            class="icon-button"
            title="关闭一起看"
            aria-label="关闭一起看"
            @click="closeWatchTogether"
          >
            <X :size="19" />
          </button>
        </div>
      </header>

      <div v-if="!sessionReady" class="connection-panel">
        <div class="connection-heading">
          <Server :size="30" />
          <div>
            <h1>连接 Emby</h1>
            <p>使用你的媒体服务器账户</p>
          </div>
        </div>

        <form class="connection-form" @submit.prevent="connect">
          <label>
            <span>服务器地址</span>
            <input
              v-model="serverUrl"
              type="url"
              inputmode="url"
              autocomplete="url"
              placeholder="https://emby.example.com"
              required
            />
          </label>
          <div class="form-row">
            <label>
              <span>用户名</span>
              <input v-model="username" autocomplete="username" required />
            </label>
            <label>
              <span>密码</span>
              <input
                v-model="password"
                type="password"
                autocomplete="current-password"
              />
            </label>
          </div>
          <p v-if="connectionError" class="form-error" role="alert">
            {{ connectionError }}
          </p>
          <button class="primary-command" type="submit" :disabled="connecting">
            <LoaderCircle v-if="connecting" :size="18" class="animate-spin" />
            <Server v-else :size="18" />
            <span>{{ connecting ? "连接中" : "连接" }}</span>
          </button>
        </form>
      </div>

      <div v-else class="stage-content">
        <EmbyPlayer
          v-if="selectedItem && client"
          :key="selectedItem.Id"
          :client="client"
          :item="selectedItem"
        />

        <div v-else class="stage-empty">
          <Play :size="32" />
          <strong>从媒体库选择内容</strong>
        </div>

        <Transition name="library-fade">
          <div v-if="libraryOpen" class="media-library">
            <div class="library-header">
              <div>
                <strong>媒体库</strong>
                <span>{{ mediaItems.length }} 项</span>
              </div>
              <button
                v-if="selectedItem"
                type="button"
                class="icon-button"
                title="关闭媒体库"
                aria-label="关闭媒体库"
                @click="libraryOpen = false"
              >
                <X :size="18" />
              </button>
            </div>

            <form class="library-search" @submit.prevent="loadMedia">
              <Search :size="17" />
              <input v-model="searchTerm" type="search" placeholder="搜索电影或剧集" />
              <button type="submit" title="搜索" aria-label="搜索">
                <Search :size="17" />
              </button>
            </form>

            <div v-if="mediaLoading" class="library-state">
              <LoaderCircle :size="24" class="animate-spin" />
              <span>正在载入媒体库</span>
            </div>
            <div v-else-if="mediaError" class="library-state library-error">
              <span>{{ mediaError }}</span>
              <button type="button" @click="loadMedia">重试</button>
            </div>
            <div v-else class="media-grid vcp-scrollable">
              <button
                v-for="item in mediaItems"
                :key="item.Id"
                type="button"
                class="media-card"
                :class="{ selected: selectedItem?.Id === item.Id }"
                @click="selectItem(item)"
              >
                <div class="poster-frame">
                  <img
                    :src="client?.imageUrl(item.Id)"
                    :alt="item.Name"
                    loading="lazy"
                    decoding="async"
                  />
                  <span class="play-badge"><Play :size="15" fill="currentColor" /></span>
                </div>
                <strong>{{ item.Name }}</strong>
                <span>{{ itemSubtitle(item) }}</span>
              </button>
            </div>
          </div>
        </Transition>
      </div>
    </section>

    <aside class="watch-chat" aria-label="AI 聊天">
      <ChatView />
    </aside>
  </div>
</template>

<style scoped>
.watch-view {
  width: 100%;
  height: 100%;
  min-width: 0;
  min-height: 0;
  display: grid;
  grid-template-columns: minmax(0, 2fr) minmax(320px, 1fr);
  overflow: hidden;
  background: var(--primary-bg);
  color: var(--primary-text);
}

.watch-stage,
.watch-chat {
  min-width: 0;
  min-height: 0;
  overflow: hidden;
}

.watch-stage {
  display: flex;
  flex-direction: column;
  border-right: 1px solid color-mix(in srgb, var(--primary-text) 9%, transparent);
  background: #08090a;
}

.watch-chat {
  background: var(--secondary-bg);
}

.watch-chat :deep(.vcp-header-fixed) {
  padding-top: max(12px, var(--vcp-safe-top, 0px));
}

.watch-toolbar {
  min-height: calc(58px + var(--vcp-safe-top, 0px));
  padding: var(--vcp-safe-top, 0px) 14px 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  flex: none;
  color: rgba(255, 255, 255, 0.92);
  background: #101214;
  border-bottom: 1px solid rgba(255, 255, 255, 0.08);
}

.toolbar-leading,
.toolbar-actions {
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 0;
}

.toolbar-mark {
  color: #ef4444;
  flex: none;
}

.toolbar-title {
  min-width: 0;
  display: flex;
  flex-direction: column;
}

.toolbar-title strong,
.toolbar-title span {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.toolbar-title strong {
  font-size: 13px;
}

.toolbar-title span {
  margin-top: 2px;
  color: rgba(255, 255, 255, 0.5);
  font-size: 10px;
}

.icon-button {
  width: 38px;
  height: 38px;
  display: inline-grid;
  place-items: center;
  flex: none;
  border: 1px solid rgba(255, 255, 255, 0.09);
  border-radius: 6px;
  color: rgba(255, 255, 255, 0.78);
  background: rgba(255, 255, 255, 0.06);
}

.icon-button:active,
.icon-button.active {
  color: white;
  background: rgba(239, 68, 68, 0.24);
  transform: scale(0.95);
}

.connection-panel {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  justify-content: center;
  width: min(560px, calc(100% - 40px));
  margin: 0 auto;
  padding: 28px 0;
  color: rgba(255, 255, 255, 0.9);
}

.connection-panel::-webkit-scrollbar {
  display: none;
}

.connection-heading {
  display: flex;
  align-items: center;
  gap: 14px;
  margin-bottom: 24px;
}

.connection-heading > svg {
  color: #ef4444;
}

.connection-heading h1 {
  margin: 0;
  font-size: 22px;
  letter-spacing: 0;
}

.connection-heading p {
  margin: 4px 0 0;
  color: rgba(255, 255, 255, 0.48);
  font-size: 12px;
}

.connection-form,
.connection-form label {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.connection-form {
  gap: 16px;
}

.connection-form label > span {
  color: rgba(255, 255, 255, 0.56);
  font-size: 11px;
  font-weight: 700;
}

.connection-form input,
.library-search input {
  width: 100%;
  min-width: 0;
  border: 1px solid rgba(255, 255, 255, 0.12);
  border-radius: 6px;
  outline: none;
  color: white;
  background: rgba(255, 255, 255, 0.07);
}

.connection-form input {
  height: 44px;
  padding: 0 12px;
  font-size: 14px;
}

.connection-form input:focus,
.library-search input:focus {
  border-color: rgba(239, 68, 68, 0.7);
}

.form-row {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 12px;
}

.form-error {
  margin: 0;
  color: #fca5a5;
  font-size: 12px;
  line-height: 1.5;
}

.primary-command {
  min-height: 42px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  border: 0;
  border-radius: 6px;
  color: white;
  background: #dc2626;
  font-weight: 800;
}

.primary-command:disabled {
  opacity: 0.55;
}

.stage-content {
  position: relative;
  flex: 1;
  min-height: 0;
  overflow: hidden;
}

.stage-empty {
  position: absolute;
  inset: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 12px;
  color: rgba(255, 255, 255, 0.38);
}

.media-library {
  position: absolute;
  inset: 0;
  display: flex;
  flex-direction: column;
  min-height: 0;
  padding: 18px;
  color: rgba(255, 255, 255, 0.9);
  background: #0d0f11;
}

.library-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-bottom: 12px;
}

.library-header > div {
  display: flex;
  align-items: baseline;
  gap: 8px;
}

.library-header strong {
  font-size: 16px;
}

.library-header span {
  color: rgba(255, 255, 255, 0.42);
  font-size: 10px;
}

.library-search {
  height: 40px;
  display: grid;
  grid-template-columns: auto 1fr auto;
  align-items: center;
  gap: 8px;
  margin-bottom: 14px;
  padding-left: 12px;
  border: 1px solid rgba(255, 255, 255, 0.1);
  border-radius: 6px;
  color: rgba(255, 255, 255, 0.45);
  background: rgba(255, 255, 255, 0.05);
}

.library-search input {
  height: 100%;
  padding: 0;
  border: 0;
  background: transparent;
  font-size: 13px;
}

.library-search button {
  width: 40px;
  height: 38px;
  display: grid;
  place-items: center;
  color: rgba(255, 255, 255, 0.72);
}

.library-state {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 10px;
  color: rgba(255, 255, 255, 0.52);
  font-size: 12px;
}

.library-error {
  color: #fca5a5;
}

.library-error button {
  padding: 8px 14px;
  border: 1px solid rgba(255, 255, 255, 0.16);
  border-radius: 6px;
  color: white;
}

.media-grid {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(118px, 1fr));
  align-content: start;
  gap: 16px 12px;
  padding: 2px 4px 20px 0;
}

.media-card {
  min-width: 0;
  display: flex;
  flex-direction: column;
  align-items: stretch;
  gap: 4px;
  color: rgba(255, 255, 255, 0.88);
  text-align: left;
}

.poster-frame {
  position: relative;
  width: 100%;
  aspect-ratio: 2 / 3;
  overflow: hidden;
  border: 1px solid rgba(255, 255, 255, 0.08);
  border-radius: 6px;
  background: #17191c;
}

.poster-frame img {
  width: 100%;
  height: 100%;
  display: block;
  object-fit: cover;
}

.play-badge {
  position: absolute;
  right: 8px;
  bottom: 8px;
  width: 30px;
  height: 30px;
  display: grid;
  place-items: center;
  border-radius: 50%;
  color: white;
  background: rgba(220, 38, 38, 0.92);
  opacity: 0;
  transform: translateY(4px);
  transition: opacity 160ms ease, transform 160ms ease;
}

.media-card:hover .play-badge,
.media-card:focus-visible .play-badge,
.media-card.selected .play-badge {
  opacity: 1;
  transform: translateY(0);
}

.media-card strong,
.media-card > span {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.media-card strong {
  margin-top: 3px;
  font-size: 12px;
}

.media-card > span {
  color: rgba(255, 255, 255, 0.42);
  font-size: 9px;
}

.library-fade-enter-active,
.library-fade-leave-active {
  transition: opacity 160ms ease;
}

.library-fade-enter-from,
.library-fade-leave-to {
  opacity: 0;
}

@media (max-width: 900px) and (orientation: portrait) {
  .watch-view {
    grid-template-columns: 1fr;
    grid-template-rows: minmax(230px, 42%) minmax(0, 58%);
  }

  .watch-stage {
    border-right: 0;
    border-bottom: 1px solid color-mix(in srgb, var(--primary-text) 9%, transparent);
  }

  .watch-toolbar {
    min-height: calc(50px + var(--vcp-safe-top, 0px));
  }

  .watch-chat :deep(.vcp-header-fixed) {
    padding-top: 8px;
  }

  .media-grid {
    grid-template-columns: repeat(auto-fill, minmax(92px, 1fr));
  }

  .connection-panel {
    width: calc(100% - 28px);
    justify-content: flex-start;
    overflow-y: auto;
    scrollbar-width: none;
    padding: 14px 0 20px;
  }

  .connection-heading {
    margin-bottom: 12px;
  }

  .connection-heading h1 {
    font-size: 18px;
  }
}

@media (max-width: 560px) {
  .form-row {
    grid-template-columns: 1fr;
  }
}
</style>
