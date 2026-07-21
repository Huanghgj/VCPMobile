// @vitest-environment jsdom

import { beforeEach, describe, expect, it } from "vitest";
import type { ContentBlock } from "../types/chat";
import {
  blockContainsRichHtml,
  compileRenderFragment,
  createRenderDocument,
  RENDER_DOCUMENT_VERSION,
} from "./renderDocument";
import { patchRenderDocumentRoot } from "./renderDomExecutor";

describe("Renderer V2 document compiler", () => {
  it("treats active HTML previews as full-width rich content", () => {
    expect(
      blockContainsRichHtml({
        type: "html-preview",
        content: "<canvas></canvas><script>void 0</script>",
      }),
    ).toBe(true);
  });

  beforeEach(() => {
    document.head.innerHTML = "";
    document.body.innerHTML = "";
  });

  it("keeps all trailing rich content inside an prematurely closed vcp root", () => {
    const block: ContentBlock = {
      type: "markdown",
      content: [
        '<div id="vcp-root" style="padding:20px">',
        "<style>@keyframes blurIn { from { opacity:0 } to { opacity:1 } }.scene{animation:blurIn .5s}</style>",
        '<section id="first" class="scene">first</section>',
        "</div>",
        "</div>",
        '<section id="second"><span>second</span></section>',
        '<img id="poster" src="https://example.com/poster.png">',
      ].join(""),
    };

    const compiled = compileRenderFragment(block, "message-rich");
    const template = document.createElement("template");
    template.innerHTML = compiled.html;
    const root = template.content.querySelector("#vcp-root");

    expect(compiled.version).toBe(RENDER_DOCUMENT_VERSION);
    expect(root?.getAttribute("data-vcp-render-version")).toBe("2");
    expect(root?.querySelector("#first")?.textContent).toBe("first");
    expect(root?.querySelector("#second")?.textContent).toBe("second");
    expect(root?.querySelector("#poster")).not.toBeNull();
    expect(
      root?.querySelector("#second span")?.getAttribute("data-vcp-render-key"),
    ).toMatch(/^v2-[\w-]+-/);
    expect(template.content.querySelectorAll(":scope > #second")).toHaveLength(
      0,
    );
    expect(compiled.html).not.toContain("<style");
    expect(compiled.css).toContain("@keyframes blurIn");
  });

  it("uses the same versioned IR for stable, streaming and fallback blocks", () => {
    const stable: ContentBlock = { type: "markdown", content: "stable" };
    const tail: ContentBlock = { type: "markdown", content: "tail" };
    const documentV2 = createRenderDocument([stable], tail, "ignored");

    expect(documentV2).toEqual({
      version: 2,
      blocks: [stable],
      tail,
    });

    const fallback = createRenderDocument(undefined, undefined, "fallback");
    expect(fallback.version).toBe(2);
    expect(fallback.blocks).toEqual([
      { type: "markdown", content: "fallback" },
    ]);
  });

  it("sanitizes active content after HTML5 normalization", () => {
    const compiled = compileRenderFragment(
      {
        type: "markdown",
        content:
          '<div id="vcp-root"><script>alert(1)</script><img src="https://example.com/a.png" onerror="alert(2)"><a href="javascript:alert(3)">bad</a></div>',
      },
      "message-security",
    );

    expect(compiled.html).not.toContain("<script");
    expect(compiled.html).not.toContain("onerror");
    expect(compiled.html).not.toContain("javascript:");
    expect(compiled.html).toContain("https://example.com/a.png");
  });

  it("opens details by default while respecting an explicit collapsed marker", () => {
    const compiled = compileRenderFragment(
      {
        type: "markdown",
        content:
          "<details id='default'><summary>默认</summary>正文</details>" +
          "<details id='collapsed' data-vcp-collapsed><summary>折叠</summary>正文</details>",
      },
      "message-details",
    );
    const template = document.createElement("template");
    template.innerHTML = compiled.html;

    expect(
      template.content.querySelector<HTMLDetailsElement>("#default")?.open,
    ).toBe(true);
    expect(
      template.content.querySelector<HTMLDetailsElement>("#collapsed")?.open,
    ).toBe(false);
  });
});

describe("Renderer V2 DOM executor", () => {
  it("patches stable and streaming frames through one identity-preserving path", () => {
    const root = document.createElement("div");
    patchRenderDocumentRoot(
      root,
      '<section id="scene"><span data-vcp-key="copy">one</span></section>',
    );
    const scene = root.querySelector("#scene");
    const copy = root.querySelector('[data-vcp-key="copy"]');

    patchRenderDocumentRoot(
      root,
      '<section id="scene"><span data-vcp-key="copy">two</span><b id="new">new</b></section>',
    );

    expect(root.querySelector("#scene")).toBe(scene);
    expect(root.querySelector('[data-vcp-key="copy"]')).toBe(copy);
    expect(copy?.textContent).toBe("two");
    expect(root.querySelector("#new")?.textContent).toBe("new");
  });

  it("preserves the user's details state across streaming patches", () => {
    const root = document.createElement("div");
    patchRenderDocumentRoot(
      root,
      '<details id="solution" open><summary>解答</summary><p>第一帧</p></details>',
    );
    const details = root.querySelector<HTMLDetailsElement>("#solution")!;
    details.open = false;

    patchRenderDocumentRoot(
      root,
      '<details id="solution" open><summary>解答</summary><p>第二帧</p></details>',
    );

    expect(root.querySelector("#solution")).toBe(details);
    expect(details.open).toBe(false);
    expect(details.textContent).toContain("第二帧");
  });
});
