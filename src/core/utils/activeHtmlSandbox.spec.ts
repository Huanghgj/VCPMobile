import { describe, expect, it } from "vitest";
import {
  ACTIVE_HTML_PERMISSIONS,
  ACTIVE_HTML_SANDBOX,
  buildActiveHtmlDocument,
  rewriteGeneratedClipboardButtons,
  rewriteGeneratedMediaSources,
} from "./activeHtmlSandbox";

describe("active HTML sandbox document", () => {
  it("preserves JavaScript, remote iframes and Three.js module imports", () => {
    const source = `<!DOCTYPE html><html><head><title>Probe</title></head><body>
      <canvas id="scene"></canvas>
      <iframe src="https://example.com"></iframe>
      <script type="module">
        import * as THREE from "https://cdn.jsdelivr.net/npm/three/build/three.module.js";
        window.__threeScene = new THREE.Scene();
      </script>
    </body></html>`;

    const result = buildActiveHtmlDocument(source, true, "nonce-1");

    expect(result).toContain('<script type="module">');
    expect(result).toContain(
      "https://cdn.jsdelivr.net/npm/three/build/three.module.js",
    );
    expect(result).toContain('<iframe src="https://example.com"></iframe>');
    expect(result).toContain("window.__threeScene = new THREE.Scene()");
    expect(result).toContain("data-vcp-preview-bridge");
  });

  it("wraps active HTML fragments without rewriting their executable content", () => {
    const source =
      '<button onclick="window.__clicked = true">Run</button><script>window.__ran = true</script>';
    const result = buildActiveHtmlDocument(source, false, "nonce-2");

    expect(result).toMatch(/^<!DOCTYPE html><html><head>/);
    expect(result).toContain(source);
    expect(result).toContain("<body>");
  });

  it("repairs a generated copy button whose quoted text breaks onclick HTML", () => {
    const copied = `<推进>\n例：聊天时说"小姨下周要来我们家玩"\n</推进>`;
    const source = `<button onclick="
      const text = \`${copied}\`;
      navigator.clipboard.writeText(text).then(() => { this.innerText = '✓ 已复制'; });
    " style="position:absolute">📋 复制</button>`;

    const result = rewriteGeneratedClipboardButtons(source);
    const encoded = result.match(/data-vcp-copy-code="([^"]*)"/)?.[1];

    expect(result).not.toContain("onclick=");
    expect(result).toContain('data-vcp-copy-encoded="uri"');
    expect(result).toContain('style="position:absolute"');
    expect(decodeURIComponent(encoded || "")).toBe(copied);
  });

  it("rewrites direct clipboard literals but leaves unrelated handlers intact", () => {
    const copyButton = `<button onclick="navigator.clipboard.writeText('copy me')">Copy</button>`;
    const unrelated = `<button onclick="window.__clicked = true">Run</button>`;

    expect(rewriteGeneratedClipboardButtons(copyButton)).toContain(
      'data-vcp-copy-code="copy%20me"',
    );
    expect(rewriteGeneratedClipboardButtons(unrelated)).toBe(unrelated);
  });

  it("maps relative generated media names to registered attachment URLs", () => {
    const source = `<video controls><source src="e322d33d.mp4" type="video/mp4"></video>`;
    const result = rewriteGeneratedMediaSources(source, {
      "e322d33d.mp4":
        "http://asset.localhost/video%20file.mp4?token=1&source=chat",
    });

    expect(result).toContain(
      'src="http://asset.localhost/video%20file.mp4?token=1&amp;source=chat"',
    );
  });

  it("does not replace remote media or unrelated relative files", () => {
    const source =
      '<video src="https://example.com/clip.mp4"></video><audio src="other.mp3"></audio>';
    expect(
      rewriteGeneratedMediaSources(source, {
        "clip.mp4": "asset://local/clip.mp4",
      }),
    ).toBe(source);
  });

  it("emits a syntactically valid bridge script", () => {
    const result = buildActiveHtmlDocument(
      "<div>probe</div>",
      false,
      "nonce-js",
    );
    const bridge = result.match(
      /<script data-vcp-preview-bridge>([\s\S]*?)<\/script>/,
    )?.[1];

    expect(bridge).toBeTruthy();
    expect(() => new Function(bridge || "")).not.toThrow();
  });

  it("keeps the requested capabilities while omitting same-origin access", () => {
    expect(ACTIVE_HTML_SANDBOX).toContain("allow-scripts");
    expect(ACTIVE_HTML_SANDBOX).toContain("allow-pointer-lock");
    expect(ACTIVE_HTML_SANDBOX).not.toContain("allow-same-origin");
    expect(ACTIVE_HTML_PERMISSIONS).toContain("fullscreen");
    expect(ACTIVE_HTML_PERMISSIONS).toContain("xr-spatial-tracking");
  });

  it("installs full-height, visibility and authenticated action bridges", () => {
    const result = buildActiveHtmlDocument(
      "<details><summary>题目</summary>答案</details><button>继续</button>",
      false,
      "nonce-visible",
    );

    expect(result).toContain("overflow-y: hidden !important");
    expect(result).toContain("new ResizeObserver");
    expect(result).toContain("post('render-size'");
    expect(result).not.toContain("post('render-scroll'");
    expect(result).not.toContain("document.addEventListener('touchmove'");
    expect(result).toContain("Math.abs(nextHeight - lastMeasuredHeight)");
    expect(result).toContain("touch-action: pan-y pinch-zoom");
    expect(result).toContain("overscroll-behavior: auto");
    expect(result).not.toContain("image.loading = 'lazy'");
    expect(result).toContain("document.getAnimations()");
    expect(result).toContain("effectTarget.element instanceof Element");
    expect(result).toContain("new MutationObserver(handleMutations)");
    expect(result).not.toContain("document.querySelectorAll('*')");
    expect(result).not.toContain("attributes: true");
    expect(result).not.toContain("attributeFilter: ['class', 'style'");
    expect(result).toContain("event.preventDefault()");
    expect(result).toContain("data.type === 'render-visibility'");
    expect(result).toContain("data.nonce !== nonce");
    expect(result).toContain("post('ai-action'");
    expect(result).toContain("target.closest(actionSelector)");
    expect(result).toContain('const actionSelector = "[data-vcp-send]"');
    expect(result).not.toContain("[style*='cursor:pointer']");
    expect(result).not.toContain('"button,');
    expect(result).toContain("target.closest('[data-vcp-copy-code]')");
    expect(result).toContain("button.setAttribute('aria-disabled', 'true')");
    expect(result).toContain("post('copy-text'");
    expect(result).toContain("data.type === 'copy-result'");
    expect(result).toContain("if (!event.isTrusted) return");
    expect(result).toContain("crypto.randomUUID");
    expect(result).toContain("nativeParentPostMessage");
    expect(result).toContain("post('bridge-ready')");
    expect(result).not.toContain("details:not([data-vcp-collapsed])");
    expect(result).toContain('"nonce-visible"');
  });

  it("closes details during sandbox initialization and subtree insertion", () => {
    const result = buildActiveHtmlDocument(
      '<details id="default"><summary>默认</summary>正文</details>' +
        '<details id="expanded" open><summary>展开</summary>正文</details>',
      false,
      "nonce-details",
    );

    expect(result).toContain(
      "detailsNodes.push(...node.querySelectorAll('details'))",
    );
    expect(result).toContain("details.open = false");
  });
});
