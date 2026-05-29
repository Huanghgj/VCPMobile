<template>
  <div class="notification-empty">
    <div class="notification-empty-icon">
      <Bell :size="34" stroke-width="1.6" />
    </div>
    <div class="notification-empty-title">暂无通知</div>
    <div class="notification-empty-subtitle">VCPLog 与 VCPInfo 的新事件会出现在这里</div>
  </div>
</template>

<script setup lang="ts">
import { Bell } from 'lucide-vue-next';
</script>

<style scoped>
.notification-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  min-height: 280px;
  color: var(--primary-text);
  text-align: center;
}

.notification-empty-icon {
  width: 62px;
  height: 62px;
  display: flex;
  align-items: center;
  justify-content: center;
  border: 1px solid rgba(255, 255, 255, 0.14);
  border-radius: 8px;
  background: rgba(255, 255, 255, 0.11);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.16);
}

.notification-empty-title {
  margin-top: 16px;
  font-size: 14px;
  font-weight: 900;
}

.notification-empty-subtitle {
  max-width: 230px;
  margin-top: 6px;
  color: color-mix(in srgb, var(--primary-text) 55%, transparent);
  font-size: 11px;
  line-height: 1.6;
}
</style>
