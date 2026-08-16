import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useAttachmentStore } from "@/core/stores/attachmentStore";
import { useChatHistoryStore } from "@/core/stores/chatHistoryStore";
import { useChatSessionStore } from "@/core/stores/chatSessionStore";
import { useSettingsStore } from "@/core/stores/settings";
import { useTopicStore } from "@/core/stores/topicListManager";
import type { Attachment, ChatMessage, ContentBlock } from "@/core/types/chat";
import { invokeMock, mockInvoke } from "@/tests/mocks/tauri";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function selectSession(ownerId: string, topicId: string) {
  const session = useChatSessionStore();
  session.currentSelectedItem = {
    id: ownerId,
    name: ownerId,
    type: "agent",
  };
  session.currentTopicId = topicId;
}

function message(id: string, content = id): ChatMessage {
  return {
    id,
    role: "user",
    content,
    timestamp: 1,
    blocks: [{ type: "markdown", content }],
  };
}

function attachment(id: string): Attachment {
  return {
    id,
    type: "image/jpeg",
    name: `${id}.jpg`,
    size: 1,
    hash: `hash-${id}`,
    internalPath: `/attachments/${id}.jpg`,
    src: `asset://localhost/attachments/${id}.jpg`,
    status: "done",
  };
}

describe("chatHistoryStore session isolation", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    useSettingsStore().settings = {
      userName: "User",
      vcpServerUrl: "",
      vcpApiKey: "",
    } as any;
  });

  it("keeps the original target while preprocessing and consumes only that send's attachments", async () => {
    const history = useChatHistoryStore();
    const attachments = useAttachmentStore();
    const preprocess = deferred<void>();
    vi.spyOn(attachments, "preProcessDocuments").mockReturnValue(
      preprocess.promise,
    );
    mockInvoke("append_single_message", () => [
      { type: "markdown", content: "compiled" },
    ]);
    mockInvoke("handle_agent_chat_message", () => ({}));

    selectSession("agent-a", "topic-a");
    attachments.stagedAttachments = [attachment("old")];
    const sendPromise = history.sendMessage("hello");
    await Promise.resolve();

    selectSession("agent-b", "topic-b");
    const topicBMessage = message("topic-b-message");
    history.currentChatHistory = [topicBMessage];
    attachments.stagedAttachments.push(attachment("new"));
    preprocess.resolve();

    await expect(sendPromise).resolves.toBe(true);
    expect(history.currentChatHistory).toEqual([topicBMessage]);
    expect(attachments.stagedAttachments.map((item) => item.id)).toEqual([
      "new",
    ]);
    expect(invokeMock).toHaveBeenCalledWith(
      "append_single_message",
      expect.objectContaining({
        ownerId: "agent-a",
        ownerType: "agent",
        topicId: "topic-a",
      }),
    );
  });

  it("retains staged attachments and does not add a phantom message when persistence fails", async () => {
    const history = useChatHistoryStore();
    const attachments = useAttachmentStore();
    mockInvoke("append_single_message", () => {
      throw new Error("database unavailable");
    });

    selectSession("agent-a", "topic-a");
    attachments.stagedAttachments = [attachment("retry")];

    await expect(history.sendMessage("hello")).resolves.toBe(false);
    expect(attachments.stagedAttachments.map((item) => item.id)).toEqual([
      "retry",
    ]);
    expect(history.currentChatHistory).toEqual([]);
    expect(invokeMock).not.toHaveBeenCalledWith(
      "handle_agent_chat_message",
      expect.anything(),
    );
  });

  it("keeps a persisted user message and consumes attachments when generation fails", async () => {
    const history = useChatHistoryStore();
    const attachments = useAttachmentStore();
    mockInvoke("append_single_message", () => [
      { type: "markdown", content: "compiled" },
    ]);
    mockInvoke("handle_agent_chat_message", () => {
      throw new Error("generation unavailable");
    });

    selectSession("agent-a", "topic-a");
    attachments.stagedAttachments = [attachment("sent")];

    await expect(history.sendMessage("hello")).resolves.toBe(false);
    expect(attachments.stagedAttachments).toEqual([]);
    expect(history.currentChatHistory).toHaveLength(1);
    expect(history.currentChatHistory[0]).toMatchObject({
      role: "user",
      content: "hello",
      attachments: [expect.objectContaining({ id: "sent" })],
      blocks: [{ type: "markdown", content: "compiled" }],
    });
  });

  it("does not delete a message from the newly selected topic after a late backend response", async () => {
    const history = useChatHistoryStore();
    const deletion = deferred<void>();
    mockInvoke("delete_messages", () => deletion.promise);
    selectSession("agent-a", "topic-a");
    history.currentChatHistory = [message("old-message")];

    const deletePromise = history.deleteMessage("old-message");
    await Promise.resolve();
    selectSession("agent-b", "topic-b");
    const newMessage = message("new-message");
    history.currentChatHistory = [newMessage];
    deletion.resolve();

    await deletePromise;
    expect(history.currentChatHistory).toEqual([newMessage]);
    expect(invokeMock).toHaveBeenCalledWith("delete_messages", {
      ownerId: "agent-a",
      ownerType: "agent",
      topicId: "topic-a",
      msgIds: ["old-message"],
    });
  });

  it("does not apply late compiled edit blocks to the newly selected topic", async () => {
    const history = useChatHistoryStore();
    const compilation = deferred<ContentBlock[]>();
    mockInvoke("patch_single_message", () => compilation.promise);
    selectSession("agent-a", "topic-a");
    history.currentChatHistory = [message("old-message", "old")];

    const updatePromise = history.updateMessageContent("old-message", "edited");
    await Promise.resolve();
    selectSession("agent-b", "topic-b");
    const newMessage = message("new-message", "new");
    history.currentChatHistory = [newMessage];
    compilation.resolve([{ type: "markdown", content: "compiled edit" }]);

    await updatePromise;
    expect(history.currentChatHistory).toEqual([newMessage]);
    expect(invokeMock).toHaveBeenCalledWith(
      "patch_single_message",
      expect.objectContaining({
        ownerId: "agent-a",
        ownerType: "agent",
        topicId: "topic-a",
      }),
    );
  });

  it("keeps the original message when an edit cannot be persisted", async () => {
    const history = useChatHistoryStore();
    mockInvoke("patch_single_message", () => {
      throw new Error("database unavailable");
    });
    selectSession("agent-a", "topic-a");
    history.currentChatHistory = [message("message-a", "original")];

    await expect(
      history.updateMessageContent("message-a", "unsaved edit"),
    ).rejects.toThrow("database unavailable");

    expect(history.currentChatHistory[0]).toMatchObject({
      id: "message-a",
      content: "original",
      blocks: [{ type: "markdown", content: "original" }],
    });
  });

  it("reloads persisted history and topic counts when regeneration fails", async () => {
    const history = useChatHistoryStore();
    const topics = useTopicStore();
    const userMessage = message("user-message", "question");
    const assistantMessage = {
      ...message("assistant-message", "answer"),
      role: "assistant",
      timestamp: 2,
    };
    mockInvoke("regenerate_topic_response", () => {
      throw new Error("generation unavailable");
    });
    mockInvoke("load_chat_history_streamed", (args) => {
      const channel = args?.onMessage as {
        emit: (chunk: unknown) => void;
      };
      channel.emit({ message: userMessage, index: 0, is_last: false });
      channel.emit({ message: assistantMessage, index: 1, is_last: true });
      return 2;
    });
    mockInvoke("get_topics_streamed", (args) => {
      const channel = args?.onChunk as {
        emit: (chunk: unknown) => void;
      };
      channel.emit([
        {
          id: "topic-a",
          name: "Topic A",
          createdAt: 1,
          msgCount: 2,
        },
      ]);
    });

    selectSession("agent-a", "topic-a");
    history.currentChatHistory = [userMessage, assistantMessage];
    topics.topics = [
      {
        id: "topic-a",
        ownerId: "agent-a",
        ownerType: "agent",
        name: "Topic A",
        createdAt: 1,
        msgCount: 2,
      },
    ];

    await history.regenerateResponse("assistant-message");

    expect(history.currentChatHistory.map((item) => item.id)).toEqual([
      "user-message",
      "assistant-message",
    ]);
    expect(topics.topics).toHaveLength(1);
    expect(topics.topics[0].msgCount).toBe(2);
    expect(invokeMock).toHaveBeenCalledWith(
      "load_chat_history_streamed",
      expect.objectContaining({
        ownerId: "agent-a",
        ownerType: "agent",
        topicId: "topic-a",
      }),
    );
  });
});
