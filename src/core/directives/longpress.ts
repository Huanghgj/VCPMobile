import type { DirectiveBinding, ObjectDirective } from "vue";

const LONG_PRESS_DELAY_MS = 600;
const MOVE_TOLERANCE_PX = 10;

type LongPressCallback = (event: Event) => void;

type LongPressState = {
  callback: LongPressCallback;
  timer: number | null;
  suppressResetTimer: number | null;
  pointerId: number | null;
  startX: number;
  startY: number;
  suppressNextClick: boolean;
  cleanup: () => void;
};

const states = new WeakMap<HTMLElement, LongPressState>();

function callbackFromBinding(binding: DirectiveBinding): LongPressCallback | null {
  if (typeof binding.value !== "function") {
    console.warn("v-longpress requires a function value");
    return null;
  }
  return binding.value as LongPressCallback;
}

export const vLongpress: ObjectDirective<HTMLElement, LongPressCallback> = {
  mounted(el, binding) {
    const callback = callbackFromBinding(binding);
    if (!callback) return;

    const state: LongPressState = {
      callback,
      timer: null,
      suppressResetTimer: null,
      pointerId: null,
      startX: 0,
      startY: 0,
      suppressNextClick: false,
      cleanup: () => undefined,
    };

    const clearPressTimer = () => {
      if (state.timer !== null) {
        window.clearTimeout(state.timer);
        state.timer = null;
      }
    };

    const armClickSuppression = () => {
      state.suppressNextClick = true;
      if (state.suppressResetTimer !== null) {
        window.clearTimeout(state.suppressResetTimer);
      }
      state.suppressResetTimer = window.setTimeout(() => {
        state.suppressNextClick = false;
        state.suppressResetTimer = null;
      }, 1000);
    };

    const onPointerDown = (event: PointerEvent) => {
      if (!event.isPrimary || (event.pointerType === "mouse" && event.button !== 0)) {
        return;
      }

      clearPressTimer();
      state.pointerId = event.pointerId;
      state.startX = event.clientX;
      state.startY = event.clientY;
      state.timer = window.setTimeout(() => {
        state.timer = null;
        armClickSuppression();
        state.callback(event);
      }, LONG_PRESS_DELAY_MS);
    };

    const onPointerMove = (event: PointerEvent) => {
      if (event.pointerId !== state.pointerId || state.timer === null) return;
      const distance = Math.hypot(
        event.clientX - state.startX,
        event.clientY - state.startY,
      );
      if (distance > MOVE_TOLERANCE_PX) {
        clearPressTimer();
        state.pointerId = null;
      }
    };

    const onPointerEnd = (event: PointerEvent) => {
      if (event.pointerId !== state.pointerId) return;
      clearPressTimer();
      state.pointerId = null;
    };

    const onClickCapture = (event: MouseEvent) => {
      if (!state.suppressNextClick) return;
      state.suppressNextClick = false;
      if (state.suppressResetTimer !== null) {
        window.clearTimeout(state.suppressResetTimer);
        state.suppressResetTimer = null;
      }
      event.preventDefault();
      event.stopImmediatePropagation();
    };

    const onContextMenu = (event: MouseEvent) => {
      event.preventDefault();
      clearPressTimer();
      state.pointerId = null;
      armClickSuppression();
      state.callback(event);
    };

    el.addEventListener("pointerdown", onPointerDown);
    el.addEventListener("pointermove", onPointerMove);
    el.addEventListener("pointerup", onPointerEnd);
    el.addEventListener("pointercancel", onPointerEnd);
    el.addEventListener("click", onClickCapture, true);
    el.addEventListener("contextmenu", onContextMenu);

    state.cleanup = () => {
      clearPressTimer();
      if (state.suppressResetTimer !== null) {
        window.clearTimeout(state.suppressResetTimer);
      }
      el.removeEventListener("pointerdown", onPointerDown);
      el.removeEventListener("pointermove", onPointerMove);
      el.removeEventListener("pointerup", onPointerEnd);
      el.removeEventListener("pointercancel", onPointerEnd);
      el.removeEventListener("click", onClickCapture, true);
      el.removeEventListener("contextmenu", onContextMenu);
    };

    states.set(el, state);
  },

  updated(el, binding) {
    const callback = callbackFromBinding(binding);
    const state = states.get(el);
    if (callback && state) state.callback = callback;
  },

  unmounted(el) {
    states.get(el)?.cleanup();
    states.delete(el);
  },
};
