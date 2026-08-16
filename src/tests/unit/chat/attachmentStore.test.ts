import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  normalizeNativePickedBatch,
  useAttachmentStore,
} from "../../../core/stores/attachmentStore";
import type { Attachment } from "../../../core/types/chat";
import { invokeMock, mockInvoke } from "../../mocks/tauri";

describe("attachmentStore native picker", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it("normalizes legacy single-file and batch responses", () => {
    const file = {
      path: "/cache/a.jpg",
      name: "a.jpg",
      mime: "image/jpeg",
      size: 10,
      hash: "hash-a",
    };

    expect(normalizeNativePickedBatch(file)).toEqual({
      files: [file],
      errors: [],
    });
    expect(
      normalizeNativePickedBatch({
        files: [file],
        errors: [{ message: "bad" }],
      }),
    ).toEqual({ files: [file], errors: [{ message: "bad" }] });
  });

  it("prepares registered video files for WebView playback", async () => {
    mockInvoke("prepare_attachment_asset", () => "/attachments/e322d33d.mp4");
    const message: { attachments: Attachment[] } = {
      attachments: [
        {
          type: "video/mp4",
          src: "/attachments/e322d33d.mp4",
          internalPath: "/attachments/e322d33d.mp4",
          name: "e322d33d.mp4",
          size: 1024,
        },
      ],
    };

    const store = useAttachmentStore();
    await store.resolveMessageAssets(message);

    expect(invokeMock).toHaveBeenCalledWith("prepare_attachment_asset", {
      path: "/attachments/e322d33d.mp4",
    });
    expect(message.attachments[0].resolvedSrc).toBe(
      "asset:///attachments/e322d33d.mp4",
    );
  });

  it("registers every image returned by the Android gallery picker", async () => {
    vi.spyOn(window.navigator, "userAgent", "get").mockReturnValue("Android");
    const files = [
      {
        nativeId: "native-a",
        path: "/cache/a.jpg",
        name: "a.jpg",
        mime: "image/jpeg",
        size: 10,
        hash: "hash-a",
      },
      {
        nativeId: "native-b",
        path: "/cache/b.jpg",
        name: "b.jpg",
        mime: "image/jpeg",
        size: 20,
        hash: "hash-b",
      },
    ];
    mockInvoke("plugin:vcp-mobile|pick_file", () => ({ files, errors: [] }));
    mockInvoke("register_local_file", (args) => {
      const localPath = String(args?.localPath);
      const picked = files.find((file) => file.path === localPath)!;
      return {
        type: picked.mime,
        internalPath: `/attachments/${picked.name}`,
        thumbnailPath: null,
        name: picked.name,
        size: picked.size,
        hash: picked.hash,
      };
    });
    mockInvoke(
      "read_image_preview_data_url",
      () => "data:image/jpeg;base64,AA==",
    );

    const store = useAttachmentStore();
    await store.handleAttachment("gallery");

    expect(
      invokeMock.mock.calls.filter(
        ([command]) => command === "register_local_file",
      ),
    ).toHaveLength(2);
    expect(store.stagedAttachments).toHaveLength(2);
    expect(
      store.stagedAttachments.every(
        (attachment) => attachment.status === "done",
      ),
    ).toBe(true);
    expect(
      store.stagedAttachments.map((attachment) => attachment.name).sort(),
    ).toEqual(["a.jpg", "b.jpg"]);
  });

  it("cleans the native cache file when registration fails", async () => {
    vi.spyOn(window.navigator, "userAgent", "get").mockReturnValue("Android");
    mockInvoke("plugin:vcp-mobile|pick_file", () => ({
      files: [
        {
          nativeId: "native-a",
          path: "/cache/a.jpg",
          name: "a.jpg",
          mime: "image/jpeg",
          size: 10,
          hash: "hash-a",
        },
      ],
      errors: [],
    }));
    mockInvoke("check_attachment_support", () => true);
    mockInvoke("register_local_file", () => {
      throw new Error("registration failed");
    });
    mockInvoke("plugin:vcp-mobile|delete_temp_file", () => undefined);

    const store = useAttachmentStore();
    await store.handleAttachment("gallery");

    expect(invokeMock).toHaveBeenCalledWith(
      "plugin:vcp-mobile|delete_temp_file",
      { filePath: "/cache/a.jpg" },
    );
    expect(store.stagedAttachments).toEqual([]);
  });

  it("allows only one native picker request at a time", async () => {
    vi.spyOn(window.navigator, "userAgent", "get").mockReturnValue("Android");
    let resolvePicker!: (value: unknown) => void;
    mockInvoke(
      "plugin:vcp-mobile|pick_file",
      () =>
        new Promise((resolve) => {
          resolvePicker = resolve;
        }),
    );

    const store = useAttachmentStore();
    const firstPick = store.handleAttachment("gallery");
    await Promise.resolve();

    await store.handleAttachment("file");
    expect(
      invokeMock.mock.calls.filter(
        ([command]) => command === "plugin:vcp-mobile|pick_file",
      ),
    ).toHaveLength(1);
    expect(store.isPickingAttachment).toBe(true);

    resolvePicker({ files: [], errors: [] });
    await firstPick;
    expect(store.isPickingAttachment).toBe(false);
  });

  it("unlocks after the native cancellation event even if invoke is still pending", async () => {
    vi.spyOn(window.navigator, "userAgent", "get").mockReturnValue("Android");
    mockInvoke(
      "plugin:vcp-mobile|pick_file",
      () => new Promise(() => undefined),
    );

    const store = useAttachmentStore();
    const pickPromise = store.handleAttachment("gallery");
    await Promise.resolve();

    const pickerCall = invokeMock.mock.calls.find(
      ([command]) => command === "plugin:vcp-mobile|pick_file",
    );
    const requestId = String(pickerCall?.[1]?.requestId);
    window.dispatchEvent(
      new CustomEvent("vcp-mobile-file-picker-dismissed", {
        detail: { requestId, reason: "picker_cancelled" },
      }),
    );

    await pickPromise;
    expect(store.isPickingAttachment).toBe(false);
    expect(store.stagedAttachments).toEqual([]);
  });

  it("unlocks when the native picker cannot be launched", async () => {
    vi.spyOn(window.navigator, "userAgent", "get").mockReturnValue("Android");
    mockInvoke("plugin:vcp-mobile|pick_file", () => {
      throw new Error("No compatible picker installed");
    });

    const store = useAttachmentStore();
    await store.handleAttachment("gallery");

    expect(store.isPickingAttachment).toBe(false);
    expect(store.stagedAttachments).toEqual([]);
  });
});

