<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { ContentBlock } from "../../core/types/chat";
import { useChatHistoryStore } from "../../core/stores/chatHistoryStore";
import { isRenderDocumentBlock } from "../../core/utils/renderDocument";
import HtmlPreviewBlock from "./blocks/HtmlPreviewBlock.vue";
import ThoughtBlock from "./blocks/ThoughtBlock.vue";
import ToolBlock from "./blocks/ToolBlock.vue";
import ToolSummaryBlock from "./blocks/ToolSummaryBlock.vue";
import RenderDocumentBlock from "./components/RenderDocumentBlock.vue";

type MarkerPolicy = "hidden" | "literal" | "none";

interface ProbeFixture {
  id: string;
  title: string;
  content: string;
  requiredTypes: ContentBlock["type"][];
  forbiddenTypes?: ContentBlock["type"][];
  markerPolicy: MarkerPolicy;
}

interface ProbeResult extends ProbeFixture {
  blocks: ContentBlock[];
  error?: string;
}

const TOOL_MARKER = "<<<[TOOL_REQUEST]>>>";
const messageId = "renderer-v2-android-probe";
const historyStore = useChatHistoryStore();
const originalSendMessage = historyStore.sendMessage;
const aiActionCount = ref(0);
const lastAiAction = ref("");
const probeReady = ref(false);
const results = ref<ProbeResult[]>([]);
const streamRenderCount = ref(0);
const identityPreserved = ref(false);
const reloadCount =
  Number(sessionStorage.getItem("vcp-renderer-probe-reloads") || "0") + 1;
sessionStorage.setItem("vcp-renderer-probe-reloads", String(reloadCount));

historyStore.sendMessage = (async (content: string) => {
  aiActionCount.value += 1;
  lastAiAction.value = content;
  return true;
}) as typeof historyStore.sendMessage;

