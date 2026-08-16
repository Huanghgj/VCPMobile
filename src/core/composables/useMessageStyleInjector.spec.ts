// @vitest-environment jsdom

import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  scopeMessageCss,
  useMessageStyleInjector,
} from "./useMessageStyleInjector";

describe("Renderer V2 CSS compiler", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    document.head.innerHTML = "";
  });

  it("scopes selectors and namespaces keyframes without corrupting from/to", () => {
    const compiled = scopeMessageCss(
      [
        '@import url("https://example.com/evil.css");',
        "@keyframes blurIn { from { opacity:0 } 50% { opacity:.5 } to { opacity:1 } }",
        "#vcp-root, [id='vcp-root'], body .card:hover { animation:blurIn .5s ease; position:fixed; background-image:url(https://example.com/a.png) }",
        "@media (min-width:320px) { .card { animation-name:blurIn } }",
      ].join("\n"),
      "message-css",
    );

    const keyframeName = compiled.match(/@keyframes\s+([\w-]+)/)?.[1];
    expect(keyframeName).toMatch(/^vcp-[\w-]+-blurIn$/);
    expect(compiled).toContain(`animation:${keyframeName} .5s ease`);
    expect(compiled).toContain(`animation-name:${keyframeName}`);
    expect(compiled).toContain("from{opacity:0}");
    expect(compiled).toContain("to{opacity:1}");
    expect(compiled).toContain(
      '[data-message-id="message-css"] [data-vcp-generated-root]',
    );
    expect(compiled).not.toContain("#vcp-root");
    expect(compiled).not.toContain("[id=vcp-root]");
    expect(compiled).toContain('[data-message-id="message-css"] .card:hover');
    expect(compiled).not.toContain("@import");
    expect(compiled).not.toContain("position:fixed");
    expect(compiled).not.toContain("example.com/a.png");
  });

  it("combines independent block style sources without overwriting either one", () => {
    const { injectScopedCss, removeScopedCss } = useMessageStyleInjector();
    injectScopedCss(".first { color:red }", "message-sources", "first");
    injectScopedCss(".second { color:blue }", "message-sources", "second");

    const style = document.getElementById("style-message-sources");
    expect(style?.textContent).toContain(".first");
    expect(style?.textContent).toContain(".second");

    removeScopedCss("message-sources", "first");
    expect(style?.textContent).not.toContain(".first");
    expect(style?.textContent).toContain(".second");

    removeScopedCss("message-sources", "second");
    vi.advanceTimersByTime(60);
    expect(document.getElementById("style-message-sources")).toBeNull();
  });
});
