// @vitest-environment jsdom

import { describe, expect, it } from "vitest";
import {
  HTML_ACTION_SELECTOR,
  buildHtmlButtonAction,
  findHtmlActionElement,
  isHtmlAiActionElement,
  wrapVcpButtonAction,
} from "./htmlActions";

function buttonFrom(html: string): HTMLButtonElement {
  const root = document.createElement("div");
  root.innerHTML = html;
  return root.querySelector("button")!;
}

describe("HTML action bridge", () => {
  it("only opts explicit VCP send controls into the AI bridge", () => {
    expect(HTML_ACTION_SELECTOR).toBe("[data-vcp-send]");

    for (const html of [
      "<button>立即下单</button>",
      '<button role="button">继续</button>',
      '<button data-send="继续">旧的通用属性</button>',
      '<button onclick="toggleImage()">展示/隐藏图片</button>',
    ]) {
      const button = buttonFrom(html);
      expect(findHtmlActionElement(button)).toBeNull();
      expect(isHtmlAiActionElement(button)).toBe(false);
      expect(buildHtmlButtonAction(button)).toBe("");
    }
  });

  it("builds an opted-in AI action from the nearest card context", () => {
    const button = buttonFrom(`
      <section class="order-card">
        <h3>夜宵配送</h3>
        <p>送到客厅茶几</p>
        <button data-vcp-send>立即下单</button>
      </section>
    `);

    expect(findHtmlActionElement(button)).toBe(button);
    expect(isHtmlAiActionElement(button)).toBe(true);
    expect(buildHtmlButtonAction(button)).toBe(
      "立即下单（夜宵配送：送到客厅茶几）",
    );
  });

  it("leaves cursor-pointer cards and their local handlers inside HTML", () => {
    const root = document.createElement("div");
    root.innerHTML = `
      <div style="padding:15px;cursor:pointer" onclick="toggleImage()">
        <div>🔄</div>
        <div>展示/隐藏图片</div>
      </div>
    `;
    const label = root.querySelector("div div:nth-child(2)")!;
    expect(findHtmlActionElement(label)).toBeNull();
  });

  it("prefers explicit actions and caps the wrapped payload at 500 chars", () => {
    const explicit = buttonFrom(
      '<button data-vcp-send="继续刚才的剧情">任意文字</button>',
    );
    expect(buildHtmlButtonAction(explicit)).toBe("继续刚才的剧情");

    const wrapped = wrapVcpButtonAction("长".repeat(800) + "]]尾部");
    expect(wrapped).toHaveLength(500);
    expect(wrapped.startsWith("[[点击按钮:")).toBe(true);
    expect(wrapped.endsWith("]]" )).toBe(true);
  });
});
