<script setup lang="ts">
import { useRouter } from 'vue-router';
import { usePerformanceDiagnostics } from '../core/utils/performanceDiagnostics';

const router = useRouter();
const diagnostics = usePerformanceDiagnostics();

const openDiagnostics = () => {
  if (diagnostics.state.running) {
    void diagnostics.stopDiagnostics();
    return;
  }
  router.push('/diagnostics');
};
</script>

<template>
  <button
    v-if="diagnostics.state.running"
    class="fixed right-3 top-[calc(var(--vcp-safe-top,0px)+10px)] z-[9999] rounded-full border border-blue-300/40 bg-blue-600/90 px-3 py-2 text-xs font-black text-white shadow-xl backdrop-blur active:scale-95"
    @click="openDiagnostics"
  >
    点我结束 {{ Math.round(diagnostics.state.runDurationMs / 1000) }}s · FPS {{ diagnostics.averageFps.value || '--' }} · CPU {{ diagnostics.state.lastCpuPercent ?? '--' }}%
  </button>
</template>
