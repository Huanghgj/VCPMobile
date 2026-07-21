import morphdom from "morphdom";

function getNodeKey(node: Node): string | undefined {
  if (node.nodeType !== Node.ELEMENT_NODE) return undefined;
  const element = node as Element;
  return (
    element.id ||
    element.getAttribute("data-vcp-key") ||
    element.getAttribute("data-vcp-render-key") ||
    undefined
  );
}

/** The only DOM mutation path used by Renderer V2 for stable and streaming HTML. */
export function patchRenderDocumentRoot(root: HTMLElement, html: string): void {
  morphdom(root, `<div>${html}</div>`, {
    childrenOnly: true,
    getNodeKey,
    onBeforeElUpdated(fromElement: HTMLElement, toElement: HTMLElement) {
      if (fromElement.isEqualNode(toElement)) return false;

      if (
        fromElement instanceof HTMLDetailsElement &&
        toElement instanceof HTMLDetailsElement
      ) {
        toElement.open = fromElement.open;
      }

      if (fromElement === document.activeElement) {
        requestAnimationFrame(() => toElement.focus?.());
      }

      if (fromElement instanceof HTMLMediaElement && !fromElement.paused) {
        return false;
      }

      if (
        fromElement instanceof HTMLImageElement &&
        toElement instanceof HTMLImageElement &&
        fromElement.currentSrc === toElement.currentSrc &&
        fromElement.complete &&
        fromElement.naturalWidth > 0
      ) {
        return false;
      }

      return true;
    },
  });
}
