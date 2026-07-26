<script setup lang="ts">
import { nextTick, onMounted, ref } from "vue";
import type { ContentBlock } from "../../core/types/chat";
import RenderDocumentBlock from "./components/RenderDocumentBlock.vue";

const messageId = "renderer-v2-browser-probe";
const stableRendered = ref(false);
const streamRenderCount = ref(0);
const identityPreserved = ref(false);
let firstStreamScene: Element | null = null;

const stableBlock: ContentBlock = {
  type: "markdown",
  content: [
    '<div id="vcp-root" data-testid="probe-rich-root" style="padding:20px16px 24px;border-radius:16px;background:linear-gradient(180deg,#fdf6e90%,#fcebd4 40%,#f9e0c0 100%);color:#3d2b1f;opacity:1">',
    "<style>",
    "@keyframes blurIn { from { opacity:0; transform:translateY(8px) } to { opacity:1; transform:translateY(0) } }",
    ".probe-scene { animation:blurIn .8s ease both; border:2px solid rgb(94,234,212); padding:12px; border-radius:10px }",
    "</style>",
    '<section id="probe-first" class="probe-scene">Renderer V2 first block</section>',
    '<div aria-hidden="true" style="height:110vh"></div>',
    '<section id="probe-offscreen" data-testid="probe-offscreen" class="probe-scene">Offscreen animation probe</section>',
    "</div>",
    "</div>",
    '<section id="probe-second" class="probe-scene"><span>Trailing block repaired into root</span></section>',
    "</div>",
  ].join(""),
};

const streamBlock = ref<ContentBlock>({
  type: "markdown",
  content:
    '<div id="vcp-root"><section id="probe-stream-scene" data-testid="probe-stream-scene"><span data-vcp-key="stream-copy">stream frame 1</span></section></div>',
});

function onStableRendered() {
  stableRendered.value = true;
}

onMounted(async () => {
  await nextTick();
  await nextTick();
  firstStreamScene = document.getElementById("probe-stream-scene");
  if (!firstStreamScene) return;
  streamRenderCount.value = 1;
  streamBlock.value = {
    type: "markdown",
    content:
      '<div id="vcp-root"><section id="probe-stream-scene" data-testid="probe-stream-scene"><span data-vcp-key="stream-copy">stream frame 2</span><b id="probe-stream-new">new node</b></section></div>',
  };
  await nextTick();
  await nextTick();
  const current = document.getElementById("probe-stream-scene");
  if (!current?.textContent?.includes("stream frame 2")) return;
  streamRenderCount.value = 2;
  identityPreserved.value = firstStreamScene === current;
});
</script>

<template>
  <div
    class="renderer-v2-probe"
    data-testid="renderer-v2-probe"
    :data-stable-rendered="stableRendered"
    :data-stream-render-count="streamRenderCount"
    :data-identity-preserved="identityPreserved"
    :data-message-id="messageId"
  >
    <h1>Renderer V2 Browser Probe</h1>
    <RenderDocumentBlock
      :block="stableBlock"
      :message-id="messageId"
      source-id="browser-probe-stable"
      @rendered="onStableRendered"
    />
    <RenderDocumentBlock
      :block="streamBlock"
      :message-id="messageId"
      source-id="browser-probe-stream"
      streaming
    />
  </div>
</template>

<style scoped>
.renderer-v2-probe {
  height: 100%;
  overflow: auto;
  padding: 24px;
  background: #0b1020;
  color: #f8fafc;
}

.renderer-v2-probe h1 {
  margin: 0 0 16px;
  font-size: 18px;
}
</style>
