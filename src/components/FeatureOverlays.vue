<script setup lang="ts">
/**
 * FeatureOverlays.vue
 *
 * 职责：作为所有全局业务 Feature 视图的统一挂载点。
 */
import { ref, onMounted } from 'vue';
import { useOverlayStore } from '../core/stores/overlay';
import SettingsView from '../features/settings/SettingsView.vue';
import AgentSettingsView from '../features/agent/AgentSettingsView.vue';
import GroupSettingsView from '../features/agent/GroupSettingsView.vue';
import SyncSessionView from '../features/sync/SyncSessionView.vue';
import VcpToolboxView from '../features/toolbox/VcpToolboxView.vue';
import RagObserverView from '../features/rag/RagObserverView.vue';

const overlayStore = useOverlayStore();
const isMounted = ref(false);

onMounted(() => {
  isMounted.value = true;
});
</script>

<template>
  <div v-if="isMounted">
    <SettingsView
      :is-open="overlayStore.isSettingsOpen"
      :z-index="overlayStore.getPageZIndex('settings')"
      @close="overlayStore.closeSettings()"
    />

    <VcpToolboxView
      :is-open="overlayStore.isToolboxOpen"
      :z-index="overlayStore.getPageZIndex('toolbox')"
      @close="overlayStore.closeToolbox()"
    />

    <RagObserverView
      :is-open="overlayStore.isRagObserverOpen"
      :z-index="overlayStore.getPageZIndex('ragObserver')"
      @close="overlayStore.closeRagObserver()"
    />

    <AgentSettingsView
      :is-open="overlayStore.isAgentSettingsOpen"
      :id="overlayStore.agentSettingsId"
      :z-index="overlayStore.getPageZIndex('agentSettings')"
      @close="overlayStore.closeAgentSettings()"
    />

    <GroupSettingsView
      :is-open="overlayStore.isGroupSettingsOpen"
      :id="overlayStore.groupSettingsId"
      :z-index="overlayStore.getPageZIndex('groupSettings')"
      @close="overlayStore.closeGroupSettings()"
    />

    <SyncSessionView />
  </div>
</template>
