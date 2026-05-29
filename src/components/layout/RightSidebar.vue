<script setup lang="ts">
import NotificationCenterPage from '../../features/notification/NotificationCenterPage.vue';

const props = defineProps<{ isOpen: boolean }>();

const emit = defineEmits<{
  close: [];
}>();
</script>

<template>
  <NotificationCenterPage
    :is-open="props.isOpen"
    @close="emit('close')"
  />
</template>
