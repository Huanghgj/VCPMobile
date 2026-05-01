export type ToolMediaType = 'image' | 'video' | 'audio';

export interface ToolResultDetail {
  key: string;
  value: string;
}

export interface ParsedToolResult {
  toolName: string;
  status: string;
  details: ToolResultDetail[];
  footer: string;
}

export interface ToolMediaInfo {
  type: ToolMediaType;
  src: string;
  originalSrc: string;
  title?: string;
  mimeType?: string;
}

export interface ToolTextSegment {
  type: 'text';
  content: string;
}

export interface ToolMediaSegment {
  type: 'media';
  media: ToolMediaInfo;
}

export type ToolResultSegment = ToolTextSegment | ToolMediaSegment;

export const TOOL_REQUEST_START = '<<<[TOOL_REQUEST]>>>';
export const TOOL_REQUEST_END = '<<<[END_TOOL_REQUEST]>>>';
export const TOOL_RESULT_START = '[[VCP调用结果信息汇总:';
export const TOOL_RESULT_END = 'VCP调用结果结束]]';

const mediaExtensionMap: Record<string, ToolMediaType> = {
  png: 'image',
  jpg: 'image',
  jpeg: 'image',
  gif: 'image',
  webp: 'image',
  avif: 'image',
  svg: 'image',
  mp4: 'video',
  webm: 'video',
  mov: 'video',
  m4v: 'video',
  mkv: 'video',
  mp3: 'audio',
  wav: 'audio',
  ogg: 'audio',
  flac: 'audio',
  m4a: 'audio',
};

const markdownMediaPattern = /(!?)\[([^\]]*)]\(([^)\s]+)(?:\s+"([^"]*)")?\)/g;

export const safeJson = (value: unknown) => {
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
};

export const toDisplayText = (value: unknown, fallback = '') => {
  if (value === undefined || value === null) return fallback;
  if (typeof value === 'string') return value;
  return safeJson(value);
};

export const compactText = (value: unknown, limit = 420) => {
  const normalized = toDisplayText(value).replace(/\s+/g, ' ').trim();
  return normalized.length > limit ? `${normalized.slice(0, limit)}...` : normalized;
};

export const extractToolNameFromRequest = (content: string) => {
  const nameMatch = content.match(
    /<tool_name>([\s\S]*?)<\/tool_name>|tool_name:\s*「始(?:exp)?」([^「」]*)「末(?:exp)?」/,
  );
  const rawName = (nameMatch?.[1] || nameMatch?.[2] || '').trim();
  if (!rawName) return 'Processing...';

  return rawName
    .replace(/「始」|「末」|「始exp」|「末exp」/g, '')
    .replace(/,$/, '')
    .trim() || 'Processing...';
};

export const stripToolResultWrapper = (rawBlock: string) => {
  const startIndex = rawBlock.indexOf(TOOL_RESULT_START);
  const contentStart = startIndex === -1 ? 0 : startIndex + TOOL_RESULT_START.length;
  const endIndex = rawBlock.indexOf(TOOL_RESULT_END, contentStart);
  return rawBlock
    .slice(contentStart, endIndex === -1 ? undefined : endIndex)
    .trim();
};

export const parseToolResultBody = (body: string): ParsedToolResult => {
  let toolName = 'Unknown Tool';
  let status = 'Unknown Status';
  const details: ToolResultDetail[] = [];
  const footerLines: string[] = [];

  let currentKey: string | null = null;
  let currentValueLines: string[] = [];

  const flushCurrent = () => {
    if (!currentKey) return;
    const value = currentValueLines.join('\n').trim();
    if (currentKey === '工具名称') {
      toolName = value || toolName;
    } else if (currentKey === '执行状态') {
      status = value || status;
    } else {
      details.push({ key: currentKey, value });
    }
    currentKey = null;
    currentValueLines = [];
  };

  for (const line of body.split(/\r?\n/)) {
    const keyMatch = line.trim().match(/^-\s*([^:：]+)\s*[:：]\s*(.*)$/);
    if (keyMatch) {
      flushCurrent();
      currentKey = keyMatch[1].trim();
      currentValueLines = [keyMatch[2].trim()];
      continue;
    }

    if (currentKey) {
      currentValueLines.push(line);
    } else if (line.trim()) {
      footerLines.push(line);
    }
  }

  flushCurrent();

  return {
    toolName,
    status,
    details,
    footer: footerLines.join('\n').trim(),
  };
};

