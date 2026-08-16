<script setup lang="ts">
import { computed, ref } from "vue";
import { useRouter } from "vue-router";
import { useTopicStore } from "../../core/stores/topicListManager";
import { useChatSessionStore } from "../../core/stores/chatSessionStore";
import { useAssistantStore } from "../../core/stores/assistant";
import { useLayoutStore } from "../../core/stores/layout";
import { useNotificationStore } from "../../core/stores/notification";
import { createDefaultTopicTitle } from "../../core/utils/topicTitle";

const topicStore = useTopicStore();
const sessionStore = useChatSessionStore();
const assistantStore = useAssistantStore();
const layoutStore = useLayoutStore();
const notificationStore = useNotificationStore();
const router = useRouter();

const isCreating = ref(false);

// create_topic 走 SQLite 事务，与同步写队列争锁时可能长时间挂起（busy_timeout 30s）。
// 若不设上限，isCreating 永不复位，按钮将永久禁用（表现为"点了没反应"）。
const CREATE_TOPIC_TIMEOUT_MS = 10000;

class CreateTopicTimeoutError extends Error {
  constructor() {
    super("创建话题超时");
  }
}

const withTimeout = <T>(promise: Promise<T>, ms: number): Promise<T> =>
  new Promise<T>((resolve, reject) => {
    const timer = setTimeout(() => reject(new CreateTopicTimeoutError()), ms);
    promise.then(
      (value) => {
        clearTimeout(timer);
        resolve(value);
      },
      (error) => {
        clearTimeout(timer);
        reject(error);
      },
    );
  });

const currentItemId = computed(
  () =>
    sessionStore.currentSelectedItem?.id || assistantStore.agents[0]?.id || null,
);
const canCreateTopic = computed(
  () => Boolean(currentItemId.value) && !isCreating.value,
);

const selectTopic = async (
  itemId: string,
  topicId: string,
  topicName: string,
) => {
  if (router.currentRoute.value.path !== "/chat") {
    await router.push("/chat");
  }

  // 使用统一的 sessionStore 选择话题，历史加载由 ChatView 的 watcher 响应
  await sessionStore.selectTopicById(itemId, topicId);

  const createdTopic = topicStore.topics.find((topic) => topic.id === topicId);
  if (createdTopic) {
    createdTopic.name = topicName;
  }

  layoutStore.setLeftDrawer(false);
};

const handleCreateTopic = async () => {
  if (isCreating.value) return;

  console.info(
    "[TopicCreator] create-topic clicked",
    sessionStore.currentSelectedItem,
  );

  if (!currentItemId.value) {
    notificationStore.addNotification({
      type: "warning",
      title: "无法创建话题",
      message: "请先选择一个助手或群组",
      toastOnly: true,
    });
    return;
  }

  isCreating.value = true;

  const newTopicName = createDefaultTopicTitle();
  const ownerId = currentItemId.value;

  try {
    const ownerType = assistantStore.agents.some((a) => a.id === ownerId)
      ? "agent"
      : "group";

    const newTopic = await withTimeout(
      topicStore.createTopic(ownerId, ownerType, newTopicName),
      CREATE_TOPIC_TIMEOUT_MS,
    );
    if (newTopic?.id && currentItemId.value === ownerId) {
      await selectTopic(ownerId, newTopic.id, newTopic.name);
    }
  } catch (error) {
    console.error("[TopicCreator] create-topic failed", error);
    if (error instanceof CreateTopicTimeoutError) {
      // store 层只对 invoke 抛错弹通知；超时由这里兜底提示
      notificationStore.addNotification({
        type: "error",
        title: "创建话题超时",
        message: "数据库繁忙，请稍后重试",
        duration: 5000,
      });
    }
    // 其余错误通知已在 store 层处理
  } finally {
    // 1秒防抖/锁定，防止快速连击
    setTimeout(() => {
      isCreating.value = false;
    }, 1000);
  }
};
</script>

<template>
  <button
    class="w-full py-2.5 bg-green-500/10 dark:bg-green-500/20 hover:bg-green-500/20 dark:hover:bg-green-500/30 text-green-600 dark:text-green-400 rounded-xl text-sm font-bold transition-all flex items-center justify-center gap-2 disabled:opacity-40 disabled:cursor-not-allowed disabled:hover:bg-green-500/10 disabled:dark:hover:bg-green-500/20"
    :disabled="!canCreateTopic" @click="handleCreateTopic">
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
      <line x1="12" y1="5" x2="12" y2="19"></line>
      <line x1="5" y1="12" x2="19" y2="12"></line>
    </svg>
    新建话题
  </button>
</template>
