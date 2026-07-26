// @vitest-environment jsdom

import { createApp, defineComponent, h, nextTick, ref } from "vue";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ContentBlock } from "../types/chat";
import RenderDocumentBlock from "../../features/chat/components/RenderDocumentBlock.vue";
import { useRenderVisibility } from "./useRenderVisibility";

class IntersectionObserverStub {
  static instances: IntersectionObserverStub[] = [];
  private target: Element | null = null;

  constructor(private readonly callback: IntersectionObserverCallback) {
    IntersectionObserverStub.instances.push(this);
  }

  observe(target: Element) {
    this.target = target;
    this.emit(false);
  }

  unobserve() {}
  disconnect() {}
  takeRecords() {
    return [];
  }

  emit(isIntersecting: boolean) {
    if (!this.target) return;
    this.callback(
      [
        {
          target: this.target,
          isIntersecting,
        } as IntersectionObserverEntry,
      ],
      this as unknown as IntersectionObserver,
    );
  }
}

class ResizeObserverStub {
  constructor(_callback: ResizeObserverCallback) {}
  observe() {}
  unobserve() {}
  disconnect() {}
}

async function flushVue() {
  await nextTick();
  await Promise.resolve();
  await nextTick();
  await Promise.resolve();
  await nextTick();
}

describe("useRenderVisibility", () => {
  const originalRect = HTMLElement.prototype.getBoundingClientRect;

  afterEach(() => {
    IntersectionObserverStub.instances = [];
    vi.unstubAllGlobals();
    HTMLElement.prototype.getBoundingClientRect = originalRect;
    document.body.replaceChildren();
  });

  it("keeps an unmeasured offscreen instance prewarmed", async () => {
    let canMeasure = false;
    let visibility: ReturnType<typeof useRenderVisibility> | undefined;
    HTMLElement.prototype.getBoundingClientRect = function () {
      return {
        height: canMeasure ? 120 : 0,
      } as DOMRect;
    };
    vi.stubGlobal("IntersectionObserver", IntersectionObserverStub);
    vi.stubGlobal("ResizeObserver", ResizeObserverStub);

    const app = createApp(
      defineComponent({
        setup() {
          const target = ref<HTMLElement | null>(null);
          visibility = useRenderVisibility(target);
          return () => h("div", { ref: target }, "probe");
        },
      }),
    );
    const host = document.createElement("div");
    document.body.appendChild(host);
    app.mount(host);
    await flushVue();

    expect(visibility?.state.value).toBe("prewarm");
    expect(visibility?.cachedHeight.value).toBe(0);

    canMeasure = true;
    visibility?.rememberHeight();
    expect(visibility?.cachedHeight.value).toBe(120);
    expect(visibility?.state.value).toBe("parked");
    app.unmount();
  });

  it("renders a first offscreen reply into a stable placeholder before it is visible", async () => {
    HTMLElement.prototype.getBoundingClientRect = function () {
      const hasReply = Boolean(this.querySelector?.("#latest-reply-probe"));
      return {
        height: hasReply ? 120 : 0,
      } as DOMRect;
    };
    vi.stubGlobal("IntersectionObserver", IntersectionObserverStub);
    vi.stubGlobal("ResizeObserver", ResizeObserverStub);
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
      callback(0);
      return 1;
    });
    vi.stubGlobal("cancelAnimationFrame", () => {});

    const block: ContentBlock = {
      type: "markdown",
      content:
        '<section id="latest-reply-probe">Latest reply renders itself</section>',
    };
    const app = createApp(
      defineComponent({
        setup() {
          return () =>
            h(RenderDocumentBlock, {
              block,
              messageId: "first-offscreen-reply",
              sourceId: "first-offscreen-reply-block",
            });
        },
      }),
    );
    const host = document.createElement("div");
    document.body.appendChild(host);
    app.mount(host);
    await flushVue();

    const root = host.querySelector<HTMLElement>(".vcp-render-document");
    expect(root?.dataset.vcpRenderSignature).toBeTruthy();
    expect(root?.style.height).toBe("120px");
    expect(root?.children.length).toBe(0);

    IntersectionObserverStub.instances.forEach((observer) =>
      observer.emit(true),
    );
    await flushVue();
    expect(host.querySelector("#latest-reply-probe")?.textContent).toBe(
      "Latest reply renders itself",
    );
    app.unmount();
  });

  it("uses markstream for a growing plain markdown tail inside the app host", async () => {
    await import("markstream-vue");
    vi.stubGlobal("IntersectionObserver", IntersectionObserverStub);
    vi.stubGlobal("ResizeObserver", ResizeObserverStub);
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
      callback(0);
      return 1;
    });
    vi.stubGlobal("cancelAnimationFrame", () => {});

    const block = ref<ContentBlock>({
      type: "markdown",
      content: "## Streaming title\n\npartial **bold",
    });
    const app = createApp(
      defineComponent({
        setup() {
          return () =>
            h(RenderDocumentBlock, {
              block: block.value,
              messageId: "streaming-markdown",
              sourceId: "stream-tail",
              streaming: true,
            });
        },
      }),
    );
    const host = document.createElement("div");
    document.body.appendChild(host);
    app.mount(host);
    await flushVue();
    await new Promise((resolve) => setTimeout(resolve, 20));
    await flushVue();

    expect(host.querySelector("[data-vcp-render-host]")).not.toBeNull();
    expect(host.querySelector(".markstream-vue")).not.toBeNull();
    expect(host.textContent).toContain("Streaming title");

    block.value = {
      type: "markdown",
      content: "## Streaming title\n\npartial **bold** complete",
    };
    await flushVue();
    expect(host.textContent).toContain("complete");
    app.unmount();
  });

  it("keeps streaming rich HTML on the sanitized document renderer", async () => {
    vi.stubGlobal("IntersectionObserver", IntersectionObserverStub);
    vi.stubGlobal("ResizeObserver", ResizeObserverStub);

    const app = createApp(
      defineComponent({
        setup() {
          return () =>
            h(RenderDocumentBlock, {
              block: {
                type: "markdown",
                content:
                  '<div id="vcp-root"><p data-probe="rich-stream">rich</p></div>',
              } satisfies ContentBlock,
              messageId: "streaming-rich-html",
              sourceId: "stream-tail",
              streaming: true,
            });
        },
      }),
    );
    const host = document.createElement("div");
    document.body.appendChild(host);
    app.mount(host);
    await flushVue();

    expect(host.querySelector(".markstream-vue")).toBeNull();
    expect(host.querySelector('[data-probe="rich-stream"]')?.textContent).toBe(
      "rich",
    );
    expect(host.querySelector("#vcp-root")).toBeNull();
    app.unmount();
  });
});
