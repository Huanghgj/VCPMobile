import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useAssistantStore } from "@/core/stores/assistant";
import { useChatSessionStore } from "@/core/stores/chatSessionStore";
import { useChatStreamStore } from "@/core/stores/chatStreamStore";
import { mockInvoke } from "@/tests/mocks/tauri";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((res) => {
    resolve = res;
  });
  return { promise, resolve };
}

describe("chatSessionStore.reconcilePersistedSelection", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it("clears a persisted owner that no longer exists", async () => {
    const assistant = useAssistantStore();
    // 列表非空说明数据已加载完成，缺失的 owner 是真的被删除了
    assistant.agents = [{ id: "surviving-agent", name: "Survivor" } as any];
    const session = useChatSessionStore();
    session.currentSelectedItem = {
      id: "deleted-agent",
      name: "Deleted",
      type: "agent",
    };
    session.currentTopicId = "deleted-topic";
    session.lastActiveTopicMap["deleted-agent"] = "deleted-topic";

    await session.reconcilePersistedSelection();

    expect(session.currentSelectedItem).toBeNull();
    expect(session.currentTopicId).toBeNull();
    expect(session.lastActiveTopicMap["deleted-agent"]).toBeUndefined();
  });

  it("keeps the persisted selection when assistant lists have not loaded", async () => {
    const session = useChatSessionStore();
    session.currentSelectedItem = {
      id: "agent-1",
      name: "Agent 1",
      type: "agent",
    };
    session.currentTopicId = "topic-1";
    session.lastActiveTopicMap["agent-1"] = "topic-1";

    // agents/groups 均为空：视为数据未就绪，不得清空用户选择
    await session.reconcilePersistedSelection();

    expect(session.currentSelectedItem?.id).toBe("agent-1");
    expect(session.currentTopicId).toBe("topic-1");
    expect(session.lastActiveTopicMap["agent-1"]).toBe("topic-1");
  });

  it("falls back from a deleted topic to the newest active topic", async () => {
    const assistant = useAssistantStore();
    assistant.agents = [{ id: "agent-1", name: "Agent 1" } as any];
    mockInvoke("get_topics", () => [
      { id: "topic-new", name: "New" },
      { id: "topic-old", name: "Old" },
    ]);
    const session = useChatSessionStore();
    session.currentSelectedItem = {
      id: "agent-1",
      name: "Stale Agent",
      type: "agent",
    };
    session.currentTopicId = "topic-deleted";
    session.lastActiveTopicMap["agent-1"] = "topic-deleted";

    await session.reconcilePersistedSelection();

    expect(session.currentSelectedItem?.name).toBe("Agent 1");
    expect(session.currentTopicId).toBe("topic-new");
    expect(session.lastActiveTopicMap["agent-1"]).toBe("topic-new");
  });

  it("ignores a late topic lookup from an older owner selection", async () => {
    const assistant = useAssistantStore();
    assistant.agents = [
      { id: "agent-a", name: "Agent A" } as any,
      { id: "agent-b", name: "Agent B" } as any,
    ];
    let resolveA!: (topics: unknown[]) => void;
    let resolveB!: (topics: unknown[]) => void;
    mockInvoke(
      "get_topics",
      (args) =>
        new Promise((resolve) => {
          if (args?.ownerId === "agent-a") resolveA = resolve;
          else resolveB = resolve;
        }),
    );
    const session = useChatSessionStore();

    const selectA = session.selectItem(assistant.agents[0]);
    const selectB = session.selectItem(assistant.agents[1]);
    resolveB([{ id: "topic-b" }]);
    await selectB;
    resolveA([{ id: "topic-a" }]);
    await selectA;

    expect(session.currentSelectedItem?.id).toBe("agent-b");
    expect(session.currentTopicId).toBe("topic-b");
  });

  it("clears selection and rejects late stream events after deleting an owner", async () => {
    mockInvoke("delete_agent", () => true);
    const assistant = useAssistantStore();
    assistant.agents = [{ id: "agent-1", name: "Agent 1" } as any];
    const session = useChatSessionStore();
    const stream = useChatStreamStore();
    session.currentSelectedItem = {
      id: "agent-1",
      name: "Agent 1",
      type: "agent",
    };
    session.currentTopicId = "topic-1";
    session.lastActiveTopicMap["agent-1"] = "topic-1";
    stream.beginGeneration("agent-1", "agent", "topic-1", "request-1");
    stream.addSessionStream("agent-1", "topic-1", "message-1");

    await assistant.deleteAgent("agent-1");

    expect(session.currentSelectedItem).toBeNull();
    expect(session.currentTopicId).toBeNull();
    expect(session.lastActiveTopicMap["agent-1"]).toBeUndefined();
    expect(stream.pendingGenerations["agent-1:topic-1"]).toBeUndefined();
    expect(stream.sessionActiveStreams["agent-1:topic-1"]).toBeUndefined();

    await stream.processStreamEvent({
      type: "data",
      messageId: "late-message",
      chunk: "late chunk",
      context: { agentId: "agent-1", topicId: "topic-1" },
    });
    expect(stream.activeStreamMessages.has("late-message")).toBe(false);
  });

  it("does not let a late share topic override a newer owner selection", async () => {
    const assistant = useAssistantStore();
    assistant.agents = [
      { id: "agent-a", name: "Agent A" } as any,
      { id: "agent-b", name: "Agent B" } as any,
    ];
    const creation = deferred<{ id: string }>();
    mockInvoke("create_topic", () => creation.promise);
    mockInvoke("delete_topic", () => undefined);
    mockInvoke("get_topics", () => [{ id: "topic-b" }]);
    const session = useChatSessionStore();

    const share = session.startShareSession("agent-a", "shared", []);
    await Promise.resolve();
    await session.selectItem(assistant.agents[1]);
    creation.resolve({ id: "superseded-share-topic" });

    await expect(share).rejects.toThrow("分享会话已被新的选择取代");
    expect(session.currentSelectedItem?.id).toBe("agent-b");
    expect(session.currentTopicId).toBe("topic-b");
    expect(session.sharePrefillText).toBe("");
    await vi.waitFor(() =>
      expect(session.currentSelectedItem?.id).toBe("agent-b"),
    );
  });
});
