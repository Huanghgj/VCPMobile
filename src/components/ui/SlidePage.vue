<script setup lang="ts">
import { LAYER_PAGE_BASE } from '../../core/constants/layers';

interface Props {
  isOpen: boolean;
  zIndex?: number;
  transitionName?: string;
}

const props = withDefaults(defineProps<Props>(), {
  zIndex: LAYER_PAGE_BASE,
  transitionName: 'slide-page',
});
</script>

<template>
  <Transition :name="props.transitionName">
    <div
      v-show="props.isOpen"
      class="fixed inset-0 pointer-events-auto"
      :style="{ zIndex: props.zIndex }"
    >
      <slot />
    </div>
  </Transition>
</template>

<style scoped>
.slide-page-enter-active {
  transition: transform 0.35s cubic-bezier(0.32, 0.72, 0, 1);
}

.slide-page-leave-active {
  transition: transform 0.3s cubic-bezier(0.32, 0.72, 0, 1);
}

.slide-page-enter-from,
.slide-page-leave-to {
  transform: translateX(100%);
}

.notification-glass-page-enter-active {
  transition:
    opacity 0.22s ease,
    transform 0.28s cubic-bezier(0.16, 1, 0.3, 1);
}

.notification-glass-page-leave-active {
  transition:
    opacity 0.18s ease,
    transform 0.22s cubic-bezier(0.3, 0, 0.2, 1);
}

.notification-glass-page-enter-from,
.notification-glass-page-leave-to {
  opacity: 0;
  transform: translateY(14px) scale(0.985);
}

@media (prefers-reduced-motion: reduce) {
  .slide-page-enter-active,
  .slide-page-leave-active,
  .notification-glass-page-enter-active,
  .notification-glass-page-leave-active {
    transition-duration: 0.01ms;
  }
}
</style>
