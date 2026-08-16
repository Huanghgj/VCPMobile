const VCP_BUTTON_PREFIX = "[[点击按钮:";
const VCP_BUTTON_SUFFIX = "]]";
const MAX_VCP_BUTTON_PAYLOAD = 500;

// Crossing from generated HTML into chat is opt-in. Ordinary HTML controls
// must keep their native/local behavior inside the rendered document.
export const HTML_ACTION_SELECTOR = "[data-vcp-send]";

function compactText(value: string | null | undefined): string {
  return (value || "").replace(/\s+/g, " ").trim();
}

export function findHtmlActionElement(target: Element): HTMLElement | null {
  const candidate = target.closest(HTML_ACTION_SELECTOR);
  return candidate instanceof HTMLElement ? candidate : null;
}

export function isHtmlAiActionElement(element: HTMLElement): boolean {
  return element.matches(HTML_ACTION_SELECTOR);
}

function closestActionScope(element: HTMLElement): Element | null {
  return element.closest(
    "[data-vcp-action-context], article, section, li, [role='group'], [class*='card'], [class*='panel'], [class*='item'], [class*='row']",
  );
}

export function buildHtmlButtonAction(element: HTMLElement): string {
  if (!isHtmlAiActionElement(element)) return "";

  const explicit = compactText(element.getAttribute("data-vcp-send"));
  if (explicit) return explicit;

  const label = compactText(
    element.getAttribute("aria-label") || element.textContent || element.title,
  );
  if (!label) return "";

  const scope = closestActionScope(element);
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
