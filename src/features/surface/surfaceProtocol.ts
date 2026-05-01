import type {
  SurfaceCommand,
  SurfaceWidgetBounds,
} from "../../core/api/surfaceBridge";

export const DESKTOP_PUSH_START = "<<<[DESKTOP_PUSH]>>>";
export const DESKTOP_PUSH_END = "<<<[DESKTOP_PUSH_END]>>>";

export interface DesktopPushBlock {
  html: string;
  startIndex: number;
  endIndex: number;
}

const bareClosingTagPattern =
  /(^|[^<])\/((?:h[1-6])|div|span|p|button|section|article|header|footer|main|nav|ul|ol|li|label|form|select|option|textarea|canvas|svg|g|path|style|script|strong|em|small)>/gi;

const rawTextBlockPattern = /<(script|style|textarea)\b[^>]*>[\s\S]*?<\/\1>/gi;

function repairMarkupSegment(segment: string) {
  return segment.replace(
    bareClosingTagPattern,
    (_match, prefix: string, tag: string) => `${prefix}</${tag}>`,
  );
}

function repairCommonScriptTypos(code: string) {
  return code
    .replace(
      /(^|[=(:,\[{\s])'([^'\n]*?),'([^'\n]*?)'/g,
      (_match, prefix: string, left: string, right: string) =>
        `${prefix}'${left}','${right}'`,
    )
    .replace(
      /(^|[=(:,\[{\s])"([^"\n]*?),"([^"\n]*?)"/g,
      (_match, prefix: string, left: string, right: string) =>
        `${prefix}"${left}","${right}"`,
    );
}

function repairScriptBlock(block: string) {
  return block.replace(
    /(<script\b[^>]*>)([\s\S]*?)(<\/script>)/i,
    (_match, open: string, code: string, close: string) =>
      `${open}${repairCommonScriptTypos(code)}${close}`,
  );
}

export function repairSurfaceHtml(content: string) {
  let output = "";
  let cursor = 0;
  rawTextBlockPattern.lastIndex = 0;

  for (
    let match = rawTextBlockPattern.exec(content);
    match;
    match = rawTextBlockPattern.exec(content)
  ) {
    output += repairMarkupSegment(content.slice(cursor, match.index));
    output += match[1].toLowerCase() === "script"
      ? repairScriptBlock(match[0])
      : match[0];
    cursor = match.index + match[0].length;
  }

  output += repairMarkupSegment(content.slice(cursor));
  return output;
}

export function extractDesktopPushBlocks(content: string): DesktopPushBlock[] {
  const blocks: DesktopPushBlock[] = [];
  let cursor = 0;

  while (cursor < content.length) {
    const start = content.indexOf(DESKTOP_PUSH_START, cursor);
    if (start === -1) break;

    const bodyStart = start + DESKTOP_PUSH_START.length;
    const end = content.indexOf(DESKTOP_PUSH_END, bodyStart);
    if (end === -1) break;

    blocks.push({
      html: content.slice(bodyStart, end).trim(),
      startIndex: start,
      endIndex: end + DESKTOP_PUSH_END.length,
    });
    cursor = end + DESKTOP_PUSH_END.length;
  }

  return blocks;
}

export function hasDesktopPushBlocks(content?: string | null): boolean {
  return Boolean(content?.includes(DESKTOP_PUSH_START));
}

export function stripDesktopPushBlocks(content: string): string {
  if (!content.includes(DESKTOP_PUSH_START)) return content;

  let output = "";
  let cursor = 0;

  while (cursor < content.length) {
    const start = content.indexOf(DESKTOP_PUSH_START, cursor);
    if (start === -1) {
      output += content.slice(cursor);
      break;
    }

    output += content.slice(cursor, start);
    const bodyStart = start + DESKTOP_PUSH_START.length;
    const end = content.indexOf(DESKTOP_PUSH_END, bodyStart);

    if (end === -1) {
      break;
    }

    cursor = end + DESKTOP_PUSH_END.length;
  }

  return output.replace(/\n{3,}/g, "\n\n").trim();
}

function createReplacementCommand(html: string): SurfaceCommand | null {
  const match = html.match(
    /^target:\s*「始」([\s\S]*?)「末」\s*,?\s*replace:\s*「始」([\s\S]*?)「末」\s*$/i,
  );
  if (!match) return null;

  return {
    action: "replace",
    targetSelector: match[1].trim(),
    content: repairSurfaceHtml(match[2].trim()),
  };
}

export function createDesktopPushCommands(
  widgetId: string,
  html: string,
  options?: Partial<SurfaceWidgetBounds>,
): SurfaceCommand[] {
  const replacement = createReplacementCommand(html);
  if (replacement) return [replacement];
  const content = repairSurfaceHtml(html);

  const bounds: SurfaceWidgetBounds = {
    x: options?.x ?? 16,
    y: options?.y ?? 96,
    width: options?.width ?? 320,
    height: options?.height ?? 220,
    zIndex: options?.zIndex ?? 1,
  };

  return [
    { action: "create", widgetId, options: bounds },
    { action: "append", widgetId, content },
    { action: "finalize", widgetId },
  ];
}
