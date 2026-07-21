const VCP_BUTTON_PREFIX = "[[点击按钮:";
const VCP_BUTTON_SUFFIX = "]]";
const MAX_VCP_BUTTON_PAYLOAD = 500;

function compactText(value: string | null | undefined): string {
  return (value || "").replace(/\s+/g, " ").trim();
}

function readInlineHandler(button: HTMLButtonElement): string {
  return compactText(button.getAttribute("onclick")).toLowerCase();
}

export function isLocalHtmlButton(button: HTMLButtonElement): boolean {
  if (button.hasAttribute("data-vcp-send") || button.hasAttribute("data-send")) {
    return false;
  }
  if (
    button.closest(
      "[data-vcp-local], [data-vcp-ui-control], [data-vcp-copy-code]",
    )
  ) {
    return true;
  }
  if (button.closest("form") && ["submit", "reset"].includes(button.type)) {
    return true;
  }

  const handler = readInlineHandler(button);
  if (
    /navigator\.clipboard|\.play\s*\(|\.pause\s*\(|requestfullscreen\s*\(|showmodal\s*\(/i.test(
      handler,
    )
  ) {
    return true;
  }

  const label = compactText(button.textContent);
  return /^(?:复制|点击复制|收听|点击收听|播放|暂停|刷新|关闭|展开|收起|预览|源码)$/i.test(
    label,
  );
}

function closestActionScope(button: HTMLButtonElement): Element | null {
  return button.closest(
    "[data-vcp-action-context], article, section, li, [role='group'], [class*='card'], [class*='panel'], [class*='item'], [class*='row']",
  );
}

export function buildHtmlButtonAction(button: HTMLButtonElement): string {
  const explicit = compactText(
    button.getAttribute("data-vcp-send") || button.getAttribute("data-send"),
  );
  if (explicit) return explicit;

  const label = compactText(
    button.getAttribute("aria-label") || button.textContent || button.title,
  );
  if (!label) return "";

  const scope = closestActionScope(button);
  const explicitContext = compactText(
    scope?.getAttribute("data-vcp-action-context"),
  );
  const heading = compactText(
    scope
      ?.querySelector(
        "[data-vcp-title], h1, h2, h3, h4, h5, h6, .title, [class*='title']",
      )
      ?.textContent,
  );
  const description = compactText(
    scope
      ?.querySelector(
        "[data-vcp-description], p, .description, [class*='description']",
      )
      ?.textContent,
  );
  const context = explicitContext || [heading, description].filter(Boolean).join("：");
  return context && !context.includes(label) ? `${label}（${context}）` : label;
}

export function wrapVcpButtonAction(action: string): string {
  const safeAction = compactText(action).replace(/\]\]/g, "] ]");
  if (!safeAction) return "";
  const maxActionLength =
    MAX_VCP_BUTTON_PAYLOAD - VCP_BUTTON_PREFIX.length - VCP_BUTTON_SUFFIX.length;
  return `${VCP_BUTTON_PREFIX}${safeAction.slice(0, maxActionLength)}${VCP_BUTTON_SUFFIX}`;
}