describe("attachmentStore HTML picker", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.spyOn(window.navigator, "userAgent", "get").mockReturnValue("Desktop");
  });

  const installPickerFiles = (files: File[]) => {
    const originalCreateElement = document.createElement.bind(document);
    vi.spyOn(document, "createElement").mockImplementation(
      (tagName: string) => {
        const element = originalCreateElement(tagName);
        if (tagName.toLowerCase() !== "input") return element;

        const input = element as HTMLInputElement;
        Object.defineProperty(input, "files", {
          configurable: true,
          value: files,
        });
        vi.spyOn(input, "click").mockImplementation(() => {
          input.dispatchEvent(new Event("change"));
        });
        return input;
      },
    );
  };

  const storedFile = (name: string) => ({
    type: "image/gif",
    internalPath: `/attachments/${name}`,
    thumbnailPath: null,
    name,
    size: 4,
    hash: `hash-${name}`,
  });

  it("stores every image selected in one gallery action", async () => {
    const files = [
      new File([new Uint8Array([1])], "one.gif", { type: "image/gif" }),
      new File([new Uint8Array([2])], "two.gif", { type: "image/gif" }),
    ];
    installPickerFiles(files);
    mockInvoke("check_attachment_support", () => undefined);
    mockInvoke("store_file", (args) => storedFile(String(args?.originalName)));
    mockInvoke(
      "read_image_preview_data_url",
      () => "data:image/png;base64,AA==",
    );

    const store = useAttachmentStore();
    await store.handleAttachment("gallery");

    expect(
      invokeMock.mock.calls.filter(([command]) => command === "store_file"),
    ).toHaveLength(2);
    expect(store.stagedAttachments).toHaveLength(2);
    expect(store.stagedAttachments.map((item) => item.name).sort()).toEqual([
      "one.gif",
      "two.gif",
    ]);
  });

  it("keeps supported files when another selected file is rejected", async () => {
    const files = [
      new File([new Uint8Array([1])], "good.gif", { type: "image/gif" }),
      new File([new Uint8Array([2])], "bad.exe", {
        type: "application/octet-stream",
      }),
    ];
    installPickerFiles(files);
    mockInvoke("check_attachment_support", (args) => {
      if (args?.originalName === "bad.exe") throw new Error("unsupported");
    });
    mockInvoke("store_file", (args) => storedFile(String(args?.originalName)));
    mockInvoke(
      "read_image_preview_data_url",
      () => "data:image/png;base64,AA==",
    );

    const store = useAttachmentStore();
    await store.handleAttachment("file");

    expect(
      invokeMock.mock.calls.filter(([command]) => command === "store_file"),
    ).toHaveLength(1);
    expect(store.stagedAttachments.map((item) => item.name)).toEqual([
      "good.gif",
    ]);
  });

  it("settles a cancelled picker when an older WebView only restores focus", async () => {
    vi.useFakeTimers();
    try {
      const originalCreateElement = document.createElement.bind(document);
      vi.spyOn(document, "createElement").mockImplementation(
        (tagName: string) => {
          const element = originalCreateElement(tagName);
          if (tagName.toLowerCase() === "input") {
            vi.spyOn(element as HTMLInputElement, "click").mockImplementation(
              () => undefined,
            );
          }
          return element;
        },
      );
      const store = useAttachmentStore();
      const pickPromise = store.handleAttachment("file");

      window.dispatchEvent(new Event("focus"));
      await vi.advanceTimersByTimeAsync(300);
      await pickPromise;

      expect(store.isPickingAttachment).toBe(false);
      expect(store.stagedAttachments).toEqual([]);
    } finally {
      vi.useRealTimers();
    }
  });
});
