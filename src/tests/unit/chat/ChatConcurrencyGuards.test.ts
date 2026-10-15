import { beforeEach, describe, expect, it } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import {
  MAX_HISTORY_MESSAGES,
  useChatHistoryStore,
} from "@/core/stores/chatHistoryStore";
import { useChatSessionStore } from "@/core/stores/chatSessionStore";
import { useChatStreamStore } from "@/core/stores/chatStreamStore";
import { useSettingsStore } from "@/core/stores/settings";
import { useAssistantStore } from "@/core/stores/assistant";
import { useAttachmentStore } from "@/core/stores/attachmentStore";
import { useNotificationStore } from "@/core/stores/notification";
import {
  channelInstances,
  invokeMock,
  mockInvoke,
} from "@/tests/mocks/tauri";
import { flushPromises } from "@/tests/utils/flush";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

const message = (id: string, topicId: string) => ({
  id,
  role: "user",
  name: "User",
  content: id,
  timestamp: 1,
  topicId,
  blocks: [],
});

describe("chat conversation concurrency guards", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    mockInvoke("get_active_generations", () => []);
  });

  it.each(["response", "rejection"])("keeps the durable cancellation end ahead of a late recovery %s", async outcome => {
    const recovery = deferred<any>();
    let output: { emit: (event: unknown) => void } | undefined;
    mockInvoke("get_active_generations", () => [{
      msgId: "cancelled", topicId: "topic-a", ownerId: "agent-a", ownerType: "agent", createdAt: 1,
    }]);
    mockInvoke("recover_active_generation", args => {
      output = args?.streamChannel as typeof output;
      return recovery.promise;
    });
    const stream = useChatStreamStore();
    await stream.checkAndRecoverInterruptedStreams();
    output?.emit({
      type: "end", messageId: "cancelled", consumerRevision: 100,
      context: { ownerId: "agent-a", ownerType: "agent", topicId: "topic-a" },
      content: "partial\n\n> VCP流式错误: 请求已中止", finishReason: "cancelled_by_user", blocks: [],
    });
    if (outcome === "response") recovery.resolve({ status: "completed", content: "partial" });
    else recovery.reject(new Error("late command delivery failure"));
    await flushPromises();
    const message = stream.getActiveStreamMessage("agent-a", "agent", "topic-a", "cancelled");
    expect(message?.content).toBe("partial\n\n> VCP流式错误: 请求已中止");
    expect(message?.finishReason).toBe("cancelled_by_user");
    expect(stream.isMessageActive("agent-a", "agent", "topic-a", "cancelled")).toBe(false);
  });

  it("receives a group's next speaker after an empty scan and a later conversation selection", async () => {
    let output: { emit: (event: unknown) => void } | undefined;
    mockInvoke("get_active_generations", args => {
      output = args?.streamChannel as typeof output;
      return [];
    });
    mockInvoke("load_chat_history", () => []);
    const stream = useChatStreamStore();
    await stream.checkAndRecoverInterruptedStreams();
    const session = useChatSessionStore();
    session.setConversation({ id: "group-a", type: "group" }, "topic-a");
    const history = useChatHistoryStore();
    await history.loadHistory("group-a", "group", "topic-a");
    output?.emit({
      type: "thinking", messageId: "other-owner", consumerRevision: 101,
      context: { ownerId: "group-b", ownerType: "group", topicId: "topic-a" },
    });
    output?.emit({
      type: "thinking", messageId: "next-speaker", consumerRevision: 102,
      context: { ownerId: "group-a", ownerType: "group", topicId: "topic-a", agentId: "speaker" },
    });
    await flushPromises();
    expect(history.currentChatHistory.map(message => message.id)).toEqual(["next-speaker"]);
    expect(stream.isMessageActive("group-a", "group", "topic-a", "next-speaker")).toBe(true);
  });

  it("reuses the loaded message when the live snapshot arrives before the scan reply", async () => {
    const session = useChatSessionStore();
    session.setConversation({ id: "agent-a", type: "agent" }, "topic-a");
    mockInvoke("load_chat_history", () => [{
      id: "live", role: "assistant", content: "", timestamp: 1, blocks: [],
    }]);
    const history = useChatHistoryStore();
    await history.loadHistory("agent-a", "agent", "topic-a");
    const visibleMessage = history.currentChatHistory[0];
    mockInvoke("get_active_generations", args => {
      const output = args?.streamChannel as { emit: (event: unknown) => void };
      output.emit({
        type: "aurora", messageId: "live", consumerRevision: 111,
        context: { ownerId: "agent-a", ownerType: "agent", topicId: "topic-a" },
        aurora: { kind: "snapshot", streamId: 50, content: "live partial", stableBlocks: [] },
      });
      return [{ msgId: "live", ownerId: "agent-a", ownerType: "agent", topicId: "topic-a", createdAt: 1 }];
    });
    mockInvoke("recover_active_generation", () => ({ status: "attached" }));
    const stream = useChatStreamStore();
    await stream.checkAndRecoverInterruptedStreams();
    await new Promise<void>(resolve => window.setTimeout(resolve, 0));
    expect(stream.getActiveStreamMessage("agent-a", "agent", "topic-a", "live")).toBe(visibleMessage);
    expect(history.currentChatHistory).toHaveLength(1);
    expect(visibleMessage.content).toBe("live partial");
  });

  it("does not reactivate a stale scan row after its durable end arrived", async () => {
    const scan = deferred<any[]>();
    let output: { emit: (event: unknown) => void } | undefined;
    mockInvoke("get_active_generations", args => {
      output = args?.streamChannel as typeof output;
      return scan.promise;
    });
    const stream = useChatStreamStore();
    const scanning = stream.checkAndRecoverInterruptedStreams();
    output?.emit({
      type: "end", messageId: "finished-during-scan", consumerRevision: 120,
      context: { ownerId: "agent-a", ownerType: "agent", topicId: "topic-a" },
      content: "committed", finishReason: "completed", blocks: [],
    });
    scan.resolve([{ msgId: "finished-during-scan", ownerId: "agent-a", ownerType: "agent", topicId: "topic-a", createdAt: 1 }]);
    await scanning;
    expect(invokeMock.mock.calls.some(([command]) => command === "recover_active_generation")).toBe(false);
    expect(stream.isMessageActive("agent-a", "agent", "topic-a", "finished-during-scan")).toBe(false);
    expect(stream.getActiveStreamMessage("agent-a", "agent", "topic-a", "finished-during-scan")?.isReconnecting).toBe(false);
  });

  it("keeps loaded content until a cold recovery snapshot replaces it before live deltas", async () => {
    const session = useChatSessionStore();
    session.setConversation({ id: "agent-a", type: "agent" }, "topic-a");
    mockInvoke("load_chat_history", () => [{
      id: "recovering", role: "assistant", content: "saved partial", timestamp: 1,
      blocks: [{ type: "markdown", content: "saved partial" }],
    }]);
    const history = useChatHistoryStore();
    await history.loadHistory("agent-a", "agent", "topic-a");
    mockInvoke("get_active_generations", () => [{
      msgId: "recovering", topicId: "topic-a", ownerId: "agent-a", ownerType: "agent", createdAt: 1,
    }]);
    const recovery = deferred<any>();
    let output: { emit: (event: unknown) => void } | undefined;
    mockInvoke("recover_active_generation", args => {
      output = args?.streamChannel as typeof output;
      return recovery.promise;
    });
    const stream = useChatStreamStore();
    await stream.checkAndRecoverInterruptedStreams();
    expect(history.currentChatHistory[0]?.content).toBe("saved partial");
    expect(history.currentChatHistory[0]?.blocks?.[0]).toMatchObject({ content: "saved partial" });

    const event = {
      type: "aurora", messageId: "recovering", consumerRevision: 130,
      context: { ownerId: "agent-a", ownerType: "agent", topicId: "topic-a" },
    };
    output?.emit({ ...event, aurora: {
      kind: "snapshot", streamId: 60, content: "helper snapshot", stableBlocks: [],
      tailBlock: { type: "markdown", content: "helper snapshot", hash: "base" },
    } });
    output?.emit({ ...event, aurora: {
      kind: "delta", streamId: 60, chunk: " + live",
      tailOp: { op: "append", content: " + live", previousHash: "base", hash: "next", mode: "plain", blockType: "markdown" },
    } });
    await new Promise<void>(resolve => window.setTimeout(resolve, 0));
    expect(history.currentChatHistory).toHaveLength(1);
    expect(history.currentChatHistory[0]?.content).toBe("helper snapshot + live");
    expect(history.currentChatHistory[0]?.tailContent).toBe("helper snapshot + live");
    expect(history.currentChatHistory[0]?.blocks).toEqual([]);
    recovery.resolve({ status: "attached" });
    await flushPromises();
  });

  it("preserves new content on recovery rejection and clears its temporary error on the next pending scan", async () => {
    const session = useChatSessionStore();
    session.setConversation({ id: "agent-a", type: "agent" }, "topic-a");
    const stream = useChatStreamStore();
    const output = useChatHistoryStore().makeStreamChannel(null);
    const event = {
      type: "aurora", messageId: "partial", consumerRevision: 140,
      context: { ownerId: "agent-a", ownerType: "agent", topicId: "topic-a" },
    };
    output.onmessage?.({ ...event, aurora: {
      kind: "snapshot", streamId: 70, content: "old", stableBlocks: [],
    } } as any);
    await new Promise<void>(resolve => window.setTimeout(resolve, 0));
    mockInvoke("get_active_generations", () => [{
      msgId: "partial", topicId: "topic-a", ownerId: "agent-a", ownerType: "agent", createdAt: 1,
    }]);
    const recovery = deferred<any>();
    mockInvoke("recover_active_generation", () => recovery.promise);
    mockInvoke("process_message_content", args => [{ type: "markdown", content: args?.content }]);
    await stream.checkAndRecoverInterruptedStreams();
    output.onmessage?.({ ...event, aurora: {
      kind: "delta", streamId: 70, chunk: " + newly received",
    } } as any);
    recovery.reject(new Error("connection unavailable"));
    await flushPromises();
    const partial = stream.getActiveStreamMessage("agent-a", "agent", "topic-a", "partial");
    expect(partial?.content).toBe("old + newly received");
    expect(partial?.blocks).toEqual([{ type: "markdown", content: "old + newly received" }]);
    expect(partial?.isReconnecting).toBe(false);
    expect(partial?.finishReason).toBe("error");
    expect(stream.isMessageActive("agent-a", "agent", "topic-a", "partial")).toBe(false);
    expect(useNotificationStore().activeToasts).toEqual(expect.arrayContaining([
      expect.objectContaining({ title: "流式接续失败", message: "connection unavailable" }),
    ]));

    mockInvoke("recover_active_generation", () => ({ status: "attached" }));
    await stream.checkAndRecoverInterruptedStreams();
    expect(partial?.content).toBe("old + newly received");
    expect(partial?.finishReason).toBeUndefined();
    expect(partial?.isReconnecting).toBe(true);
    output.onmessage?.({ ...event, consumerRevision: 141, aurora: {
      kind: "snapshot", streamId: 71, content: "reattached content", stableBlocks: [],
    } } as any);
    await new Promise<void>(resolve => window.setTimeout(resolve, 0));
    expect(partial?.content).toBe("reattached content");
    expect(partial?.finishReason).toBeUndefined();
    expect(partial?.isReconnecting).toBe(false);
    expect(stream.isMessageActive("agent-a", "agent", "topic-a", "partial")).toBe(true);
  });

  it.each(["response", "rejection"])("keeps a committed deletion ahead of pending frames and a late recovery %s", async outcome => {
    useAssistantStore().agents = [{ id: "agent-a", name: "Agent A", model: "test", avatarCalculatedColor: null }];
    const session = useChatSessionStore();
    session.setConversation({ id: "agent-a", type: "agent" }, "topic-a");
    mockInvoke("load_chat_history", () => [{
      id: "deleted", role: "assistant", content: "partial", timestamp: 1, blocks: [],
    }]);
    const history = useChatHistoryStore();
    await history.loadHistory("agent-a", "agent", "topic-a");
    mockInvoke("get_active_generations", () => [{
      msgId: "deleted", topicId: "topic-a", ownerId: "agent-a", ownerType: "agent", createdAt: 1,
    }]);
    const recovery = deferred<any>();
    let output: { emit: (event: unknown) => void } | undefined;
    mockInvoke("recover_active_generation", args => {
      output = args?.streamChannel as typeof output;
      return recovery.promise;
    });
    mockInvoke("delete_messages", () => ({ msgCount: 0, updatedAt: 2 }));
    mockInvoke("get_topics_streamed", args => {
      const topics = args?.onChunk as { emit: (event: unknown) => void };
      topics.emit([{ id: "topic-a", name: "Topic A", createdAt: 1 }]);
    });
    const stream = useChatStreamStore();
    await stream.checkAndRecoverInterruptedStreams();
    const context = { ownerId: "agent-a", ownerType: "agent", topicId: "topic-a" };
    output?.emit({
      type: "aurora", messageId: "deleted", consumerRevision: 150, context,
      aurora: { kind: "snapshot", streamId: 80, content: "pending frame", stableBlocks: [] },
    });
    await history.deleteMessage(history.captureMessageActionKey("deleted")!);
    output?.emit({
      type: "end", messageId: "deleted", consumerRevision: 150, context,
      content: "late terminal", finishReason: "completed", blocks: [],
    });
    if (outcome === "response") recovery.resolve({ status: "completed", content: "late result" });
    else recovery.reject(new Error("late rejection"));
    await flushPromises();
    await new Promise<void>(resolve => window.setTimeout(resolve, 0));
    expect(history.currentChatHistory).toEqual([]);
    expect(stream.getActiveStreamMessage("agent-a", "agent", "topic-a", "deleted")).toBeUndefined();
    expect(stream.isMessageActive("agent-a", "agent", "topic-a", "deleted")).toBe(false);
    expect(useNotificationStore().activeToasts).toEqual([]);
    await stream.checkAndRecoverInterruptedStreams();
    expect(invokeMock.mock.calls.filter(([command]) => command === "recover_active_generation")).toHaveLength(1);
  });

  it("inserts an offline durable end delivered during the startup scan without a recovery placeholder", async () => {
    const session = useChatSessionStore();
    session.setConversation({ id: "agent-a", type: "agent" }, "topic-a");
    mockInvoke("load_chat_history", () => []);
    const history = useChatHistoryStore();
    await history.loadHistory("agent-a", "agent", "topic-a");
    mockInvoke("get_active_generations", args => {
      const output = args?.streamChannel as { emit: (event: unknown) => void };
      output.emit({
        type: "end", messageId: "offline", consumerRevision: 160,
        context: { ownerId: "agent-a", ownerType: "agent", topicId: "topic-a" },
        content: "offline result", finishReason: "completed",
        blocks: [{ type: "markdown", content: "offline result" }],
      });
      return [];
    });
    await useChatStreamStore().checkAndRecoverInterruptedStreams();
    expect(history.currentChatHistory).toEqual([
      expect.objectContaining({ id: "offline", content: "offline result", isReconnecting: false }),
    ]);
    expect(invokeMock.mock.calls.some(([command]) => command === "recover_active_generation")).toBe(false);
  });

  it("invalidates an A snapshot even when selection later returns to A", () => {
    const session = useChatSessionStore();
    session.setConversation({ id: "agent-a", type: "agent" }, "topic-a");
    const firstA = { ...session.currentConversationKey! };

    session.setConversation({ id: "agent-b", type: "agent" }, "topic-b");
    session.setConversation({ id: "agent-a", type: "agent" }, "topic-a");

    expect(session.currentConversationKey?.epoch).toBeGreaterThan(firstA.epoch);
    expect(session.isConversationCurrent(firstA)).toBe(false);
  });

  it("refreshes owner presentation without changing conversation identity", () => {
    const session = useChatSessionStore();
    session.setConversation(
      {
        id: "agent-a",
        name: "Old name",
        type: "agent",
        avatarCalculatedColor: "#111111",
      },
      "topic-a",
    );
    const originalKey = session.currentConversationKey;

    session.setConversation(
      {
        id: "agent-a",
        name: "New name",
        type: "agent",
        avatarCalculatedColor: "#222222",
      },
      "topic-a",
    );

    expect(session.currentConversationKey).toBe(originalKey);
    expect(session.currentSelectedItem).toMatchObject({
      name: "New name",
      avatarCalculatedColor: "#222222",
    });
  });

  it("does not let a slow owner selection overwrite a newer selection", async () => {
    const session = useChatSessionStore();
    const assistant = useAssistantStore();
    assistant.agents = [
      { id: "agent-b", name: "B", model: "test", avatarCalculatedColor: null },
      { id: "agent-c", name: "C", model: "test", avatarCalculatedColor: null },
    ];
    const topicsB = deferred<void>();
    const topicsC = deferred<void>();
    mockInvoke("get_topics_streamed", (args) =>
      args?.ownerId === "agent-b" ? topicsB.promise : topicsC.promise,
    );

    const selectingB = session.selectItem({ id: "agent-b", name: "B", type: "agent" });
    const selectingC = session.selectItem({ id: "agent-c", name: "C", type: "agent" });
    channelInstances[1]?.emit([{
      id: "topic-c",
      name: "Topic C",
    }]);
    topicsC.resolve();
    await selectingC;
    channelInstances[0]?.emit([{
      id: "topic-b",
      name: "Topic B",
    }]);
    topicsB.resolve();
    await selectingB;

    expect(session.currentConversationKey).toMatchObject({
      ownerId: "agent-c",
      topicId: "topic-c",
    });
  });

  it("does not let an old load settle the new conversation latch or messages", async () => {
    const session = useChatSessionStore();
    const history = useChatHistoryStore();
    const loadA = deferred<any[]>();
    const loadB = deferred<any[]>();
    mockInvoke("load_chat_history", (args) =>
      args?.topicId === "topic-a" ? loadA.promise : loadB.promise,
    );

    session.setConversation({ id: "agent-a", type: "agent" }, "topic-a");
    const pendingA = history.loadHistoryPaginated(
      "agent-a",
      "agent",
      "topic-a",
    );
    session.setConversation({ id: "agent-b", type: "agent" }, "topic-b");
    const pendingB = history.loadHistoryPaginated(
      "agent-b",
      "agent",
      "topic-b",
    );

    loadA.resolve([message("message-a", "topic-a")]);
    await pendingA;
    expect(history.isLoadingHistory).toBe(true);
    expect(history.currentChatHistory).toEqual([]);

    loadB.resolve([message("message-b", "topic-b")]);
    await pendingB;
    expect(history.isLoadingHistory).toBe(false);
    expect(history.currentChatHistory.map((item) => item.id)).toEqual([
      "message-b",
    ]);
    expect(history.loadedConversationKey?.topicId).toBe("topic-b");
  });

  it("keeps a send bound to its entry conversation across an await", async () => {
    const session = useChatSessionStore();
    const history = useChatHistoryStore();
    const settings = useSettingsStore();
    settings.settings = {
      userName: "User",
      vcpServerUrl: "https://example.invalid",
      vcpApiKey: "key",
    } as any;
    mockInvoke("load_chat_history", () => []);

    session.setConversation({ id: "agent-a", type: "agent" }, "topic-a");
    await history.loadHistoryPaginated("agent-a", "agent", "topic-a");

    const append = deferred<{ blocks: any[]; topicUpdatedAt: number }>();
    mockInvoke("append_single_message", () => append.promise);
    const pendingSend = history.sendMessage("hello");

    session.setConversation({ id: "agent-b", type: "agent" }, "topic-b");
    history.resetHistoryForConversation();
    append.resolve({ blocks: [], topicUpdatedAt: 100 });
    await pendingSend;

    const appendCall = invokeMock.mock.calls.find(
      ([command]) => command === "append_single_message",
    );
    expect(appendCall?.[1]).toMatchObject({
      ownerId: "agent-a",
      ownerType: "agent",
      topicId: "topic-a",
    });
    const generationCall = invokeMock.mock.calls.find(
      ([command]) => command === "handle_agent_chat_message",
    );
    expect(generationCall?.[1]).toMatchObject({
      payload: { agentId: "agent-a", topicId: "topic-a" },
    });
    expect(history.currentChatHistory).toEqual([]);
  });

  it("loads older history with a stable keyset cursor and bounded window", async () => {
    const session = useChatSessionStore();
    const history = useChatHistoryStore();
    const recent = Array.from({ length: MAX_HISTORY_MESSAGES }, (_, index) => ({
      ...message(`message-${String(index).padStart(4, "0")}`, "topic-a"),
      timestamp: index + 100,
    }));
    let page = 0;
    mockInvoke("load_chat_history", () => {
      page += 1;
      return page === 1
        ? recent
        : [{ ...message("message-older", "topic-a"), timestamp: 1 }];
    });

    session.setConversation({ id: "agent-a", type: "agent" }, "topic-a");
    await history.loadHistoryPaginated("agent-a", "agent", "topic-a");
    await history.loadMoreHistory();

    const historyCalls = invokeMock.mock.calls.filter(
      ([command]) => command === "load_chat_history",
    );
    expect(historyCalls[1]?.[1]).toMatchObject({
      beforeTimestamp: 100,
      beforeMessageId: "message-0000",
      offset: null,
    });
    expect(history.currentChatHistory).toHaveLength(MAX_HISTORY_MESSAGES);
    expect(history.currentChatHistory[0]?.id).toBe("message-older");
    expect(history.currentChatHistory.some(item => item.id === "message-0499")).toBe(false);
    expect(history.hasEvictedNewer).toBe(true);

    mockInvoke("load_chat_history", () => recent.slice(-15));
    await history.returnToLatest();

    const latestCalls = invokeMock.mock.calls.filter(
      ([command]) => command === "load_chat_history",
    );
    const latestCall = latestCalls[latestCalls.length - 1]?.[1];
    expect(latestCall).toMatchObject({
      beforeTimestamp: null,
      beforeMessageId: null,
      offset: 0,
    });
    expect(history.hasEvictedNewer).toBe(false);
    expect(
      history.currentChatHistory[history.currentChatHistory.length - 1]?.id,
    ).toBe("message-0499");
  });

  it("does not clear or send attachments that are still loading", async () => {
    const session = useChatSessionStore();
    const history = useChatHistoryStore();
    const attachments = useAttachmentStore();
    mockInvoke("load_chat_history", () => []);
    session.setConversation({ id: "agent-a", type: "agent" }, "topic-a");
    await history.loadHistoryPaginated("agent-a", "agent", "topic-a");
    attachments.stagedAttachments.push({
      id: "attachment-loading",
      name: "large.pdf",
      type: "application/pdf",
      size: 10,
      src: "file:///large.pdf",
      status: "loading",
    } as any);

    await history.sendMessage("send with attachment");

    expect(attachments.stagedAttachments).toHaveLength(1);
    expect(
      invokeMock.mock.calls.some(([command]) => command === "append_single_message"),
    ).toBe(false);
  });

  it("does not persist a frontend skeleton for a thinking event", async () => {
    const stream = useChatStreamStore();
    await stream.processStreamEvent({
      consumerRevision: 1,
      type: "thinking",
      chunk: null,
      messageId: "assistant-1",
      context: {
        ownerId: "agent-a",
        ownerType: "agent",
        topicId: "topic-a",
        agentId: "agent-a",
      },
      finishReason: null,
      error: null,
      aurora: null,
      blocks: null,
      timestamp: null,
      topicUpdatedAt: null,
    });

    expect(
      invokeMock.mock.calls.some(
        ([command]) => command === "append_single_message",
      ),
    ).toBe(false);
  });

  it("submits stop to the original message and waits for its durable end", async () => {
    const session = useChatSessionStore();
    const stream = useChatStreamStore();
    session.setConversation({ id: "agent-a", type: "agent" }, "topic-a");
    await stream.processStreamEvent({
      consumerRevision: 1,
      type: "thinking",
      chunk: null,
      messageId: "assistant-a",
      context: {
        ownerId: "agent-a",
        ownerType: "agent",
        topicId: "topic-a",
        agentId: "agent-a",
      },
      finishReason: null,
      error: null,
      aurora: null,
      blocks: null,
      timestamp: null,
      topicUpdatedAt: null,
    });

    const interrupt = deferred<unknown>();
    mockInvoke("interruptRequest", () => interrupt.promise);
    const stoppingA = stream.stopMessage(
      "agent-a",
      "agent",
      "topic-a",
      "assistant-a",
    );

    session.setConversation({ id: "agent-b", type: "agent" }, "topic-b");
    await stream.processStreamEvent({
      consumerRevision: 1,
      type: "thinking",
      chunk: null,
      messageId: "assistant-b",
      context: {
        ownerId: "agent-b",
        ownerType: "agent",
        topicId: "topic-b",
        agentId: "agent-b",
      },
      finishReason: null,
      error: null,
      aurora: null,
      blocks: null,
      timestamp: null,
      topicUpdatedAt: null,
    });
    interrupt.resolve(undefined);
    await stoppingA;

    expect(
      stream.isMessageActive("agent-a", "agent", "topic-a", "assistant-a"),
    ).toBe(true);
    expect(
      stream.isMessageActiveInSession(
        "agent-b",
        "agent",
        "topic-b",
        "assistant-b",
      ),
    ).toBe(true);

    await stream.processStreamEvent({
      consumerRevision: 1,
      type: "end",
      chunk: null,
      messageId: "assistant-a",
      context: {
        ownerId: "agent-a",
        ownerType: "agent",
        topicId: "topic-a",
        agentId: "agent-a",
      },
      finishReason: "cancelled_by_user",
      error: null,
      content: "partial\n\n> VCP流式错误: 请求已中止",
      aurora: null,
      blocks: [],
      timestamp: 123,
      topicUpdatedAt: 124,
    });
    expect(
      stream.isMessageActive("agent-a", "agent", "topic-a", "assistant-a"),
    ).toBe(false);
  });

  it("claims a cold recovery once and never starts the removed two-step resume path", async () => {
    const session = useChatSessionStore();
    session.setConversation({ id: "agent-a", type: "agent" }, "topic-a");
    const recovery = deferred<any>();
    mockInvoke("get_active_generations", () => [{
      msgId: "assistant-recovery",
      topicId: "topic-a",
      ownerId: "agent-a",
      ownerType: "agent",
      createdAt: 1,
    }]);
    mockInvoke("recover_active_generation", () => recovery.promise);

    const stream = useChatStreamStore();
    await stream.checkAndRecoverInterruptedStreams();
    await stream.checkAndRecoverInterruptedStreams();

    const recoveryCalls = invokeMock.mock.calls.filter(
      ([command]) => command === "recover_active_generation",
    );
    expect(recoveryCalls).toHaveLength(1);
    expect(recoveryCalls[0]?.[1]).toMatchObject({
      msgId: "assistant-recovery",
    });
    expect(recoveryCalls[0]?.[1]).not.toHaveProperty("isWarm");
    expect(
      invokeMock.mock.calls.some(([command]) => command === "resume_stream"),
    ).toBe(false);

    recovery.resolve({ status: "completed", content: "recovered" });
    await Promise.resolve();
    await Promise.resolve();
  });

  it("lets the backend finalize a durable turn while offline and fences late recovery frames", async () => {
    const onlineDescriptor = Object.getOwnPropertyDescriptor(navigator, "onLine");
    Object.defineProperty(navigator, "onLine", {
      configurable: true,
      value: false,
    });

    try {
      const session = useChatSessionStore();
      session.setConversation({ id: "agent-a", type: "agent" }, "topic-a");
      mockInvoke("get_active_generations", () => [
        {
          msgId: "assistant-offline-finalizing",
          topicId: "topic-a",
          ownerId: "agent-a",
          ownerType: "agent",
          createdAt: 1,
        },
      ]);

      let recoveryChannel: { emit: (event: unknown) => void } | undefined;
      mockInvoke("recover_active_generation", (args) => {
        recoveryChannel = args?.streamChannel as typeof recoveryChannel;
        recoveryChannel?.emit({
          consumerRevision: 1,
          type: "end",
          messageId: "assistant-offline-finalizing",
          turnAttempt: "attempt-offline",
          stepIndex: 2,
          projectionReset: false,
          finishReason: "completed",
          content: "durable offline result",
          blocks: [],
          context: {
            ownerId: "agent-a",
            ownerType: "agent",
            topicId: "topic-a",
            agentId: "agent-a",
          },
        });
        return { status: "completed", content: "durable offline result" };
      });

      const stream = useChatStreamStore();
      await stream.checkAndRecoverInterruptedStreams();
      await flushPromises();

      expect(
        invokeMock.mock.calls.filter(
          ([command]) => command === "recover_active_generation",
        ),
      ).toHaveLength(1);
      const recovered = stream.getActiveStreamMessage(
        "agent-a",
        "agent",
        "topic-a",
        "assistant-offline-finalizing",
      );
      expect(recovered?.content).toBe("durable offline result");
      expect(recovered?.finishReason).toBe("completed");
      expect(
        stream.isMessageActive(
          "agent-a",
          "agent",
          "topic-a",
          "assistant-offline-finalizing",
        ),
      ).toBe(false);

      recoveryChannel?.emit({
        consumerRevision: 1,
        type: "thinking",
        messageId: "assistant-offline-finalizing",
        turnAttempt: "attempt-offline",
        stepIndex: 2,
        projectionReset: false,
        context: {
          ownerId: "agent-a",
          ownerType: "agent",
          topicId: "topic-a",
          agentId: "agent-a",
        },
      });
      await flushPromises();

      expect(recovered?.isThinking).toBe(false);
      expect(
        stream.isMessageActive(
          "agent-a",
          "agent",
          "topic-a",
          "assistant-offline-finalizing",
        ),
      ).toBe(false);
      expect(recovered?.content).toBe("durable offline result");
    } finally {
      if (onlineDescriptor) {
        Object.defineProperty(navigator, "onLine", onlineDescriptor);
      } else {
        Reflect.deleteProperty(navigator, "onLine");
      }
    }
  });

});
