import { beforeEach, describe, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { useTopicStore } from '@/core/stores/topicListManager';
import { useChatSessionStore } from '@/core/stores/chatSessionStore';
import { useChatStreamStore } from '@/core/stores/chatStreamStore';
import { invokeMock, mockInvoke } from '@/tests/mocks/tauri';

describe('topicListManager.deleteTopic', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it('invokes the backend and removes the deleted topic after success', async () => {
    mockInvoke('delete_topic', () => undefined);
    const store = useTopicStore();
    store.topics = [
      {
        id: 'topic-alpha',
        ownerId: 'agent-alpha',
        ownerType: 'agent',
        name: 'Alpha',
        createdAt: 1,
      },
      {
        id: 'topic-beta',
        ownerId: 'agent-alpha',
        ownerType: 'agent',
        name: 'Beta',
        createdAt: 2,
      },
    ];

    await store.deleteTopic('agent-alpha', 'agent', 'topic-alpha');

    expect(invokeMock).toHaveBeenCalledWith('delete_topic', {
      ownerId: 'agent-alpha',
      ownerType: 'agent',
      topicId: 'topic-alpha',
    });
    expect(store.topics.map((topic) => topic.id)).toEqual(['topic-beta']);
  });

  it('keeps the topic when the backend deletion fails', async () => {
    mockInvoke('delete_topic', () => {
      throw new Error('database failure');
    });
    const store = useTopicStore();
    store.topics = [
      {
        id: 'topic-alpha',
        ownerId: 'agent-alpha',
        ownerType: 'agent',
        name: 'Alpha',
        createdAt: 1,
      },
    ];

    await expect(
      store.deleteTopic('agent-alpha', 'agent', 'topic-alpha'),
    ).rejects.toThrow('database failure');
    expect(store.topics.map((topic) => topic.id)).toEqual(['topic-alpha']);
  });

  it('clears a persisted last-active topic when that topic is deleted', async () => {
    mockInvoke('delete_topic', () => undefined);
    const store = useTopicStore();
    const sessionStore = useChatSessionStore();
    store.topics = [{
      id: 'topic-alpha',
      ownerId: 'agent-alpha',
      ownerType: 'agent',
      name: 'Alpha',
      createdAt: 1,
    }];
    sessionStore.currentTopicId = 'topic-alpha';
    sessionStore.lastActiveTopicMap['agent-alpha'] = 'topic-alpha';

    await store.deleteTopic('agent-alpha', 'agent', 'topic-alpha');

    expect(sessionStore.currentTopicId).toBeNull();
    expect(sessionStore.lastActiveTopicMap['agent-alpha']).toBeUndefined();
  });

  it('clears pending and active stream state for a deleted topic', async () => {
    mockInvoke('delete_topic', () => undefined);
    const store = useTopicStore();
    const streamStore = useChatStreamStore();
    store.topics = [{
      id: 'topic-alpha',
      ownerId: 'agent-alpha',
      ownerType: 'agent',
      name: 'Alpha',
      createdAt: 1,
    }];
    streamStore.beginGeneration('agent-alpha', 'agent', 'topic-alpha', 'request-alpha');
    streamStore.addSessionStream('agent-alpha', 'topic-alpha', 'message-alpha');

    await store.deleteTopic('agent-alpha', 'agent', 'topic-alpha');

    expect(streamStore.pendingGenerations['agent-alpha:topic-alpha']).toBeUndefined();
    expect(streamStore.sessionActiveStreams['agent-alpha:topic-alpha']).toBeUndefined();

    await streamStore.processStreamEvent({
      type: 'data',
      messageId: 'late-message',
      chunk: 'late chunk',
      context: { agentId: 'agent-alpha', topicId: 'topic-alpha' },
    });
    expect(streamStore.activeStreamMessages.has('late-message')).toBe(false);
  });

  it('does not reinsert a deleted topic from a late streamed chunk', async () => {
    let channel: { emit: (topics: any[]) => void } | undefined;
    let finishLoad: (() => void) | undefined;
    mockInvoke('get_topics_streamed', (args) => {
      channel = args?.onChunk as typeof channel;
      return new Promise<void>((resolve) => {
        finishLoad = resolve;
      });
    });
    mockInvoke('delete_topic', () => undefined);

    const store = useTopicStore();
    const loading = store.loadTopicList('agent-alpha', 'agent');
    await vi.waitFor(() => expect(channel).toBeDefined());
    channel!.emit([{
      id: 'topic-alpha',
      name: 'Alpha',
      createdAt: 1,
    }]);
    expect(store.topics.map((topic) => topic.id)).toEqual(['topic-alpha']);

    await store.deleteTopic('agent-alpha', 'agent', 'topic-alpha');
    channel!.emit([{
      id: 'topic-alpha',
      name: 'Alpha',
      createdAt: 1,
    }]);
    finishLoad!();
    await loading;

    expect(store.topics).toEqual([]);
  });

  it('discards an older reload for the same owner', async () => {
    const channels: Array<{ emit: (topics: any[]) => void }> = [];
    const finishLoads: Array<() => void> = [];
    mockInvoke('get_topics_streamed', (args) => {
      channels.push(args?.onChunk as (typeof channels)[number]);
      return new Promise<void>((resolve) => finishLoads.push(resolve));
    });

    const store = useTopicStore();
    const firstLoad = store.loadTopicList('agent-alpha', 'agent');
    await vi.waitFor(() => expect(channels).toHaveLength(1));
    const secondLoad = store.loadTopicList('agent-alpha', 'agent');
    await vi.waitFor(() => expect(channels).toHaveLength(2));

    channels[1].emit([{ id: 'topic-new', name: 'New', createdAt: 2 }]);
    finishLoads[1]();
    await secondLoad;
    channels[0].emit([{ id: 'topic-old', name: 'Old', createdAt: 1 }]);
    finishLoads[0]();
    await firstLoad;

    expect(store.topics.map((topic) => topic.id)).toEqual(['topic-new']);
  });

  it('does not mutate the visible owner after an asynchronous delete returns', async () => {
    let finishDelete!: () => void;
    mockInvoke('delete_topic', () => new Promise<void>((resolve) => {
      finishDelete = resolve;
    }));
    const store = useTopicStore();
    const session = useChatSessionStore();
    store.currentAgentId = 'agent-alpha';
    session.currentSelectedItem = { id: 'agent-alpha', type: 'agent' };
    store.topics = [{
      id: 'topic-alpha',
      ownerId: 'agent-alpha',
      ownerType: 'agent',
      name: 'Alpha',
      createdAt: 1,
    }];

    const deletion = store.deleteTopic('agent-alpha', 'agent', 'topic-alpha');
    await vi.waitFor(() => expect(finishDelete).toBeDefined());
    store.currentAgentId = 'agent-beta';
    session.currentSelectedItem = { id: 'agent-beta', type: 'agent' };
    store.topics = [{
      id: 'topic-beta',
      ownerId: 'agent-beta',
      ownerType: 'agent',
      name: 'Beta',
      createdAt: 2,
    }];

    finishDelete();
    await deletion;

    expect(store.topics.map((topic) => topic.id)).toEqual(['topic-beta']);
    expect(session.currentSelectedItem.id).toBe('agent-beta');
  });

  it('does not insert a newly created topic into a different visible owner', async () => {
    mockInvoke('create_topic', () => ({
      id: 'topic-alpha',
      name: 'Alpha',
      createdAt: 1,
    }));
    const store = useTopicStore();
    const session = useChatSessionStore();
    store.currentAgentId = 'agent-beta';
    session.currentSelectedItem = { id: 'agent-beta', type: 'agent' };
    store.topics = [{
      id: 'topic-beta',
      ownerId: 'agent-beta',
      ownerType: 'agent',
      name: 'Beta',
      createdAt: 2,
    }];

    await store.createTopic('agent-alpha', 'agent', 'Alpha');

    expect(store.topics.map((topic) => topic.id)).toEqual(['topic-beta']);
  });

  it('prefers the selected owner over a stale topic-list owner marker', async () => {
    mockInvoke('create_topic', () => ({
      id: 'topic-alpha',
      name: 'Alpha',
      createdAt: 1,
    }));
    const store = useTopicStore();
    const session = useChatSessionStore();
    store.currentAgentId = 'agent-alpha';
    session.currentSelectedItem = { id: 'agent-beta', type: 'agent' };
    store.topics = [{
      id: 'topic-beta',
      ownerId: 'agent-beta',
      ownerType: 'agent',
      name: 'Beta',
      createdAt: 2,
    }];

    await store.createTopic('agent-alpha', 'agent', 'Alpha');

    expect(store.topics.map((topic) => topic.id)).toEqual(['topic-beta']);
  });
});