export const parseToolResultBlock = (rawBlock: string): ParsedToolResult => {
  return parseToolResultBody(stripToolResultWrapper(rawBlock));
};

export const findCompleteToolResultBlocks = (text: string) => {
  const blocks: Array<{ raw: string; body: string; start: number; end: number }> = [];
  let cursor = 0;

  while (cursor < text.length) {
    const start = text.indexOf(TOOL_RESULT_START, cursor);
    if (start === -1) break;

    const bodyStart = start + TOOL_RESULT_START.length;
    const endTokenStart = text.indexOf(TOOL_RESULT_END, bodyStart);
    if (endTokenStart === -1) break;

    const end = endTokenStart + TOOL_RESULT_END.length;
    blocks.push({
      raw: text.slice(start, end),
      body: text.slice(bodyStart, endTokenStart),
      start,
      end,
    });
    cursor = end;
  }

  return blocks;
};

const getMediaExtension = (src: string) => {
  try {
    const parsed = new URL(src, window.location.href);
    const fileName = parsed.pathname.split('/').pop() || '';
    return fileName.split('.').pop()?.toLowerCase() || '';
  } catch {
    const clean = src.split(/[?#]/)[0] || '';
    return clean.split('.').pop()?.toLowerCase() || '';
  }
};

export const inferToolMediaType = (src: string): ToolMediaType | null => {
  if (/^data:image\//i.test(src)) return 'image';
  if (/^data:video\//i.test(src)) return 'video';
  if (/^data:audio\//i.test(src)) return 'audio';
  return mediaExtensionMap[getMediaExtension(src)] || null;
};

const getDataMimeType = (src: string) => {
  const match = src.match(/^data:([^;,]+)[;,]/i);
  return match?.[1];
};

const shouldPreferDirectMedia = (key: string) => {
  return [
    '可访问URL',
    '返回内容',
    '返回结果',
    '内容',
    'Result',
    'output',
    'url',
    'image',
    'video',
    'audio',
  ].some((candidate) => candidate.toLowerCase() === key.toLowerCase());
};

const buildMediaInfo = (src: string, title?: string): ToolMediaInfo | null => {
  const type = inferToolMediaType(src);
  if (!type) return null;

  return {
    type,
    src,
    originalSrc: src,
    title,
    mimeType: getDataMimeType(src),
  };
};

export const detectToolMedia = (key: string, value: string): ToolMediaInfo | null => {
  const trimmed = value.trim();
  if (!trimmed || !shouldPreferDirectMedia(key)) return null;

  const directSrc = /^(https?:\/\/\S+|data:(?:image|video|audio)\/[^,\s]+,[\s\S]+)$/i.test(trimmed)
    ? trimmed
    : '';
  const src = directSrc || extractFirstMarkdownMediaSrc(trimmed);
  if (!src) return null;

  return buildMediaInfo(src, key);
};

export const extractFirstMarkdownMediaSrc = (value: string) => {
  markdownMediaPattern.lastIndex = 0;
  const match = markdownMediaPattern.exec(value);
  return match?.[3] || '';
};

export const splitToolResultSegments = (key: string, value: string): ToolResultSegment[] => {
  const trimmed = value.trim();
  if (!trimmed) return [];

  const directMedia = detectToolMedia(key, trimmed);
  if (directMedia && directMedia.src === trimmed) {
    return [{ type: 'media', media: directMedia }];
  }

  const segments: ToolResultSegment[] = [];
  let cursor = 0;
  markdownMediaPattern.lastIndex = 0;

  let match: RegExpExecArray | null;
  while ((match = markdownMediaPattern.exec(value)) !== null) {
    const raw = match[0];
    const label = match[2] || key;
    const src = match[3] || '';
    const media = buildMediaInfo(src, label || key);
    if (!media) continue;

    if (match.index > cursor) {
      segments.push({
        type: 'text',
        content: value.slice(cursor, match.index),
      });
    }
    segments.push({ type: 'media', media });
    cursor = match.index + raw.length;
  }

  if (cursor < value.length) {
    segments.push({
      type: 'text',
      content: value.slice(cursor),
    });
  }

  return segments.length > 0 ? segments : [{ type: 'text', content: value }];
};

export const isLikelyMarkdownToolField = (key: string) => {
  const normalizedKey = key.toLowerCase();
  return [
    '返回内容',
    '返回结果',
    '内容',
    'result',
    'output',
    'message',
    'markdown',
  ].some((candidate) => candidate.toLowerCase() === normalizedKey);
};
