// @vitest-environment jsdom

import { computed, nextTick, ref } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useChatScroll } from "./useChatScroll";

let resizeCallback: ResizeObserverCallback | null = null;

class ResizeObserverStub {
  constructor(callback: ResizeObserverCallback) {
    resizeCallback = callback;
  }

  observe() {}
  disconnect() {}
  unobserve() {}
}

async function flushScheduledWork() {
  await vi.runAllTimersAsync();
  await nextTick();
}

describe("useChatScroll", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    resizeCallback = null;
    vi.stubGlobal("ResizeObserver", ResizeObserverStub);
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) =>
      window.setTimeout(() => callback(performance.now()), 0),
    );
    vi.stubGlobal("cancelAnimationFrame", (id: number) => clearTimeout(id));
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.useRealTimers();
    document.body.replaceChildren();
  });

  it("stops following before a user drag can race a content resize", async () => {
    let contentHeight = 1_800;
    const viewportHeight = 600;
    const list = document.createElement("div");
    const inner = document.createElement("div");
    inner.className = "messages-inner-container";
    list.appendChild(inner);
    document.body.appendChild(list);

    Object.defineProperties(list, {
      scrollHeight: { get: () => contentHeight },
      clientHeight: { get: () => viewportHeight },
    });
    Object.defineProperty(list, "scrollTo", {
      value: ({ top }: ScrollToOptions) => {
        list.scrollTop = Math.min(
          Number(top) || 0,
          Math.max(0, contentHeight - viewportHeight),
        );
        list.dispatchEvent(new Event("scroll"));
      },
    });

    const messageListRef = ref<HTMLElement | null>(null);
    const messageCount = ref(1);
    const controller = useChatScroll({
      messageListRef,
      messageCount: computed(() => messageCount.value),
      hasMoreHistory: ref(false),
      isLoadingHistory: ref(false),
      onLoadMore: vi.fn(),
    });

    messageListRef.value = list;
    await nextTick();
    resizeCallback?.([], {} as ResizeObserver);
    await flushScheduledWork();
    expect(list.scrollTop).toBe(1_200);

    list.scrollTop = 1_120;
    list.dispatchEvent(new Event("touchstart"));
    contentHeight = 2_400;
    resizeCallback?.([], {} as ResizeObserver);
    await flushScheduledWork();
    expect(list.scrollTop).toBe(1_120);

    list.dispatchEvent(new Event("touchend"));
    await flushScheduledWork();
    contentHeight = 2_700;
    resizeCallback?.([], {} as ResizeObserver);
    await flushScheduledWork();
    expect(list.scrollTop).toBe(1_120);

    controller.scrollToBottom(false);
    expect(list.scrollTop).toBe(2_100);
    contentHeight = 3_000;
    resizeCallback?.([], {} as ResizeObserver);
    await flushScheduledWork();
    expect(list.scrollTop).toBe(2_400);

    controller.dispose();
  });

  it("keeps a short upward iframe-origin scroll detached from the bottom", async () => {
    let contentHeight = 1_800;
    const viewportHeight = 600;
    const list = document.createElement("div");
    const inner = document.createElement("div");
    inner.className = "messages-inner-container";
    list.appendChild(inner);
    document.body.appendChild(list);

    Object.defineProperties(list, {
      scrollHeight: { get: () => contentHeight },
      clientHeight: { get: () => viewportHeight },
    });
    Object.defineProperty(list, "scrollTo", {
      value: ({ top }: ScrollToOptions) => {
        list.scrollTop = Math.min(
          Number(top) || 0,
          Math.max(0, contentHeight - viewportHeight),
        );
        list.dispatchEvent(new Event("scroll"));
      },
    });

    const messageListRef = ref<HTMLElement | null>(null);
    const controller = useChatScroll({
      messageListRef,
      messageCount: computed(() => 1),
      hasMoreHistory: ref(false),
      isLoadingHistory: ref(false),
      onLoadMore: vi.fn(),
    });

    messageListRef.value = list;
    await nextTick();
    resizeCallback?.([], {} as ResizeObserver);
    await flushScheduledWork();
    expect(list.scrollTop).toBe(1_200);

    // A gesture that starts inside an iframe does not bubble touchstart to the
    // parent. The first parent signal is the native scroll event, and 80px is
    // deliberately still inside the old 150px "near bottom" threshold.
    list.scrollTop = 1_120;
    list.dispatchEvent(new Event("scroll"));
    await flushScheduledWork();

    contentHeight = 2_400;
    resizeCallback?.([], {} as ResizeObserver);
    await flushScheduledWork();
    expect(list.scrollTop).toBe(1_120);

    controller.dispose();
  });
});
