import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";

// ── 类型 ──
export interface DiaryNote {
  name: string;
  lastModified: string; // ISO
  preview: string;
}

export interface DiaryEntry extends DiaryNote {
  folder: string;
}

export interface DiaryNoteRef {
  folder: string;
  file: string;
}

export interface DiaryMutationResult {
  ok: string[]; // 成功项 "folder/file"
  errors: { note: string; error: string }[];
}

// ── 单例状态（页面级共享，避免在子视图间 prop 透传）──
const folders = ref<string[]>([]);
const foldersLoaded = ref(false);
const notesByFolder = ref<Record<string, DiaryNote[]>>({});
const timeline = ref<DiaryEntry[]>([]);

const loadingFolders = ref(false);
const loadingTimeline = ref(false);
const loadingNotes = ref<Record<string, boolean>>({});
const lastError = ref<string | null>(null);
let foldersRequestSeq = 0;
let timelineRequestSeq = 0;
const notesRequestSeq: Record<string, number> = {};

function note(err: unknown): string {
  return typeof err === "string" ? err : (err as any)?.message || String(err);
}

export function useDiary() {
  const loadFolders = async (force = false): Promise<string[]> => {
    if (foldersLoaded.value && !force) return folders.value;
    const seq = ++foldersRequestSeq;
    loadingFolders.value = true;
    lastError.value = null;
    try {
      const nextFolders = await invoke<string[]>("diary_list_folders");
      if (seq === foldersRequestSeq) {
        folders.value = nextFolders;
        foldersLoaded.value = true;
      }
    } catch (e) {
      lastError.value = note(e);
      throw e;
    } finally {
      if (seq === foldersRequestSeq) {
        loadingFolders.value = false;
      }
    }
    return folders.value;
  };

  const loadNotes = async (folder: string, force = false): Promise<DiaryNote[]> => {
    if (notesByFolder.value[folder] && !force) return notesByFolder.value[folder];
    const seq = (notesRequestSeq[folder] || 0) + 1;
    notesRequestSeq[folder] = seq;
    loadingNotes.value = { ...loadingNotes.value, [folder]: true };
    lastError.value = null;
    try {
      const list = await invoke<DiaryNote[]>("diary_list_notes", { folder });
      if (seq === notesRequestSeq[folder]) {
        notesByFolder.value = { ...notesByFolder.value, [folder]: list };
      }
      return list;
    } catch (e) {
      lastError.value = note(e);
      throw e;
    } finally {
      if (seq === notesRequestSeq[folder]) {
        loadingNotes.value = { ...loadingNotes.value, [folder]: false };
      }
    }
  };

  // 时间线：拉全部本子 → 并发拉各本子条目 → 合并按 lastModified 倒序
  const loadTimeline = async (force = false): Promise<DiaryEntry[]> => {
    if (timeline.value.length > 0 && !force) return timeline.value;
    const seq = ++timelineRequestSeq;
    loadingTimeline.value = true;
    lastError.value = null;
    try {
      const fs = await loadFolders(force);
      const lists = await Promise.all(
        fs.map(async (folder) => {
          try {
            const notes = await loadNotes(folder, force);
            return notes.map((n) => ({ ...n, folder }));
          } catch {
            return [] as DiaryEntry[];
          }
        }),
      );
      const merged = lists.flat();
      merged.sort((a, b) => {
        const ta = Date.parse(a.lastModified) || 0;
        const tb = Date.parse(b.lastModified) || 0;
        return tb - ta;
      });
      if (seq === timelineRequestSeq) {
        timeline.value = merged;
      }
      return merged;
    } catch (e) {
      lastError.value = note(e);
      throw e;
    } finally {
      if (seq === timelineRequestSeq) {
        loadingTimeline.value = false;
      }
    }
  };

  const readNote = (folder: string, file: string): Promise<string> =>
    invoke<string>("diary_read_note", { folder, file });

  const saveNote = async (folder: string, file: string, content: string): Promise<void> => {
    const wasKnownFolder = folders.value.includes(folder);
    await invoke("diary_save_note", { folder, file, content });
    if (!wasKnownFolder) foldersLoaded.value = false;
    invalidate(folder);
  };

  // 重命名 = 存新名 + 删旧名（服务端无 rename 端点）
  const renameNote = async (
    folder: string,
    oldFile: string,
    newFile: string,
    content: string,
  ): Promise<void> => {
    if (newFile === oldFile) {
      await saveNote(folder, newFile, content);
      return;
    }
    await invoke("diary_save_note", { folder, file: newFile, content });
    await invoke("diary_delete_notes", { notes: [{ folder, file: oldFile }] });
    invalidate(folder);
  };

  const deleteNotes = async (notes: DiaryNoteRef[]): Promise<DiaryMutationResult> => {
    const res = await invoke<any>("diary_delete_notes", { notes });
    notes.forEach((n) => invalidate(n.folder));
    foldersLoaded.value = false;
    return { ok: res?.deleted ?? [], errors: res?.errors ?? [] };
  };

  const moveNotes = async (
    sourceNotes: DiaryNoteRef[],
    targetFolder: string,
  ): Promise<DiaryMutationResult> => {
    const res = await invoke<any>("diary_move_notes", { sourceNotes, targetFolder });
    sourceNotes.forEach((n) => invalidate(n.folder));
    invalidate(targetFolder);
    foldersLoaded.value = false; // 目标本子可能是新建的
    return { ok: res?.moved ?? [], errors: res?.errors ?? [] };
  };

  const deleteFolder = async (folder: string): Promise<void> => {
    await invoke("diary_delete_folder", { folder });
    delete notesByFolder.value[folder];
    notesByFolder.value = { ...notesByFolder.value };
    foldersLoaded.value = false;
    timeline.value = [];
  };

  const search = (term: string, folder?: string, limit?: number): Promise<any> =>
    invoke<any>("diary_search", { term, folder: folder || null, limit: limit ?? null });

  // 服务端全文搜索 → 归一化为 DiaryEntry[]。
  // 服务端返回 { notes: [{ name, folderName, lastModified, preview }], total, limited }。
  const searchEntries = async (term: string, folder?: string): Promise<DiaryEntry[]> => {
    const res = await invoke<any>("diary_search", {
      term,
      folder: folder || null,
      limit: 200,
    });
    const notes: any[] = Array.isArray(res?.notes) ? res.notes : [];
    return notes.map((n) => ({
      folder: n.folderName ?? n.folder ?? "",
      name: n.name ?? n.file_name ?? "",
      lastModified: n.lastModified ?? n.last_modified ?? "",
      preview: n.preview ?? "",
    }));
  };

  const associativeDiscovery = (sourceFilePath: string, k = 10): Promise<any> =>
    invoke<any>("diary_associative_discovery", { sourceFilePath, k });

  // 失效某本子缓存 + 时间线（下次重新聚合）
  function invalidate(folder: string) {
    notesRequestSeq[folder] = (notesRequestSeq[folder] || 0) + 1;
    timelineRequestSeq++;
    delete notesByFolder.value[folder];
    notesByFolder.value = { ...notesByFolder.value };
    timeline.value = [];
  }

  const invalidateAll = () => {
    foldersRequestSeq++;
    timelineRequestSeq++;
    Object.keys(notesRequestSeq).forEach((folder) => {
      notesRequestSeq[folder] = (notesRequestSeq[folder] || 0) + 1;
    });
    foldersLoaded.value = false;
    notesByFolder.value = {};
    timeline.value = [];
  };

  return {
    // state
    folders,
    foldersLoaded,
    notesByFolder,
    timeline,
    loadingFolders,
    loadingTimeline,
    loadingNotes,
    lastError,
    // actions
    loadFolders,
    loadNotes,
    loadTimeline,
    readNote,
    saveNote,
    renameNote,
    deleteNotes,
    moveNotes,
    deleteFolder,
    search,
    searchEntries,
    associativeDiscovery,
    invalidate,
    invalidateAll,
  };
}
