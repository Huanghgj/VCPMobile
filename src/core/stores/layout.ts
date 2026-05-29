import { defineStore } from 'pinia';
import { ref } from 'vue';
import { useModalHistory } from '../composables/useModalHistory';

export const useLayoutStore = defineStore('layout', () => {
  const { registerModal, unregisterModal } = useModalHistory();

  const leftDrawerOpen = ref(false);
  const rightDrawerOpen = ref(false);
  const rightDrawerPreparing = ref(false);
  let rightDrawerRequestId = 0;

  const toggleLeftDrawer = () => setLeftDrawer(!leftDrawerOpen.value);
  const toggleRightDrawer = () => { void setRightDrawer(!rightDrawerOpen.value); };

  const closeRightDrawer = () => {
    rightDrawerRequestId++;
    rightDrawerPreparing.value = false;
    if (!rightDrawerOpen.value) return;

    rightDrawerOpen.value = false;
    unregisterModal('RightDrawer');
  };

  const openRightDrawer = () => {
    if (rightDrawerOpen.value || rightDrawerPreparing.value) return;

    const requestId = ++rightDrawerRequestId;
    rightDrawerPreparing.value = true;
    setLeftDrawer(false);

    if (requestId === rightDrawerRequestId) {
      rightDrawerOpen.value = true;
      registerModal('RightDrawer', closeRightDrawer);
    }
    rightDrawerPreparing.value = false;
  };

  const setLeftDrawer = (open: boolean) => {
    if (open === leftDrawerOpen.value) return;

    if (!open) {
      leftDrawerOpen.value = false;
      unregisterModal('LeftDrawer');
      return;
    }

    void setRightDrawer(false);
    leftDrawerOpen.value = true;
    registerModal('LeftDrawer', () => { leftDrawerOpen.value = false; });
  };

  const setRightDrawer = async (open: boolean) => {
    if (!open) {
      closeRightDrawer();
      return;
    }

    openRightDrawer();
  };

  return {
    leftDrawerOpen,
    rightDrawerOpen,
    rightDrawerPreparing,
    toggleLeftDrawer,
    toggleRightDrawer,
    setLeftDrawer,
    setRightDrawer
  };
});
