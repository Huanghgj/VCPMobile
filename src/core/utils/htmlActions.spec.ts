// @vitest-environment jsdom

import { describe, expect, it } from "vitest";
import {
  buildHtmlButtonAction,
  isLocalHtmlButton,
  wrapVcpButtonAction,
} from "./htmlActions";

function buttonFrom(html: string): HTMLButtonElement {
  const root = document.createElement("div");
  root.innerHTML = html;
  return root.querySelector("button")!;
}

describe("HTML action bridge", () => {
  it("keeps copy and media controls local", () => {
    expect(isLocalHtmlButton(buttonFrom("<button>点击复制</button>"))).toBe(true);
    expect(
      isLocalHtmlButton(
        buttonFrom('<button onclick="document.querySelector(\'audio\').play()">试听</button>'),
      ),
    ).toBe(true);
    expect(
      isLocalHtmlButton(buttonFrom('<button data-vcp-local>本地切换</button>')),
    ).toBe(true);
  });

  it("builds an AI action from the nearest card title and description", () => {
    const button = buttonFrom(`
      <section class="order-card">
        <h3>夜宵配送</h3>
        <p>送到客厅茶几</p>
        <button>立即下单</button>
      </section>
    `);

    expect(isLocalHtmlButton(button)).toBe(false);
    expect(buildHtmlButtonAction(button)).toBe(
      "立即下单（夜宵配送：送到客厅茶几）",
    );
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
