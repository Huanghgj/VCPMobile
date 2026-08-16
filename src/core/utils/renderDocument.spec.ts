// @vitest-environment jsdom

import { beforeEach, describe, expect, it } from "vitest";
import type { ContentBlock } from "../types/chat";
import {
  blockContainsRichHtml,
  blockUsesFramelessLayout,
  compileRenderFragment,
  createRenderDocument,
  RENDER_DOCUMENT_VERSION,
} from "./renderDocument";
import { renderMarkdownNodesToHtml } from "./astRenderer";
import { patchRenderDocumentRoot } from "./renderDomExecutor";

describe("Renderer V2 document compiler", () => {
  it("anchors each copy control inside its own code surface", () => {
    const compiled = compileRenderFragment(
      {
        type: "markdown",
        nodes: [
          {
            type: "raw_html",
            content: '<div id="vcp-root"><div class="generated-layout">',
          },
          {
            type: "code_block",
            lang: "text",
            code: "copy me\n",
            highlighted_html:
              '<pre class="vcp-code-block vcp-scrollable"><code>copy me</code></pre>',
          },
          { type: "raw_html", content: "</div></div>" },
        ],
      },
      "message-code-copy-anchor",
    );

    const template = document.createElement("template");
    template.innerHTML = compiled.html;
    const pre = template.content.querySelector("pre.vcp-code-shell");
    const button = template.content.querySelector("[data-vcp-copy-code]");

    expect(pre).not.toBeNull();
    expect(button?.closest("pre")).toBe(pre);
    expect(button?.closest(".generated-layout")).not.toBeNull();
    expect(button?.getAttribute("data-vcp-copy-code")).toBe("copy%20me%0A");
  });

  it("treats active HTML previews as full-width rich content", () => {
    expect(
      blockContainsRichHtml({
        type: "html-preview",
        content: "<canvas></canvas><script>void 0</script>",
      }),
    ).toBe(true);
  });

  it("uses frameless bubbles only for generated roots and active previews", () => {
    expect(
      blockUsesFramelessLayout({
        type: "markdown",
        content: '<div style="color:#222">styled fragment</div>',
      }),
    ).toBe(false);
    expect(
      blockUsesFramelessLayout({
        type: "markdown",
        content: '<div id="vcp-root">generated document</div>',
      }),
    ).toBe(true);
    expect(
      blockUsesFramelessLayout({
        type: "markdown",
        content: "<DIV ID=vcp-root>generated document</DIV>",
      }),
    ).toBe(true);
    expect(
      blockUsesFramelessLayout({
        type: "html-preview",
        content: "<canvas></canvas>",
      }),
    ).toBe(true);
  });

  beforeEach(() => {
    document.head.innerHTML = "";
    document.body.innerHTML = "";
  });

  it("normalizes a generated vcp root without capturing trailing content", () => {
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
    const root = template.content.querySelector<HTMLElement>(
      "[data-vcp-generated-root]",
    );

    expect(compiled.version).toBe(RENDER_DOCUMENT_VERSION);
    expect(template.content.querySelector("#vcp-root")).toBeNull();
    expect(root?.getAttribute("data-vcp-render-version")).toBe(
      String(RENDER_DOCUMENT_VERSION),
    );
    expect(root?.querySelector("#first")?.textContent).toBe("first");
    expect(template.content.querySelector("#second")?.textContent).toBe(
      "second",
    );
    expect(template.content.querySelector("#poster")).not.toBeNull();
    expect(
      template.content
        .querySelector("#second span")
        ?.getAttribute("data-vcp-render-key"),
    ).toMatch(new RegExp(`^v${RENDER_DOCUMENT_VERSION}-[\\w-]+-`));
    expect(compiled.html).not.toContain("<style");
    expect(compiled.css).toContain("@keyframes blurIn");
  });

  it("preserves the tail marker of a large completed vcp document", () => {
    const marker = "VCP_BROWSER_RENDER_TAIL";
    const compiled = compileRenderFragment(
      {
        type: "markdown",
        content: `<div id="vcp-root"><p>${"x".repeat(512 * 1024)}${marker}</p></div>`,
      },
      "message-large-vcp-root",
    );

    expect(compiled.html).toContain(marker);
  });

  it("drops an empty hidden placeholder root and keeps a later rich root visible", () => {
    const compiled = compileRenderFragment(
      {
        type: "markdown",
        content: [
          '<div id="vcp-root" style="display:none"></div>',
          "\n\nprotocol text\n\n",
          '<div id="vcp-root" style="display:block"><p data-probe="final">final body</p></div>',
        ].join(""),
      },
      "message-double-root",
    );
    const template = document.createElement("template");
    template.innerHTML = compiled.html;

    expect(template.content.querySelector("#vcp-root")).toBeNull();
    expect(
      template.content.querySelectorAll("[data-vcp-generated-root]"),
    ).toHaveLength(1);
    const finalBody = template.content.querySelector<HTMLElement>(
      '[data-probe="final"]',
    );
    expect(finalBody?.textContent).toBe("final body");
    expect(
      finalBody?.closest<HTMLElement>("[data-vcp-generated-root]")?.style
        .display,
    ).toBe("block");
    expect(compiled.html).not.toContain("display:none");
  });

  it("unwraps a hidden generated root when repair output nested a later root", () => {
    const compiled = compileRenderFragment(
      {
        type: "markdown",
        content:
          '<div id="vcp-root" style="display:none">' +
          '<div id="vcp-root"><p data-probe="nested-final">visible</p></div>' +
          "</div>",
      },
      "message-nested-double-root",
    );
    const template = document.createElement("template");
    template.innerHTML = compiled.html;

    expect(template.content.querySelector("#vcp-root")).toBeNull();
    expect(compiled.html).not.toContain("display:none");
    expect(
      template.content.querySelector('[data-probe="nested-final"]')
        ?.textContent,
    ).toBe("visible");
  });

  it("uses the same versioned IR for stable, streaming and fallback blocks", () => {
    const stable: ContentBlock = { type: "markdown", content: "stable" };
    const tail: ContentBlock = { type: "markdown", content: "tail" };
    const documentV2 = createRenderDocument([stable], tail, "ignored");

    expect(documentV2).toEqual({
      version: RENDER_DOCUMENT_VERSION,
      blocks: [stable],
      tail,
    });

    const fallback = createRenderDocument(undefined, undefined, "fallback");
    expect(fallback.version).toBe(RENDER_DOCUMENT_VERSION);
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

  it("keeps details collapsed by default even when generated HTML requests open", () => {
    const compiled = compileRenderFragment(
      {
        type: "markdown",
        content:
          "<details id='default'><summary>默认</summary>正文</details>" +
          "<details id='expanded' open><summary>展开</summary>正文</details>",
      },
      "message-details",
    );
    const template = document.createElement("template");
    template.innerHTML = compiled.html;

    expect(
      template.content.querySelector<HTMLDetailsElement>("#default")?.open,
    ).toBe(false);
    expect(
      template.content.querySelector<HTMLDetailsElement>("#expanded")?.open,
    ).toBe(false);
  });

  it("preserves inline CSS token boundaries from AST source through DOM", () => {
    const inlineStyle =
      "background:linear-gradient(180deg,#fdf6e9 0%,#fcebd4 40%,#f9e0c0 100%);padding:20px 16px 24px;opacity:1";
    const block: ContentBlock = {
      type: "markdown",
      nodes: [
        {
          type: "raw_html",
          content: `<div id="vcp-root" style="${inlineStyle}">visible</div>`,
        },
      ],
    };

    const astHtml = renderMarkdownNodesToHtml(block.nodes!, "message-css");
    expect(astHtml).toContain(`style="${inlineStyle}"`);

    const compiled = compileRenderFragment(block, "message-css");
    const template = document.createElement("template");
    template.innerHTML = compiled.html;
    const root = template.content.querySelector<HTMLElement>(
      "[data-vcp-generated-root]",
    );

    expect(compiled.html).toContain("#fdf6e9 0%");
    expect(compiled.html).toContain("padding:20px 16px 24px");
    expect(root?.style.background).toContain("linear-gradient");
    expect(root?.style.padding).toBe("20px 16px 24px");
    expect(root?.style.opacity).toBe("1");
    expect(root?.hasAttribute("data-vcp-style-repaired")).toBe(false);
  });

  it("keeps normalized roots addressable by the fallback surface selector", () => {
    const compiled = compileRenderFragment(
      {
        type: "markdown",
        content:
          '<div id="vcp-root" style="padding:1.5em;color:#2b2b2b">visible</div>',
      },
      "message-fallback-surface",
    );
    const host = document.createElement("div");
    host.className = "vcp-rich-html-block";
    host.innerHTML = compiled.html;
    document.body.appendChild(host);

    const root = host.querySelector<HTMLElement>("[data-vcp-generated-root]");
    expect(root).not.toBeNull();
    expect(root?.style.background).toBe("");
    expect(
      root?.matches(".vcp-rich-html-block > [data-vcp-generated-root]"),
    ).toBe(true);
    expect(root?.dataset.vcpFallbackSurface).toBe("light");
  });

  it("selects a dark fallback surface for unrepairable backgrounds with light text", () => {
    const compiled = compileRenderFragment(
      {
        type: "markdown",
        content:
          '<div id="vcp-root" style="background:linear-gradient(not-a-gradient);color:#e6ddd4">visible</div>',
      },
      "message-dark-fallback",
    );
    const template = document.createElement("template");
    template.innerHTML = compiled.html;
    const root = template.content.querySelector<HTMLElement>(
      "[data-vcp-generated-root]",
    );

    expect(root?.style.background).toBe("");
    expect(root?.dataset.vcpFallbackSurface).toBe("dark");
  });

  it("repairs validated missing CSS token boundaries", () => {
    const compiled = compileRenderFragment(
      {
        type: "markdown",
        content:
          '<div id="vcp-root" style="background:linear-gradient(180deg,#fdf6e90%,#fcebd4 40%,#f9e0c0 100%);padding:20px16px 24px">visible</div>',
      },
      "message-css-repair",
    );
    const template = document.createElement("template");
    template.innerHTML = compiled.html;
    const root = template.content.querySelector<HTMLElement>(
      "[data-vcp-generated-root]",
    );

    expect(root?.style.background).toContain("linear-gradient");
    expect(root?.style.background).toContain("0%");
    expect(root?.style.padding).toBe("20px 16px 24px");
    expect(root?.hasAttribute("data-vcp-style-repaired")).toBe(true);
  });

  it("repairs the dark gradient stops from the reported mobile response", () => {
    const compiled = compileRenderFragment(
      {
        type: "markdown",
        content:
          '<div id="vcp-root" style="background:linear-gradient(180deg,#0f0f180%,#1a1525100%);color:#e6ddd4">visible</div>',
      },
      "message-reported-gradient",
    );
    const template = document.createElement("template");
    template.innerHTML = compiled.html;
    const root = template.content.querySelector<HTMLElement>(
      "[data-vcp-generated-root]",
    );

    expect(compiled.html).toContain("#0f0f18 0%");
    expect(compiled.html).toContain("#1a1525 100%");
    expect(root?.style.background).toContain("linear-gradient");
    expect(root?.hasAttribute("data-vcp-style-repaired")).toBe(true);
  });

  it("repairs a legacy AST paragraph wrapper after the full response closes", () => {
    const legacyBlock: ContentBlock = {
      type: "markdown",
      nodes: [
        {
          type: "raw_html",
          content: '<div id="vcp-root">',
        },
        {
          type: "raw_html",
          content:
            '<p data-testid="promoted-wrapper" style="padding:16px;border-left:2px solid #5a4a6a">',
        },
        {
          type: "paragraph",
          children: [{ type: "text", value: "First paragraph" }],
        },
        {
          type: "paragraph",
          children: [{ type: "text", value: "Second paragraph" }],
        },
        { type: "raw_html", content: "</p>" },
        { type: "raw_html", content: "</div>" },
      ],
    };
    const streaming = compileRenderFragment(
      legacyBlock,
      "message-promoted-wrapper",
      { final: false },
    );
    const compiled = compileRenderFragment(
      legacyBlock,
      "message-promoted-wrapper",
    );
    const template = document.createElement("template");
    template.innerHTML = compiled.html;
    const wrapper = template.content.querySelector<HTMLElement>(
      '[data-testid="promoted-wrapper"]',
    );

    expect(wrapper).not.toBeNull();
    expect(wrapper?.tagName).toBe("DIV");
    expect(wrapper?.hasAttribute("data-vcp-paragraph-promoted")).toBe(true);
    expect(wrapper?.querySelectorAll(":scope > p")).toHaveLength(2);
    expect(wrapper?.textContent).toContain("First paragraph");
    expect(wrapper?.textContent).toContain("Second paragraph");
    expect(wrapper?.style.borderLeft).toContain("2px");
    expect(streaming.html).not.toContain("data-vcp-paragraph-promoted");
    expect(streaming.signature).toContain(":stream:");
    expect(compiled.signature).toContain(":final:");
    expect(streaming.signature).not.toBe(compiled.signature);
  });

  it("does not promote a closed paragraph containing only inline content", () => {
    const compiled = compileRenderFragment(
      {
        type: "markdown",
        nodes: [
          {
            type: "raw_html",
            content:
              '<p data-testid="inline-paragraph" style="color:red"><span>inline content</span></p>',
          },
        ],
      },
      "message-inline-paragraph",
    );
    const template = document.createElement("template");
    template.innerHTML = compiled.html;
    const paragraph = template.content.querySelector<HTMLElement>(
      '[data-testid="inline-paragraph"]',
    );

    expect(paragraph?.tagName).toBe("P");
    expect(paragraph?.hasAttribute("data-vcp-paragraph-promoted")).toBe(false);
  });

  it("ignores paragraph-shaped text inside generated style blocks", () => {
    const compiled = compileRenderFragment(
      {
        type: "markdown",
        nodes: [
          { type: "raw_html", content: '<div id="vcp-root">' },
          {
            type: "raw_html",
            content:
              '<style>.literal::before{content:"<p><div></div></p>"}</style>',
          },
          {
            type: "raw_html",
            content: '<p data-testid="legacy-wrapper">',
          },
          {
            type: "paragraph",
            children: [{ type: "text", value: "wrapped" }],
          },
          { type: "raw_html", content: "</p></div>" },
        ],
      },
      "message-style-literal",
    );

    expect(compiled.css).toContain('content:"<p><div></div></p>"');
    expect(compiled.html).toContain("data-vcp-paragraph-promoted");
  });

  it("leaves valid, custom and ambiguous CSS values untouched", () => {
    const style =
      "--poster-size:20px16px;padding:20px 16px;color:#ffffff;outline-color:#ffffff%";
    const compiled = compileRenderFragment(
      {
        type: "markdown",
        content: `<div id="vcp-root" style="${style}">visible</div>`,
      },
      "message-css-no-repair",
    );
    const template = document.createElement("template");
    template.innerHTML = compiled.html;
    const root = template.content.querySelector<HTMLElement>(
      "[data-vcp-generated-root]",
    );

    expect(root?.style.getPropertyValue("--poster-size")).toBe("20px16px");
    expect(root?.style.padding).toBe("20px 16px");
    expect(root?.getAttribute("style")).toContain("#ffffff%");
    expect(root?.hasAttribute("data-vcp-style-repaired")).toBe(false);
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
