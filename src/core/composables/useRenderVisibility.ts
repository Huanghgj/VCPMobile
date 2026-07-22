import {
  computed,
  onMounted,
  onUnmounted,
  ref,
  type Ref,
} from "vue";

export type RenderVisibilityState = "visible" | "prewarm" | "parked";

export function useRenderVisibility(
  target: Ref<HTMLElement | null>,
  initialHeight = 0,
) {
  const isVisible = ref(true);
  const isNearViewport = ref(true);
  const isForeground = ref(
    typeof document === "undefined" ? true : !document.hidden,
  );
  const cachedHeight = ref(Math.max(0, initialHeight));

  const state = computed<RenderVisibilityState>(() => {
    if (!isForeground.value) return "parked";
    if (isVisible.value) return "visible";
    return isNearViewport.value ? "prewarm" : "parked";
  });

  let visibleObserver: IntersectionObserver | null = null;
  let prewarmObserver: IntersectionObserver | null = null;
  let resizeObserver: ResizeObserver | null = null;

  const rememberHeight = () => {
    const element = target.value;
    if (!element || state.value === "parked") return;
    const height = Math.ceil(element.getBoundingClientRect().height);
    if (height > 0) cachedHeight.value = height;
  };

  const handleVisibilityChange = () => {
    isForeground.value = !document.hidden;
  };

  const handleLifecycle = (event: Event) => {
    const lifecycleState = (event as CustomEvent).detail?.state;
    if (lifecycleState === "pause" || lifecycleState === "stop") {
      isForeground.value = false;
    } else if (lifecycleState === "resume") {
      isForeground.value = true;
    }
  };

  onMounted(() => {
    const element = target.value;
    if (!element) return;

    if (typeof IntersectionObserver !== "undefined") {
      visibleObserver = new IntersectionObserver(
        ([entry]) => {
          isVisible.value = Boolean(entry?.isIntersecting);
        },
        { threshold: 0 },
      );
      prewarmObserver = new IntersectionObserver(
        ([entry]) => {
          isNearViewport.value = Boolean(entry?.isIntersecting);
        },
        { threshold: 0, rootMargin: "75% 0px" },
      );
      visibleObserver.observe(element);
      prewarmObserver.observe(element);
    }

    if (typeof ResizeObserver !== "undefined") {
      resizeObserver = new ResizeObserver(rememberHeight);
      resizeObserver.observe(element);
    }
    rememberHeight();
    document.addEventListener("visibilitychange", handleVisibilityChange);
    window.addEventListener("vcp-lifecycle", handleLifecycle);
  });

  onUnmounted(() => {
    visibleObserver?.disconnect();
    prewarmObserver?.disconnect();
    resizeObserver?.disconnect();
    document.removeEventListener("visibilitychange", handleVisibilityChange);
    window.removeEventListener("vcp-lifecycle", handleLifecycle);
  });

  return {
    state,
    cachedHeight,
    isVisible,
    isForeground,
    rememberHeight,
  };
}

export class ViewportAnimationController {
  private observer: IntersectionObserver | null = null;
  private elements = new Set<Element>();
  private frameId: number | null = null;
  private refreshTimer: ReturnType<typeof setTimeout> | null = null;
  private lastRefreshAt = 0;
  private active = true;

  constructor(private readonly root: HTMLElement) {
    if (typeof IntersectionObserver !== "undefined") {
      this.observer = new IntersectionObserver(
        (entries) => {
          for (const entry of entries) {
            const element = entry.target as HTMLElement;
            element.classList.toggle(
              "vcp-element-offscreen",
              !this.active || !entry.isIntersecting,
            );
          }
        },
        { threshold: 0 },
      );
    }
  }

  setActive(active: boolean) {
    this.active = active;
    this.root.classList.toggle("vcp-animation-paused", !active);
    if (!active) {
      this.elements.forEach((element) =>
        element.classList.add("vcp-element-offscreen"),
      );
    }
  }

  refresh() {
    if (!this.observer || this.frameId !== null || this.refreshTimer !== null) {
      return;
    }
    const elapsed = performance.now() - this.lastRefreshAt;
    if (this.lastRefreshAt > 0 && elapsed < 250) {
      this.refreshTimer = setTimeout(() => {
        this.refreshTimer = null;
        this.refresh();
      }, 250 - elapsed);
      return;
    }
    const observer = this.observer;
    this.frameId = requestAnimationFrame(() => {
      this.frameId = null;
      this.lastRefreshAt = performance.now();
      observer.disconnect();
      this.elements.forEach((element) =>
        element.classList.remove("vcp-element-offscreen"),
      );
      this.elements.clear();

      const candidates = new Set<Element>([
        ...this.root.querySelectorAll<Element>(
          "canvas, video, [data-vcp-animate]",
        ),
      ]);
      if (typeof this.root.getAnimations === "function") {
        for (const animation of this.root.getAnimations({ subtree: true })) {
          const effectTarget = (animation.effect as KeyframeEffect | null)?.target;
          const element =
            effectTarget instanceof Element
              ? effectTarget
              : (effectTarget as { element?: Element } | null)?.element;
          if (element instanceof Element && this.root.contains(element)) {
            candidates.add(element);
          }
        }
      }
      for (const element of candidates) {
        this.elements.add(element);
        observer.observe(element);
        if (!this.active) element.classList.add("vcp-element-offscreen");
      }
    });
  }

  disconnect() {
    if (this.frameId !== null) cancelAnimationFrame(this.frameId);
    if (this.refreshTimer !== null) clearTimeout(this.refreshTimer);
    this.frameId = null;
    this.refreshTimer = null;
    this.observer?.disconnect();
    this.elements.clear();
  }
}
