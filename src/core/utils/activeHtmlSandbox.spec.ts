import { describe, expect, it } from "vitest";
import {
  ACTIVE_HTML_PERMISSIONS,
  ACTIVE_HTML_SANDBOX,
  buildActiveHtmlDocument,
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

  it("emits a syntactically valid bridge script", () => {
    const result = buildActiveHtmlDocument("<div>probe</div>", false, "nonce-js");
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
    expect(result).toContain("post('render-scroll'");
    expect(result).toContain("Math.abs(totalY) > Math.abs(totalX)");
    expect(result).toContain("Math.abs(nextHeight - lastMeasuredHeight)");
    expect(result).toContain("touch-action: pan-x pinch-zoom");
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
    expect(result).not.toContain("details:not([data-vcp-collapsed])");
    expect(result).toContain('"nonce-visible"');
  });

  it("keeps details collapsed unless the source explicitly opens them", () => {
    const result = buildActiveHtmlDocument(
      '<details id="default"><summary>默认</summary>正文</details>' +
        '<details id="expanded" open><summary>展开</summary>正文</details>',
      false,
      "nonce-details",
    );

    expect(result).toContain('<details id="default">');
    expect(result).toContain('<details id="expanded" open>');
  });
});
