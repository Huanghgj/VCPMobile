<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import Hls from "hls.js";
import { AlertTriangle, LoaderCircle } from "lucide-vue-next";
import { useWatchTogetherStore } from "../../core/stores/watchTogether";
import {
  EmbyClient,
  type EmbyMediaItem,
  type EmbyMediaSource,
} from "./embyClient";
import {
  RollingWatchCapture,
  type WatchCaptureMetadata,
} from "./watchCapture";

const props = defineProps<{
  client: EmbyClient;
  item: EmbyMediaItem;
}>();

const emit = defineEmits<{
  (event: "ready"): void;
}>();

const watchStore = useWatchTogetherStore();
const videoRef = ref<HTMLVideoElement | null>(null);
const loading = ref(true);
const errorMessage = ref("");
const mediaSource = ref<EmbyMediaSource | null>(null);
const playSessionId = ref<string>();
let hls: Hls | null = null;
let capture: RollingWatchCapture | null = null;
let progressTimer: number | null = null;

const displayTitle = computed(() =>
  props.item.SeriesName
    ? `${props.item.SeriesName} · ${props.item.Name}`
    : props.item.Name,
);

const activeSubtitle = () => {
  const video = videoRef.value;
  if (!video) return "";
  const lines: string[] = [];
  for (const track of Array.from(video.textTracks)) {
    if (!track.activeCues) continue;
    for (const cue of Array.from(track.activeCues)) {
      const text = "text" in cue ? String((cue as VTTCue).text || "") : "";
      if (text.trim()) lines.push(text.replace(/<[^>]+>/g, " ").replace(/\s+/g, " ").trim());
    }
  }
  return [...new Set(lines)].join(" / ");
};

const captureMetadata = (): WatchCaptureMetadata => {
  const video = videoRef.value;
  return {
    itemId: props.item.Id,
    title: props.item.Name,
    seriesName: props.item.SeriesName,
    currentTime: video?.currentTime || 0,
    duration:
      video && Number.isFinite(video.duration)
        ? video.duration
        : (props.item.RunTimeTicks || 0) / 10_000_000,
    paused: video?.paused ?? true,
    subtitle: activeSubtitle() || undefined,
  };
};

const reportPlayback = (event: "Playing" | "Progress" | "Stopped") => {
  const video = videoRef.value;
  const source = mediaSource.value;
  if (!video || !source) return;
  void props.client.reportPlayback(
    event,
    props.item.Id,
    source.Id,
    playSessionId.value,
    video.currentTime || 0,
    video.paused,
  );
};

const clearProgressTimer = () => {
  if (progressTimer !== null) {
    window.clearInterval(progressTimer);
    progressTimer = null;
  }
};

const startProgressTimer = () => {
  clearProgressTimer();
  progressTimer = window.setInterval(() => reportPlayback("Progress"), 10_000);
};

const destroyPlayback = () => {
  clearProgressTimer();
  hls?.destroy();
  hls = null;
  const video = videoRef.value;
  if (video) {
    video.pause();
    video.removeAttribute("src");
    video.load();
  }
};

const loadPlayback = async () => {
  destroyPlayback();
  loading.value = true;
  errorMessage.value = "";
  mediaSource.value = null;
  playSessionId.value = undefined;

  try {
    const info = await props.client.getPlaybackInfo(props.item.Id);
    const source = info.MediaSources?.[0];
    if (!source?.Id) throw new Error("Emby 没有返回可播放的媒体源");
    mediaSource.value = source;
    playSessionId.value = info.PlaySessionId;
    await nextTick();

    const video = videoRef.value;
    if (!video) return;
    const url = props.client.hlsUrl(props.item.Id, source.Id, info.PlaySessionId);

    if (video.canPlayType("application/vnd.apple.mpegurl")) {
      video.src = url;
    } else if (Hls.isSupported()) {
      hls = new Hls({
        enableWorker: true,
        lowLatencyMode: false,
        backBufferLength: 30,
        maxBufferLength: 45,
      });
      hls.on(Hls.Events.ERROR, (_event, data) => {
        if (!data.fatal) return;
        if (data.type === Hls.ErrorTypes.NETWORK_ERROR) {
          hls?.startLoad();
        } else if (data.type === Hls.ErrorTypes.MEDIA_ERROR) {
          hls?.recoverMediaError();
        } else {
          errorMessage.value = "播放器无法恢复当前媒体流";
          loading.value = false;
          hls?.destroy();
          hls = null;
        }
      });
      hls.loadSource(url);
      hls.attachMedia(video);
    } else {
      throw new Error("当前 WebView 不支持 HLS/MSE 播放");
    }
  } catch (error) {
    errorMessage.value = error instanceof Error ? error.message : String(error);
    loading.value = false;
  }
};

const handleLoaded = () => {
  loading.value = false;
  emit("ready");
  if (!capture && videoRef.value) {
    capture = new RollingWatchCapture(videoRef.value, captureMetadata);
  }
};

const handlePlay = () => {
  reportPlayback("Playing");
  startProgressTimer();
  void capture?.start();
};

const handlePause = () => {
  reportPlayback("Progress");
  clearProgressTimer();
};

const captureContext = async () => {
  if (!capture) {
    return { ...captureMetadata(), captureKind: "metadata" as const };
  }
  return capture.capture();
};

watch(() => props.item.Id, loadPlayback);

onMounted(() => {
  watchStore.setContextProvider({ capture: captureContext });
  void loadPlayback();
});

onBeforeUnmount(() => {
  reportPlayback("Stopped");
  watchStore.setContextProvider(null);
  capture?.dispose();
  capture = null;
  destroyPlayback();
});
</script>

<template>
  <div class="player-shell">
    <video
      ref="videoRef"
      class="watch-video"
      controls
      playsinline
      crossorigin="anonymous"
      :poster="client.imageUrl(item.Id, 'Backdrop')"
      @loadedmetadata="handleLoaded"
      @canplay="handleLoaded"
      @play="handlePlay"
      @pause="handlePause"
      @ended="reportPlayback('Stopped')"
    ></video>

    <div v-if="loading" class="player-state" aria-live="polite">
      <LoaderCircle :size="28" class="animate-spin" />
      <span>正在载入 {{ displayTitle }}</span>
    </div>
    <div v-else-if="errorMessage" class="player-state player-error" role="alert">
      <AlertTriangle :size="28" />
      <span>{{ errorMessage }}</span>
      <button type="button" @click="loadPlayback">重试</button>
    </div>
  </div>
</template>

<style scoped>
.player-shell {
  position: relative;
  width: 100%;
  height: 100%;
  min-height: 0;
  overflow: hidden;
  background: #050607;
}

.watch-video {
  width: 100%;
  height: 100%;
  display: block;
  object-fit: contain;
  background: #050607;
}

.player-state {
  position: absolute;
  inset: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 12px;
  padding: 24px;
  color: rgba(255, 255, 255, 0.82);
  background: rgba(5, 6, 7, 0.74);
  text-align: center;
  font-size: 13px;
}

.player-error {
  color: #fecaca;
}

.player-state button {
  min-height: 36px;
  padding: 0 16px;
  border: 1px solid rgba(255, 255, 255, 0.18);
  border-radius: 6px;
  background: rgba(255, 255, 255, 0.1);
  color: white;
  font-weight: 700;
}
</style>