const fixtures: ProbeFixture[] = [
  {
    id: "unclosed-html-incomplete-tool",
    title: "Unclosed vcp-root/w2g/catsay plus incomplete ComfyUIGen",
    content: [
      '<div id="vcp-root" data-probe="unclosed-root" style="padding:12px;background:#17345f;color:#fff">',
      '<section data-probe="unclosed-visible">Visible story before tool request</section>',
      "<w2g><catsay><details><summary>Local review</summary><p>Review body</p></details>",
      TOOL_MARKER,
      "maid:「始」Nova「末」,",
      "tool_name:「始」ComfyUIGen「末」,",
      "mode:「始」anima「末」,",
      "prompt:「始」runtime prompt still streaming「末」",
    ].join("\n"),
    requiredTypes: ["markdown", "tool-use"],
    markerPolicy: "hidden",
  },
  {
    id: "closed-html-incomplete-tool",
    title: "Closed HTML plus incomplete tool request",
    content: [
      '<div id="vcp-root"><section data-probe="closed-visible">Closed HTML remains visible</section></div>',
      TOOL_MARKER,
      "maid:「始」Nova「末」,",
      "tool_name:「始」ComfyUIGen「末」,",
      "prompt:「始」closed container stream tail「末」",
    ].join("\n"),
    requiredTypes: ["markdown", "tool-use"],
    markerPolicy: "hidden",
  },
  {
    id: "stuck-dailynote-no-newline",
    title: "DailyNote marker stuck directly to closing HTML",
    content: [
      '<div id="vcp-root"><section data-probe="stuck-visible">Stuck marker story</section></div>' +
        TOOL_MARKER,
      "maid:「始」Nova「末」,",
      "tool_name:「始」DailyNote「末」,",
      "command:「始」create「末」,",
      "Date:「始」2026-08-02「末」,",
      "folder:「始」Nova「末」,",
      "Content:「始ESCAPE」[23:40] runtime diary probe「末ESCAPE」,",
      "archery:「始」no_reply「末」",
      "<<<[END_TOOL_REQUEST]>>>",
    ].join("\n"),
    requiredTypes: ["markdown", "diary"],
    markerPolicy: "hidden",
  },
  {
    id: "nested-dailynote-tail",
    title: "Complete nested DailyNote followed by generated tail HTML",
    content: [
      '<div id="vcp-root"><section data-probe="nested-visible">Nested diary story</section><w2g><catsay>Tail card',
      TOOL_MARKER,
      "maid:「始」Nova「末」,",
      "tool_name:「始」DailyNote「末」,",
      "command:「始」create「末」,",
      "Date:「始」2026-08-02「末」,",
      "folder:「始」Nova「末」,",
      "Content:「始ESCAPE」[23:41] nested diary body「末ESCAPE」,",
      "archery:「始」no_reply「末」",
      "<<<[END_TOOL_REQUEST]>>>",
      '<section data-probe="nested-tail">Tail content after DailyNote</section>',
    ].join("\n"),
    requiredTypes: ["markdown", "diary"],
    markerPolicy: "hidden",
  },
  {
    id: "literal-pre-code",
    title: "Literal tool marker inside pre/code",
    content: [
      '<div id="vcp-root" data-probe="literal-pre"><pre><code>Protocol documentation',
      TOOL_MARKER,
      "tool_name: literal-only",
      "</code></pre></div>",
    ].join("\n"),
    requiredTypes: ["markdown"],
    forbiddenTypes: ["tool-use", "diary"],
    markerPolicy: "literal",
  },
  {
    id: "literal-script",
    title: "Literal tool marker inside script source",
    content: [
      '<div id="web-app" data-probe="literal-script"><p>Script literal remains local</p>',
      "<script>window.__vcpLiteralToolMarker = '<<<[TOOL_REQUEST]>>>';<\/script>",
      "</div>",
    ].join("\n"),
    requiredTypes: ["html-preview"],
    forbiddenTypes: ["tool-use", "diary"],
    markerPolicy: "none",
  },
  {
    id: "role-and-tool-result",
    title: "Role divider and VCP tool result boundaries",
    content: [
      "Visible before divider",
      "<<<[ROLE_DIVIDE_USER]>>>",
      "User partition",
      "<<<[END_ROLE_DIVIDE_USER]>>>",
      "[[VCP调用结果信息汇总:",
      "- 工具名称: DailyNote",
      "- 执行状态: SUCCESS",
      "- 返回内容: Probe saved",
      "VCP调用结果结束]]",
      "Visible after result",
    ].join("\n"),
    requiredTypes: ["role-divider", "tool-result", "markdown"],
    markerPolicy: "none",
  },
  {
    id: "malformed-html-css",
    title: "Malformed CSS and excess closing tags",
    content:
      '<div id="vcp-root" style="padding:12px;color:rgb(255,0,0;background:#111"><section data-probe="malformed-visible">Malformed content survives</section></div></div></section>',
    requiredTypes: ["markdown"],
    markerPolicy: "none",
  },
  {
    id: "active-html-actions",
    title: "Local controls versus explicit AI send action",
    content: `
      <div id="web-app" data-probe="action-frame" style="padding:12px;color:#111;background:#fff">
        <button id="local-toggle" type="button" onclick="probeToggle()">Show or hide image</button>
        <div id="cursor-card" role="button" style="cursor:pointer;padding:12px" onclick="probeCard()">Cursor card local action</div>
        <button id="plain-button" type="button">Plain inert button</button>
        <button id="ai-send" type="button" data-vcp-send="explicit runtime action">Explicit AI action</button>
        <div id="toggle-target" hidden>LOCAL TARGET VISIBLE</div>
        <div id="local-count">0</div>
        <script>
          window.__probeLocalCount = 0;
          function syncProbeCount() {
            document.getElementById('local-count').textContent = String(window.__probeLocalCount);
          }
          function probeToggle() {
            window.__probeLocalCount += 1;
            const target = document.getElementById('toggle-target');
            target.hidden = !target.hidden;
            syncProbeCount();
          }
          function probeCard() {
            window.__probeLocalCount += 1;
            syncProbeCount();
          }
        <\/script>
      </div>
    `,
    requiredTypes: ["html-preview"],
    markerPolicy: "none",
  },
];

const streamBlock = ref<ContentBlock>({
  type: "markdown",
  content:
    '<div id="vcp-root"><section id="probe-stream-scene" data-probe="stream-scene"><span data-vcp-key="stream-copy">stream frame 1</span></section></div>',
});

const parserPassCount = computed(
  () => results.value.filter((result) => parserCasePassed(result)).length,
);

const blockSummary = computed(() =>
  results.value
    .map(
      (result) =>
        `${result.id}:${result.blocks.map((block) => block.type).join(",")}`,
    )
    .join("|"),
);

function isPlainBlock(type: string): boolean {
  return isRenderDocumentBlock(type);
}

function parserCasePassed(result: ProbeResult): boolean {
  const types = result.blocks.map((block) => block.type);
  return (
    !result.error &&
    result.requiredTypes.every((type) => types.includes(type)) &&
    !(result.forbiddenTypes || []).some((type) => types.includes(type))
  );
}

async function compileFixtures() {
  const compiled: ProbeResult[] = [];
  for (const fixture of fixtures) {
    try {
      const blocks = await invoke<ContentBlock[]>("process_message_content", {
        content: fixture.content,
      });
      compiled.push({ ...fixture, blocks });
    } catch (error) {
      compiled.push({ ...fixture, blocks: [], error: String(error) });
    }
  }
  results.value = compiled;
}

