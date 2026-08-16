import { defineStore } from 'pinia';
import { computed, onScopeDispose, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { useAssistantStore } from './assistant';
import { useSettingsStore } from './settings';
import { useThemeStore } from './theme';
import { useNotificationStore } from './notification';
import { isTauriRuntime } from '../utils/runtime';
import { useAvatarStore } from './avatar';
import { useChatSessionStore } from './chatSessionStore';
import { updateDistributedState } from '../../features/distributed/composables/useDistributed';

export type AppState = 'PERMISSIONS' | 'BOOTING' | 'CONNECTING' | 'PRELOADING' | 'READY' | 'ERROR';

export interface CoreStatus {
  status: 'initializing' | 'ready' | 'error' | 'none';
  message: string;
}

const CONNECT_TIMEOUT_MS = 15000;
const PRELOAD_TIMEOUT_MS = 20000;

async function withTimeout<T>(
  operation: Promise<T>,
  timeoutMs: number,
  timeoutMessage: string,
): Promise<T> {
  let timeoutId: ReturnType<typeof setTimeout> | null = null;
  try {
    return await Promise.race([
      operation,
      new Promise<never>((_, reject) => {
        timeoutId = setTimeout(() => reject(new Error(timeoutMessage)), timeoutMs);
      }),
    ]);
  } finally {
    if (timeoutId) clearTimeout(timeoutId);
  }
}

export const useAppLifecycleStore = defineStore('appLifecycle', () => {
  const state = ref<AppState>('BOOTING');
  const errorMsg = ref<string | null>(null);
  const isBootstrapping = ref(false);
  const hasBootstrapped = ref(false);
  const currentPhaseLabel = ref('准备启动...');
  const lastTransitionAt = ref<number | null>(null);

  const assistantStore = useAssistantStore();
  const sessionStore = useChatSessionStore();
  const settingsStore = useSettingsStore();
  const themeStore = useThemeStore();
  const avatarStore = useAvatarStore();
  const notificationStore = useNotificationStore();

  let bootstrapPromise: Promise<void> | null = null;
  let coreReadyUnlisten: (() => void) | null = null;
  let connectTimeoutId: ReturnType<typeof setTimeout> | null = null;

  const statusText = computed(() => {
    switch (state.value) {
      case 'PERMISSIONS':
        return '正在检查系统权限...';
      case 'BOOTING':
        return '正在初始化界面资源...';
      case 'CONNECTING':
        return '正在连接核心服务...';
      case 'PRELOADING':
        return currentPhaseLabel.value || '正在预加载核心数据...';
      case 'ERROR':
        return errorMsg.value || '启动失败';
      case 'READY':
      default:
        return '应用已就绪';
    }
  });

  const setState = (nextState: AppState, reason: string) => {
    state.value = nextState;
    lastTransitionAt.value = Date.now();
    console.log(`[Lifecycle] -> ${nextState} | ${reason}`);
  };

  const updatePhaseLabel = (label: string) => {
    currentPhaseLabel.value = label;
    console.log(`[Lifecycle] ${label}`);
  };

  const cleanupConnectionWaiters = () => {
    if (connectTimeoutId) {
      clearTimeout(connectTimeoutId);
      connectTimeoutId = null;
    }

    if (coreReadyUnlisten) {
      coreReadyUnlisten();
      coreReadyUnlisten = null;
    }
  };

  const fail = (message: string) => {
    cleanupConnectionWaiters();
    errorMsg.value = message;
    isBootstrapping.value = false;
    bootstrapPromise = null;
    setState('ERROR', message);
    // 统一更新通知系统的核心状态槽
    notificationStore.updateCoreStatus({
      status: 'error',
      message,
      source: 'Core'
    });
    console.error('[Lifecycle] FATAL:', message);
  };




  // 全局监听同步状态变化 (移除自动触发神经同步逻辑)
  const unwatchVcpStatus = watch(
    () => notificationStore.vcpStatus.status,
    async (newStatus, oldStatus) => {
      if (newStatus === 'connected' && oldStatus !== 'connected' && state.value === 'READY') {
        // 仅在连接成功时重新拉取数据快照，但不触发耗时的 Manifest 全量同步
        try {
          await assistantStore.fetchAgents();
          await assistantStore.fetchGroups();
          await sessionStore.reconcilePersistedSelection();
        } catch (error) {
          console.error('[Lifecycle] Failed to refresh data after reconnect:', error);
        }
      }
    }
  );

  onScopeDispose(() => {
    unwatchVcpStatus();
  });

  const startPreloading = async () => {
    if (state.value === 'PRELOADING' || state.value === 'READY') {
      console.log(`[Lifecycle] Skip preloading in state: ${state.value}`);
      return;
    }

    cleanupConnectionWaiters();
    setState('PRELOADING', '开始预加载核心业务数据');
    const startTime = Date.now();

    try {
      updatePhaseLabel('正在并发预加载配置与助手数据...');
      console.log('[Lifecycle] [Concurrent] START Preloading Settings and AgentsAndGroups');

      await withTimeout(
        Promise.all([
          settingsStore.fetchSettings(),
          assistantStore.fetchAgentsAndGroups()
        ]),
        PRELOAD_TIMEOUT_MS,
        '核心数据预加载超时',
      );

      console.log(`[Lifecycle] [Concurrent] DONE Preloading in ${Date.now() - startTime}ms`);
      await sessionStore.reconcilePersistedSelection();

      updatePhaseLabel('正在预加载头像资源...');
      await withTimeout(
        avatarStore.preloadAvatars([
          { ownerType: 'user', ownerId: 'user_avatar' },
          ...assistantStore.agents.map((agent) => ({ ownerType: 'agent' as const, ownerId: agent.id })),
          ...assistantStore.groups.map((group) => ({ ownerType: 'group' as const, ownerId: group.id })),
        ]),
        PRELOAD_TIMEOUT_MS,
        '头像资源预加载超时',
      );

      updatePhaseLabel('核心数据预加载完成');

      hasBootstrapped.value = true;
      isBootstrapping.value = false;
      bootstrapPromise = null;
      setState('READY', '应用就绪');
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      fail(`预加载失败: ${message}`);
      throw error;
    }
  };

  const waitForCoreReady = async () => {
    updatePhaseLabel('检查核心服务状态...');

    // 核心优化：直接读取第一步在 hydrateSystemStatus 中拉回并写入真相源的状态，免去重复 IPC 检测
    const currentStatus = notificationStore.vcpCoreStatus.status;
    console.log(`[Lifecycle] Checked core status from snapshot -> ${currentStatus}`);

    if (currentStatus === 'ready') {
      return;
    }

    if (currentStatus === 'error') {
      const lastError = await invoke<string | null>('get_last_error');
      const msg = lastError || '核心服务在初始化阶段发生崩溃';
      notificationStore.updateCoreStatus({ status: 'error', message: msg, source: 'Core' });
      throw new Error(msg);
    }

    // 2. 等待状态变为 ready (由 useNotificationProcessor 触发)
    updatePhaseLabel('等待核心就绪...');

    await new Promise<void>((resolve, reject) => {
      let settled = false;
      let timeoutId: ReturnType<typeof setTimeout>;
      let unwatch: (() => void) | null = null;
      let shouldStopImmediately = false;

      const cleanup = () => {
        clearTimeout(timeoutId);
        if (unwatch) {
          unwatch();
        } else {
          shouldStopImmediately = true;
        }
      };

      // 仅作为极端挂死的兜底
      timeoutId = setTimeout(() => {
        if (!settled) {
          settled = true;
          cleanup();
          reject(new Error(`等待核心引擎就绪超时（${CONNECT_TIMEOUT_MS}ms）`));
        }
      }, CONNECT_TIMEOUT_MS);

      unwatch = watch(
        () => notificationStore.vcpCoreStatus.status,
        (newStatus) => {
          if (settled) return;
          if (newStatus === 'ready') {
            settled = true;
            cleanup();
            resolve();
          } else if (newStatus === 'error') {
            settled = true;
            cleanup();
            reject(new Error(notificationStore.vcpCoreStatus.message || '核心引擎启动失败'));
          }
        },
        { immediate: true }
      );
      if (shouldStopImmediately) {
        unwatch();
      }
    });
  };

  const hydrateSystemStatus = async () => {
    try {
      console.log('[Lifecycle] Fetching system status snapshot...');
      const snapshot = await invoke<{ core: string; log: string; sync: string; distributed: string }>('get_system_snapshot');

      // 同步到 Notification Store (唯一真相源)
      notificationStore.updateCoreStatus({
        status: snapshot.core as any,
        message: snapshot.core === 'ready' ? '核心引擎已就绪' : '核心引擎初始化中...',
        source: 'Core'
      });

      notificationStore.updateStatus({
        status: snapshot.log as any,
        message: snapshot.log === 'connected' ? '已连接' : '正在连接...',
        source: 'VCPLog'
      });

      // 同步分布式连接状态到专有的 Distributed Composable
      updateDistributedState(snapshot.distributed as any);

      console.log('[Lifecycle] Snapshot hydrated:', JSON.stringify(snapshot));
    } catch (e) {
      console.error('[Lifecycle] Failed to hydrate status snapshot:', e);
    }
  };

  const bootstrap = async (force = false) => {
    if (isBootstrapping.value && force) {
      console.log('[Lifecycle] Reusing existing bootstrap promise (force-in-progress ignored)');
      return bootstrapPromise;
    }

    if (bootstrapPromise && !force) {
      console.log('[Lifecycle] Reusing existing bootstrap promise');
      return bootstrapPromise;
    }

    bootstrapPromise = (async () => {
      try {
        isBootstrapping.value = true;
        errorMsg.value = null;
        hasBootstrapped.value = false;

        if (!isTauriRuntime()) {
          setState('BOOTING', 'Web 预览模式初始化');
          updatePhaseLabel('加载本地界面资源...');
          await themeStore.initTheme();
          await themeStore.preloadCurrentThemeAssets();
          notificationStore.updateCoreStatus({
            status: 'disconnected',
            message: 'Web 预览模式（无核心服务）',
            source: 'Core'
          });
          notificationStore.updateStatus({
            status: 'disconnected',
            message: 'Web 预览模式（未连接 VCPToolBox）',
            source: 'VCPLog'
          });
          hasBootstrapped.value = true;
          isBootstrapping.value = false;
          bootstrapPromise = null;
          setState('READY', 'Web 预览模式就绪');
          return;
        }

        setState('PERMISSIONS', '检查系统权限完整性');
        updatePhaseLabel('预加载前端静态资源...');
        await themeStore.initTheme();
        await themeStore.preloadCurrentThemeAssets();

        const pStatus = await invoke<{ notification: boolean; storage: boolean; battery: boolean }>('plugin:vcp-mobile|check_all_permissions');
        if (!pStatus.notification || !pStatus.storage || !pStatus.battery) {
          console.log('[Lifecycle] Missing permissions, waiting for user action');
          // 清除 Promise，以便下次点击“进入应用”时能重新触发
          bootstrapPromise = null;
          isBootstrapping.value = false;
          return;
        }

        setState('BOOTING', '开始前端主线程启动编排');

        // --- 核心优化：先拿快照，再跑流程 ---
        await hydrateSystemStatus();

        updatePhaseLabel('初始化主题资源...');
        console.log('[Lifecycle] Theme initialization complete');

        setState('CONNECTING', '等待后端核心服务就绪');
        await waitForCoreReady();
        await startPreloading();
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        fail(message);
        throw error;
      }
    })();

    return bootstrapPromise;
  };

  return {
    state,
    errorMsg,
    statusText,
    currentPhaseLabel,
    isBootstrapping,
    hasBootstrapped,
    lastTransitionAt,
    coreStatus: computed(() => notificationStore.vcpCoreStatus),
    bootstrap,
    hydrateSystemStatus
  };
});
