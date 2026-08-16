import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DirectiveBinding } from "vue";
import { vLongpress } from "@/core/directives/longpress";

function binding(callback: (event: Event) => void): DirectiveBinding {
  return { value: callback } as DirectiveBinding;
}

const mountDirective = (el: HTMLElement, callback: (event: Event) => void) => {
  vLongpress.mounted!(el, binding(callback), {} as never, null);
};

const updateDirective = (el: HTMLElement, callback: (event: Event) => void) => {
  vLongpress.updated!(el, binding(callback), {} as never, {} as never);
};

const unmountDirective = (el: HTMLElement) => {
  vLongpress.unmounted!(el, binding(() => undefined), {} as never, null);
};

function pointerEvent(
  type: string,
  options: { pointerId?: number; x?: number; y?: number } = {},
): PointerEvent {
  const event = new MouseEvent(type, {
    bubbles: true,
    cancelable: true,
    clientX: options.x ?? 0,
    clientY: options.y ?? 0,
    button: 0,
  });
  Object.defineProperties(event, {
    isPrimary: { value: true },
    pointerId: { value: options.pointerId ?? 1 },
    pointerType: { value: "touch" },
  });
  return event as PointerEvent;
}

describe("v-longpress", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("tolerates small finger movement but cancels a real drag", () => {
    const el = document.createElement("button");
    const callback = vi.fn();
    mountDirective(el, callback);

    el.dispatchEvent(pointerEvent("pointerdown", { x: 10, y: 10 }));
    el.dispatchEvent(pointerEvent("pointermove", { x: 15, y: 14 }));
    vi.advanceTimersByTime(600);
    expect(callback).toHaveBeenCalledTimes(1);

    el.dispatchEvent(pointerEvent("pointerdown", { pointerId: 2, x: 10, y: 10 }));
    el.dispatchEvent(pointerEvent("pointermove", { pointerId: 2, x: 30, y: 10 }));
    vi.advanceTimersByTime(600);
    expect(callback).toHaveBeenCalledTimes(1);

    unmountDirective(el);
  });

  it("suppresses the synthetic click after a completed long press", () => {
    const el = document.createElement("button");
    const longPress = vi.fn();
    const click = vi.fn();
    el.addEventListener("click", click);
    mountDirective(el, longPress);

    el.dispatchEvent(pointerEvent("pointerdown"));
    vi.advanceTimersByTime(600);
    el.dispatchEvent(pointerEvent("pointerup"));
    const clickEvent = new MouseEvent("click", { bubbles: true, cancelable: true });
    el.dispatchEvent(clickEvent);

    expect(longPress).toHaveBeenCalledTimes(1);
    expect(click).not.toHaveBeenCalled();
    expect(clickEvent.defaultPrevented).toBe(true);

    unmountDirective(el);
  });

  it("uses the latest callback when a virtual-list element is reused", () => {
    const el = document.createElement("button");
    const oldCallback = vi.fn();
    const newCallback = vi.fn();
    mountDirective(el, oldCallback);
    updateDirective(el, newCallback);

    el.dispatchEvent(pointerEvent("pointerdown"));
    vi.advanceTimersByTime(600);

    expect(oldCallback).not.toHaveBeenCalled();
    expect(newCallback).toHaveBeenCalledTimes(1);

    unmountDirective(el);
  });
});
