import { nextTick } from 'vue';
import { createPinia, setActivePinia } from 'pinia';
import { mount } from '@vue/test-utils';
import { describe, expect, it, vi } from 'vitest';
import GlobalOverlayManager from '@/components/GlobalOverlayManager.vue';
import { useModalHistory } from '@/core/composables/useModalHistory';
import { useOverlayStore } from '@/core/stores/overlay';

const ConfirmStub = {
  props: ['isOpen', 'title'],
  emits: ['confirm', 'cancel', 'update:isOpen'],
  template: `
    <div v-if="isOpen">
      <span>{{ title }}</span>
      <button data-testid="confirm" @click="$emit('confirm'); $emit('update:isOpen', false)">confirm</button>
      <button data-testid="cancel" @click="$emit('cancel'); $emit('update:isOpen', false)">cancel</button>
    </div>
  `,
};

function mountManager(options: { realConfirm?: boolean } = {}) {
  const pinia = createPinia();
  setActivePinia(pinia);
  const wrapper = mount(GlobalOverlayManager, {
    global: {
      plugins: [pinia],
      stubs: {
        VcpConfirm: options.realConfirm ? false : ConfirmStub,
        VcpPrompt: true,
        ToastManager: true,
        FullScreenEditor: true,
        RenderedImageViewer: true,
      },
    },
  });
  return { wrapper, store: useOverlayStore() };
}

describe('GlobalOverlayManager', () => {
  it('can resolve consecutive confirmations without invoking a cleared callback', async () => {
    const { wrapper, store } = mountManager();

    const first = store.showConfirm({ title: '第一次', message: '确认删除？' });
    expect(useModalHistory().modalStackLength()).toBe(1);
    await nextTick();
    await wrapper.get('[data-testid="confirm"]').trigger('click');
    await expect(first).resolves.toBe(true);
    expect(store.confirmConfig).toBeNull();
    expect(useModalHistory().modalStackLength()).toBe(0);

    const second = store.showConfirm({ title: '第二次', message: '再次确认？' });
    expect(useModalHistory().modalStackLength()).toBe(1);
    await nextTick();
    await wrapper.get('[data-testid="confirm"]').trigger('click');
    await expect(second).resolves.toBe(true);
    expect(store.confirmConfig).toBeNull();
    expect(useModalHistory().modalStackLength()).toBe(0);
  });

  it('replaces a context-menu history entry when an action opens a confirmation', async () => {
    const { wrapper, store } = mountManager();
    store.openContextMenu([], 'Actions');
    expect(useModalHistory().modalStackLength()).toBe(1);

    const confirmation = store.showConfirm({ title: 'Delete', message: 'Confirm delete?' });
    expect(store.contextMenuConfig).toBeNull();
    expect(store.confirmConfig?.title).toBe('Delete');
    expect(useModalHistory().modalStackLength()).toBe(1);
    expect(window.history.state?.vcpModalId).toBe('Confirm');

    await nextTick();
    await wrapper.get('[data-testid="confirm"]').trigger('click');
    await expect(confirmation).resolves.toBe(true);
    expect(useModalHistory().modalStackLength()).toBe(0);
  });

  it('keeps the confirmation open through the real context-menu action click sequence', async () => {
    const { wrapper, store } = mountManager();
    let confirmation: Promise<boolean> | undefined;
    store.openContextMenu([
      {
        label: 'Delete topic',
        handler: () => {
          confirmation = store.showConfirm({
            title: 'Delete',
            message: 'Confirm delete?',
          });
        },
      },
    ]);
    await nextTick();

    const actionButton = document.body.querySelector('button');
    expect(actionButton).not.toBeNull();
    actionButton!.click();
    await nextTick();

    expect(store.contextMenuConfig).toBeNull();
    expect(store.confirmConfig?.title).toBe('Delete');
    expect(useModalHistory().modalStackLength()).toBe(1);
    await wrapper.get('[data-testid="confirm"]').trigger('click');
    await expect(confirmation).resolves.toBe(true);
  });

  it('executes a context-menu action exactly once through the real confirmation component', async () => {
    const { store } = mountManager({ realConfirm: true });
    let deleteCalls = 0;

    store.openContextMenu([
      {
        label: 'Delete topic',
        handler: async () => {
          const confirmed = await store.showConfirm({
            title: 'Delete topic',
            message: 'Permanently delete this topic?',
            isDanger: true,
          });
          if (confirmed) deleteCalls += 1;
        },
      },
    ]);
    await nextTick();

    const actionButton = Array.from(document.body.querySelectorAll('button')).find(
      (button) => button.textContent?.includes('Delete topic'),
    );
    expect(actionButton).toBeDefined();
    actionButton!.click();
    await nextTick();

    expect(store.contextMenuConfig).toBeNull();
    expect(store.confirmConfig?.title).toBe('Delete topic');
    const confirmButton = Array.from(document.body.querySelectorAll('button')).find(
      (button) => button.textContent?.trim() === '确认',
    );
    expect(confirmButton).toBeDefined();
    confirmButton!.click();

    await vi.waitFor(() => expect(deleteCalls).toBe(1));
    expect(store.confirmConfig).toBeNull();
    expect(useModalHistory().modalStackLength()).toBe(0);
  });

  it('replaces a context-menu history entry when an action opens a prompt', () => {
    const { store } = mountManager();
    store.openContextMenu([], 'Actions');
    expect(useModalHistory().modalStackLength()).toBe(1);

    store.openPrompt({
      title: 'Rename',
      initialValue: 'Old name',
      placeholder: 'New name',
      onConfirm: () => undefined,
    });

    expect(store.contextMenuConfig).toBeNull();
    expect(store.promptConfig?.title).toBe('Rename');
    expect(useModalHistory().modalStackLength()).toBe(1);
    expect(window.history.state?.vcpModalId).toBe('Prompt');

    store.closePrompt();
    expect(useModalHistory().modalStackLength()).toBe(0);
  });

  it('does not overwrite an active confirmation with a concurrent request', async () => {
    const { wrapper, store } = mountManager();
    const first = store.showConfirm({ title: 'First', message: 'First request' });
    const second = store.showConfirm({ title: 'Second', message: 'Second request' });

    await expect(second).resolves.toBe(false);
    expect(store.confirmConfig?.title).toBe('First');
    expect(useModalHistory().modalStackLength()).toBe(1);

    await nextTick();
    await wrapper.get('[data-testid="confirm"]').trigger('click');
    await expect(first).resolves.toBe(true);
    expect(useModalHistory().modalStackLength()).toBe(0);
  });
});