async function exerciseStreamingPatch() {
  await nextTick();
  await nextTick();
  const firstScene = document.getElementById("probe-stream-scene");
  if (!firstScene) return;
  streamRenderCount.value = 1;

  for (const frame of [2, 3]) {
    streamBlock.value = {
      type: "markdown",
      content: `<div id="vcp-root"><section id="probe-stream-scene" data-probe="stream-scene"><span data-vcp-key="stream-copy">stream frame ${frame}</span><b data-probe="stream-new-${frame}">new node ${frame}</b></section></div>`,
    };
    await nextTick();
    await nextTick();
    if (
      document
        .getElementById("probe-stream-scene")
        ?.textContent?.includes(`stream frame ${frame}`)
    ) {
      streamRenderCount.value = frame;
    }
  }

  identityPreserved.value =
    firstScene === document.getElementById("probe-stream-scene");
}

onMounted(async () => {
  await compileFixtures();
  await exerciseStreamingPatch();
  await nextTick();
  window.setTimeout(() => {
    probeReady.value = true;
  }, 600);
});

onBeforeUnmount(() => {
  historyStore.sendMessage = originalSendMessage;
});
</script>

<template>
  <main
    class="renderer-probe"
    data-testid="renderer-v2-probe"
    :data-probe-ready="probeReady"
    :data-parser-pass-count="parserPassCount"
    :data-parser-case-count="results.length"
    :data-block-summary="blockSummary"
    :data-ai-action-count="aiActionCount"
    :data-last-ai-action="lastAiAction"
    :data-stream-render-count="streamRenderCount"
    :data-identity-preserved="identityPreserved"
    :data-reload-count="reloadCount"
  >
    <header class="probe-header">
      <h1>Android renderer runtime probe</h1>
      <output data-testid="probe-status">
        parser {{ parserPassCount }}/{{ results.length }}; stream
        {{ streamRenderCount }}; identity {{ identityPreserved }}; AI actions
        {{ aiActionCount }}
      </output>
    </header>

    <section class="probe-case" data-testid="probe-streaming-case">
      <h2>Simulated streaming patch</h2>
      <RenderDocumentBlock
        :block="streamBlock"
        :message-id="messageId"
        source-id="probe-stream"
        streaming
      />
    </section>

    <section
      v-for="(result, caseIndex) in results"
      :key="result.id"
      class="probe-case"
      :data-testid="`probe-case-${result.id}`"
      :data-case-id="result.id"
      :data-parser-pass="parserCasePassed(result)"
      :data-block-types="result.blocks.map((block) => block.type).join(',')"
      :data-marker-policy="result.markerPolicy"
    >
      <h2>{{ caseIndex + 1 }}. {{ result.title }}</h2>
      <p v-if="result.error" class="probe-error">{{ result.error }}</p>

      <template
        v-for="(block, blockIndex) in result.blocks"
        :key="`${result.id}-${blockIndex}`"
      >
        <div class="probe-block" :data-block-type="block.type">
          <RenderDocumentBlock
            v-if="isPlainBlock(block.type)"
            :block="block"
            :message-id="`${messageId}-${result.id}`"
            :source-id="`${result.id}-${blockIndex}`"
          />
          <ToolBlock
            v-else-if="
              block.type === 'tool-use' || block.type === 'tool-result'
            "
            :type="block.type"
            :content="block.content"
            :block="block"
            default-expanded
          />
          <ThoughtBlock
            v-else-if="block.type === 'thought'"
            :block="block"
            :message-id="`${messageId}-${result.id}`"
            :source-id="`${result.id}-${blockIndex}`"
            :default-expanded="false"
          />
          <HtmlPreviewBlock
            v-else-if="block.type === 'html-preview'"
            :content="block.content || ''"
            :highlighted-content="block.highlighted_content"
            :message-id="`${messageId}-${result.id}`"
          />
          <ToolSummaryBlock
            v-else-if="block.type === 'tool-call-summary'"
            :block="block"
          />
          <pre v-else class="probe-error">
Unhandled block: {{ block.type }}</pre
          >
        </div>
      </template>
    </section>
  </main>
</template>

<style scoped>
.renderer-probe {
  min-height: 100%;
  overflow: auto;
  padding: 16px;
  background: #f4f7fb;
  color: #18212f;
}

.probe-header {
  position: sticky;
  top: 0;
  z-index: 5;
  padding: 12px 0;
  background: #f4f7fb;
  border-bottom: 1px solid #cad3df;
}

.probe-header h1,
.probe-case h2 {
  margin: 0;
  letter-spacing: 0;
}

.probe-header h1 {
  font-size: 18px;
}

.probe-header output {
  display: block;
  margin-top: 6px;
  font: 12px/1.4 monospace;
}

.probe-case {
  padding: 14px 0;
  border-bottom: 1px solid #d7dee8;
}

.probe-case h2 {
  margin-bottom: 10px;
  font-size: 14px;
}

.probe-block {
  min-width: 0;
}

.probe-error {
  color: #b42318;
  white-space: pre-wrap;
}
</style>
