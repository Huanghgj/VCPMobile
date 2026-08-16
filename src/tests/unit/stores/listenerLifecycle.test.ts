import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { listenMock } from "@/tests/mocks/tauri";
import { useRagObserverStore } from "@/core/stores/ragObserver";
import { useRebuildSessionStore } from "@/core/stores/rebuildSession";
import { useSyncSessionStore } from "@/core/stores/syncSession";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

describe("async Tauri listener lifecycle", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it("unregisters sync listeners that resolve after the panel closes", async () => {
    const registrations = Array.from({ length: 4 }, () =>
      deferred<() => void>(),
    );
    const unlistenFns = registrations.map(() => vi.fn());
    registrations.forEach((registration) => {
      listenMock.mockImplementationOnce(() => registration.promise);
    });

    const store = useSyncSessionStore();
    store.open();
    store.close();
    registrations.forEach((registration, index) => {
      registration.resolve(unlistenFns[index]);
    });
    await Promise.all(registrations.map(({ promise }) => promise));
    await Promise.resolve();

    unlistenFns.forEach((unlisten) => expect(unlisten).toHaveBeenCalledOnce());
  });

  it("unregisters a late rebuild progress listener", async () => {
    const registration = deferred<() => void>();
    const unlisten = vi.fn();
    listenMock.mockImplementationOnce(() => registration.promise);

    const store = useRebuildSessionStore();
    store.open();
    store.close();
    registration.resolve(unlisten);
    await registration.promise;
    await Promise.resolve();

    expect(unlisten).toHaveBeenCalledOnce();
  });

  it("unregisters a RAG listener when destroy wins the registration race", async () => {
    const registration = deferred<() => void>();
    const unlisten = vi.fn();
    listenMock.mockImplementationOnce(() => registration.promise);

    const store = useRagObserverStore();
    const init = store.initListener();
    for (let turn = 0; turn < 10 && listenMock.mock.calls.length === 0; turn += 1) {
      await Promise.resolve();
    }
    expect(listenMock).toHaveBeenCalledOnce();
    store.destroyListener();
    registration.resolve(unlisten);
    await init;

    expect(unlisten).toHaveBeenCalledOnce();
  });
});
