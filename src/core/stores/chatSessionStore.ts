import { defineStore } from "pinia";
import { onScopeDispose, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { useAssistantStore } from "./assistant";
import { createDefaultTopicTitle } from "../utils/topicTitle";

export interface PickedFileInfo {
  path: string;
  name: string;
  mime: string;
  size: number;
  hash: string;
  thumbnailPath?: string;
  internalPath?: string;
}

export const useChatSessionStore = defineStore(
  "chatSession",
  () => {
    const currentSelectedItem = ref<any>(null);
    const currentTopicId = ref<string | null>(null);
    const lastActiveTopicMap = ref<Record<string, string>>({});

    // Share intent prefill state
    const sharePrefillText = ref("");
    const sharePrefillFiles = ref<PickedFileInfo[]>([]);
    let selectionSequence = 0;

    const assistantStore = useAssistantStore();

    /**
     * 从外部分享意图启动会话
     * 1. 选择 Agent → 创建话题 → 切换到聊天 → 预填输入
     */
    const startShareSession = async (
      agentId: string,
      sharedText: string,
      sharedFiles: PickedFileInfo[],
    ) => {
      const sequence = ++selectionSequence;

      // 1. 查找并选中 agent
      const agent = assistantStore.agents.find((a) => a.id === agentId);
      if (!agent) {
        throw new Error(`Agent ${agentId} not found`);
      }

      // 2. 创建新话题（复用 TopicCreator 默认命名逻辑）
      const newTopicName = createDefaultTopicTitle();

      const newTopic = await invoke<any>("create_topic", {
        ownerId: agentId,
        ownerType: "agent",
        name: newTopicName,
      });

      if (!newTopic?.id) {
        throw new Error("Failed to create topic");
      }

      if (sequence !== selectionSequence) {
        try {
          await invoke("delete_topic", {
            ownerId: agentId,
            ownerType: "agent",
            topicId: newTopic.id,
          });
        } catch (error) {
          console.warn(
            `[ChatSessionStore] Failed to clean superseded share topic ${newTopic.id}:`,
            error,
          );
        }
        throw new Error("分享会话已被新的选择取代");
      }

      // 3. 选择 topic（设置 currentSelectedItem 和 currentTopicId）
      await applyTopicSelection(agentId, newTopic.id, sequence);

      if (sequence !== selectionSequence) {
        throw new Error("分享会话已被新的选择取代");
      }

      // 4. 存储预填数据（由 ChatView/InputEnhancer 消费后清空）
      sharePrefillText.value = sharedText;
      sharePrefillFiles.value = sharedFiles;

      return { topicId: newTopic.id, agentId };
    };

    /**
     * 消费分享预填数据（调用后清空）
     */
    const consumeSharePrefill = () => {
      const text = sharePrefillText.value;
      const files = sharePrefillFiles.value;
      sharePrefillText.value = "";
      sharePrefillFiles.value = [];
      return { text, files };
    };

    /**
     * 选择一个助手或群组，并自动跳转到最近的话题
     * @param loadHistoryCallback 回调函数，用于触发历史加载 (解耦 HistoryStore)
     */
    const applyTopicSelection = async (
      itemId: string,
      topicId: string,
      sequence: number,
      loadHistoryCallback?: (
        itemId: string,
        ownerType: string,
        topicId: string,
      ) => Promise<void>,
    ) => {
      if (sequence !== selectionSequence) {
        console.warn(
          `[ChatSessionStore] Selection ${topicId} superseded (seq ${sequence} != ${selectionSequence}), dropped.`,
        );
        return;
      }

      // 立即更新 currentTopicId，确保话题列表高亮实时响应
      currentTopicId.value = topicId;

      // 记录在该 itemId 下最后一次选中的活跃话题 ID
      lastActiveTopicMap.value[itemId] = topicId;

      const ownerType = assistantStore.agents.some((a) => a.id === itemId)
        ? "agent"
        : "group";

      // 设置当前选中的项目详情 (确保头像和色调同步)
      const agent = assistantStore.agents.find((a: any) => a.id === itemId);
      const group = assistantStore.groups.find((g) => g.id === itemId);
      if (agent) {
        currentSelectedItem.value = { ...agent, type: "agent" };
      } else if (group) {
        currentSelectedItem.value = { ...group, type: "group" };
      }

      if (loadHistoryCallback) {
        await loadHistoryCallback(itemId, ownerType, topicId);
      }
    };

    const selectTopicById = async (
      itemId: string,
      topicId: string,
      loadHistoryCallback?: (
        itemId: string,
        ownerType: string,
        topicId: string,
      ) => Promise<void>,
    ) => {
      const sequence = ++selectionSequence;
      await applyTopicSelection(itemId, topicId, sequence, loadHistoryCallback);
    };

    /**
     * 选择一个项目 (Agent/Group)，自动加载其记录的或最新的话题
     */
    const selectItem = async (
      item: any,
      loadHistoryCallback?: (
        itemId: string,
        ownerType: string,
        topicId: string,
      ) => Promise<void>,
    ) => {
      if (!item) return;

      const ownerId = item.id;
      const ownerType = item.members ? "group" : "agent";

      // 如果已经选中了该项，且当前已有话题，则不重复加载
      if (currentSelectedItem.value?.id === ownerId && currentTopicId.value) {
        return;
      }

      const sequence = ++selectionSequence;
      currentSelectedItem.value = { ...item, type: ownerType };
      currentTopicId.value = null;

      // 1. 优先从 Pinia 持久化的 lastActiveTopicMap 中获取最后一次打开的话题 ID
      let targetTopicId = lastActiveTopicMap.value[ownerId];

      // 2. 如果 Pinia 中没有记录，则尝试获取该 Owner 下最新的话题
      if (!targetTopicId) {
        try {
          const topics = await invoke<any[]>("get_topics", {
            ownerId,
            ownerType,
          });
          if (topics && topics.length > 0) {
            // 列表通常按 updated_at 倒序，取第一个
            targetTopicId = topics[0].id || topics[0].topic_id;
          }
        } catch (e) {
          console.error(
            "[ChatSessionStore] Failed to fetch fallback topics:",
            e,
          );
        }
      }

      if (targetTopicId) {
        await applyTopicSelection(
          ownerId,
          targetTopicId,
          sequence,
          loadHistoryCallback,
        );
      } else {
        if (sequence !== selectionSequence) return;
        // 没有任何话题的极端情况
        console.warn(`[ChatSessionStore] No topics found for ${ownerId}`);
        currentSelectedItem.value = { ...item, type: ownerType };
        currentTopicId.value = null;
      }
    };

    const reconcilePersistedSelection = async () => {
      const sequence = ++selectionSequence;
      const selected = currentSelectedItem.value;
      if (!selected?.id) {
        currentSelectedItem.value = null;
        currentTopicId.value = null;
        return;
      }

      const agent = assistantStore.agents.find(
        (item) => item.id === selected.id,
      );
      const group = assistantStore.groups.find(
        (item) => item.id === selected.id,
      );
      const activeOwner = agent || group;
      if (!activeOwner) {
        // 双列表为空通常意味着数据尚未加载（而非用户删光了所有助手）。
        // 此时清空持久化选择会导致启动后"无选中、新建话题按钮禁用"，故跳过。
        if (
          assistantStore.agents.length === 0 &&
          assistantStore.groups.length === 0
        ) {
          console.warn(
            "[ChatSessionStore] Skip reconcile: assistant lists are empty (possibly not loaded yet).",
          );
          return;
        }
        delete lastActiveTopicMap.value[selected.id];
        currentSelectedItem.value = null;
        currentTopicId.value = null;
        return;
      }

      const ownerType = agent ? "agent" : "group";
      try {
        const topics = await invoke<any[]>("get_topics", {
          ownerId: selected.id,
          ownerType,
        });
        if (sequence !== selectionSequence) return;
        const topicIds = new Set(
          topics.map((topic) => topic.id || topic.topic_id).filter(Boolean),
        );
        const rememberedTopicId = lastActiveTopicMap.value[selected.id];
        let targetTopicId = currentTopicId.value;
        if (!targetTopicId || !topicIds.has(targetTopicId)) {
          targetTopicId =
            rememberedTopicId && topicIds.has(rememberedTopicId)
              ? rememberedTopicId
              : topics[0]?.id || topics[0]?.topic_id || null;
        }

        currentSelectedItem.value = { ...activeOwner, type: ownerType };
        currentTopicId.value = targetTopicId;
        if (targetTopicId) {
          lastActiveTopicMap.value[selected.id] = targetTopicId;
        } else {
          delete lastActiveTopicMap.value[selected.id];
        }
      } catch (error) {
        console.error(
          "[ChatSessionStore] Failed to reconcile persisted selection:",
          error,
        );
      }
    };

    const discardOwnerSelection = (ownerId: string) => {
      selectionSequence += 1;
      delete lastActiveTopicMap.value[ownerId];
      if (currentSelectedItem.value?.id === ownerId) {
        currentSelectedItem.value = null;
        currentTopicId.value = null;
      }
    };

    const handleOwnerDeleted = (event: Event) => {
      const detail = (event as CustomEvent<{ ownerId?: string }>).detail;
      if (detail?.ownerId) discardOwnerSelection(detail.ownerId);
    };

    if (typeof window !== "undefined") {
      window.addEventListener("vcp-owner-deleted", handleOwnerDeleted);
    }

    onScopeDispose(() => {
      if (typeof window !== "undefined") {
        window.removeEventListener("vcp-owner-deleted", handleOwnerDeleted);
      }
    });

    return {
      currentSelectedItem,
      currentTopicId,
      lastActiveTopicMap,
      sharePrefillText,
      sharePrefillFiles,
      startShareSession,
      consumeSharePrefill,
      selectTopicById,
      selectItem,
      reconcilePersistedSelection,
      discardOwnerSelection,
    };
  },
  {
    persist: {
      pick: ["currentSelectedItem", "currentTopicId", "lastActiveTopicMap"],
    },
  },
);
