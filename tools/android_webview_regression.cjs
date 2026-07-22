const { spawn } = require("node:child_process");

const DEFAULT_PORT = 9222;
const DEFAULT_SELECTOR = ".overflow-y-auto";

const args = Object.fromEntries(
  process.argv.slice(2).map((arg) => {
    const [key, ...rest] = arg.replace(/^--/, "").split("=");
    return [key, rest.length ? rest.join("=") : true];
  }),
);

const port = Number(args.port || DEFAULT_PORT);
const mode = String(args.mode || "inspect");
const selector = String(args.selector || DEFAULT_SELECTOR);
const inputMode = String(args.input || "cdp");
const device = String(args.device || "emulator-5554");

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

class CdpClient {
  constructor(url) {
    this.nextId = 1;
    this.pending = new Map();
    this.socket = new WebSocket(url);
  }

  async connect() {
    await new Promise((resolve, reject) => {
      this.socket.addEventListener("open", resolve, { once: true });
      this.socket.addEventListener("error", reject, { once: true });
    });
    this.socket.addEventListener("message", (event) => {
      const message = JSON.parse(event.data);
      if (!message.id || !this.pending.has(message.id)) return;
      const { resolve, reject, timer } = this.pending.get(message.id);
      clearTimeout(timer);
      this.pending.delete(message.id);
      if (message.error) reject(new Error(message.error.message));
      else resolve(message.result);
    });
  }

  send(method, params = {}) {
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`CDP command timed out: ${method}`));
      }, 10_000);
      this.pending.set(id, { resolve, reject, timer });
      this.socket.send(JSON.stringify({ id, method, params }));
    });
  }

  async evaluate(expression) {
    const result = await this.send("Runtime.evaluate", {
      expression,
      awaitPromise: true,
      returnByValue: true,
    });
    if (result.exceptionDetails) {
      throw new Error(result.exceptionDetails.text || "Runtime.evaluate failed");
    }
    return result.result.value;
  }

  close() {
    this.socket.close();
  }
}

async function findPage() {
  const response = await fetch(`http://127.0.0.1:${port}/json`);
  if (!response.ok) throw new Error(`Unable to read CDP targets on port ${port}`);
  const pages = await response.json();
  const page = pages.find((entry) => entry.type === "page" && entry.webSocketDebuggerUrl);
  if (!page) throw new Error("No debuggable Android WebView page was found");
  return page;
}

function selectorExpression(body) {
  return `(() => { const element = document.querySelector(${JSON.stringify(selector)}); if (!element) throw new Error(${JSON.stringify(`Missing scroll container: ${selector}`)}); ${body} })()`;
}

async function inspect(client, page) {
  return client.evaluate(
    selectorExpression(`
      const vueApp = document.querySelector('#app')?.__vue_app__;
      const provideSymbols = vueApp ? Object.getOwnPropertySymbols(vueApp._context.provides) : [];
      const piniaCandidate = vueApp
        ? [...provideSymbols.map((symbol) => vueApp._context.provides[symbol]), ...Object.values(vueApp._context.provides)]
            .find((value) => value?._s instanceof Map)
        : null;
      const rect = element.getBoundingClientRect();
      const centerX = rect.left + rect.width / 2;
      const centerY = rect.top + rect.height / 2;
      const hit = document.elementFromPoint(centerX, centerY);
      const style = getComputedStyle(element);
      return {
        url: location.href,
        title: document.title,
        devicePixelRatio,
        viewport: { width: innerWidth, height: innerHeight },
        rect: { left: rect.left, top: rect.top, right: rect.right, bottom: rect.bottom },
        hit: hit ? { tag: hit.tagName, className: String(hit.className || '') } : null,
        scrollingStyle: {
          overflowY: style.overflowY,
          overscrollBehaviorY: style.overscrollBehaviorY,
          touchAction: style.touchAction
        },
        scrollTop: element.scrollTop,
        scrollHeight: element.scrollHeight,
        clientHeight: element.clientHeight,
        details: Array.from(document.querySelectorAll('details')).map((node, index) => ({
          index,
          id: node.id || null,
          open: node.open,
          summary: node.querySelector('summary')?.textContent?.trim().slice(0, 120) || ''
        })),
        iframeCount: document.querySelectorAll('iframe').length,
        messageCount: document.querySelectorAll('[data-message-id]').length,
        messages: Array.from(document.querySelectorAll('[data-message-id]')).map((node) => ({
          id: node.getAttribute('data-message-id'),
          text: (node.textContent || '').trim().slice(0, 100),
          details: node.querySelectorAll('details').length,
          iframe: node.querySelectorAll('iframe').length,
          parked: node.querySelectorAll('.vcp-render-parked').length
        })),
        vueDebug: {
          hasApp: Boolean(vueApp),
          provideSymbols: provideSymbols.map(String),
          provideKeys: vueApp ? Object.keys(vueApp._context.provides) : [],
          storeIds: piniaCandidate ? Array.from(piniaCandidate._s.keys()) : []
        },
        pageUrl: ${JSON.stringify(page.url)}
      };
    `),
  );
}

async function dispatchTouch(client, type, x, y) {
  await client.send("Input.dispatchTouchEvent", {
    type,
    touchPoints: type === "touchEnd" ? [] : [{ x, y, id: 1, radiusX: 4, radiusY: 4 }],
  });
}

async function sampleScrollTop(client) {
  return client.evaluate(selectorExpression("return element.scrollTop;"));
}

async function injectRegressionProbe(client) {
  return client.evaluate(`(async () => {
    const app = document.querySelector('#app')?.__vue_app__;
    if (!app) throw new Error('Vue app instance is unavailable');
    const pinia = [
      ...Object.getOwnPropertySymbols(app._context.provides)
        .map((symbol) => app._context.provides[symbol]),
      ...Object.values(app._context.provides)
    ].find((value) => value?._s instanceof Map);
    const store = pinia?._s?.get('chatHistory');
    if (!store) throw new Error('chatHistory Pinia store is unavailable');

    const now = Date.now();
    const id = 'codex_android_render_probe_' + now;
    const previous = store.currentChatHistory.at(-1) || {};
    const directDetails = [
      '<details id="android-default-fold" open>',
      '<summary><b>Android default fold probe</b></summary>',
      '<p>' + 'Long folded solution content. '.repeat(120) + '</p>',
      '</details>'
    ].join('');
    const activePreview = [
      '<div id="android-iframe-probe" style="padding:12px;background:#172033;color:#f8fafc">',
      '<style>@keyframes probePulse{0%,100%{opacity:.72}50%{opacity:1}} .probe-pulse{animation:probePulse 1s infinite}</style>',
      '<div class="probe-pulse" data-vcp-animate style="padding:12px;border:1px solid #5eead4">Visible animation probe</div>',
      '<details id="android-iframe-fold" open><summary>Iframe fold probe</summary><p>Iframe details body</p></details>',
      '<div style="height:720px;padding-top:24px">Iframe native scroll-chain area</div>',
      '<script>setTimeout(() => { const image = new Image(); image.id = "android-delayed-image"; image.alt = "Delayed image probe"; image.style.cssText = "display:block;width:100%;height:auto"; image.src = "data:image/svg+xml,%3Csvg xmlns=%27http://www.w3.org/2000/svg%27 width=%27360%27 height=%27640%27%3E%3Crect width=%27360%27 height=%27640%27 fill=%27%232563eb%27/%3E%3C/svg%3E"; document.body.appendChild(image); }, 1800);<\\/script>',
      '</div>'
    ].join('');
    store.currentChatHistory.push({
      ...previous,
      id,
      role: 'assistant',
      name: 'Android Renderer Regression Probe',
      content: directDetails,
      timestamp: now,
      blocks: [
        { type: 'markdown', content: directDetails, hash: id + '-details' },
        { type: 'html-preview', content: activePreview, hash: id + '-iframe' }
      ],
      tailBlock: null,
      renderRevision: now
    });
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    return { id, messageCount: store.currentChatHistory.length };
  })()`);
}

async function runLatestReplyWithoutTouch(client) {
  return client.evaluate(`(async () => {
    const app = document.querySelector('#app')?.__vue_app__;
    const pinia = app && [
      ...Object.getOwnPropertySymbols(app._context.provides)
        .map((symbol) => app._context.provides[symbol]),
      ...Object.values(app._context.provides)
    ].find((value) => value?._s instanceof Map);
    const store = pinia?._s?.get('chatHistory');
    if (!store) throw new Error('chatHistory Pinia store is unavailable');
    const list = document.querySelector(${JSON.stringify(selector)});
    if (!list) throw new Error('Chat scroll container is unavailable');

    // This deliberately uses only programmatic state changes. No touch, wheel,
    // scrollIntoView or pointer event is dispatched after the reply is added.
    list.scrollTo({ top: list.scrollHeight, behavior: 'auto' });
    list.dispatchEvent(new Event('scroll'));
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    const startingBottomGap = list.scrollHeight - list.scrollTop - list.clientHeight;
    const now = Date.now();
    const id = 'codex_android_latest_reply_' + now;
    const elementId = 'android-no-touch-reply-' + now;
    const previous = store.currentChatHistory.at(-1) || {};
    const html = [
      '<div id="' + elementId + '" style="padding:20px;background:#172033;color:#f8fafc">',
      '<h2>Latest reply appeared without touching the screen</h2>',
      '<details open><summary>Default folded details probe</summary><p>Folded content</p></details>',
      '<div style="height:260px">Iframe first-paint probe</div>',
      '</div>'
    ].join('');
    const message = {
      ...previous,
      id,
      role: 'assistant',
      name: 'Latest Reply No-Touch Probe',
      content: html,
      timestamp: now,
      blocks: [{ type: 'html-preview', content: html, hash: id + '-first' }],
      tailBlock: null,
      tailContent: '',
      renderRevision: now
    };
    store.currentChatHistory.push(message);
    const reactiveMessage = store.currentChatHistory.find((item) => item.id === id);
    if (!reactiveMessage) throw new Error('Inserted latest reply is unavailable');

    const waitFor = async (predicate, timeout = 4000) => {
      const deadline = performance.now() + timeout;
      while (performance.now() < deadline) {
        const value = predicate();
        if (value) return value;
        await new Promise((resolve) => setTimeout(resolve, 40));
      }
      return null;
    };
    const findFrame = () => document.querySelector('[data-message-id="' + id + '"] iframe');
    const firstFrame = await waitFor(() => {
      const frame = findFrame();
      return frame && frame.getBoundingClientRect().height > 0 ? frame : null;
    });
    const initialIframeHeight = firstFrame?.getBoundingClientRect().height || 0;
    await new Promise((resolve) => setTimeout(resolve, 450));
    const firstBottomGap = list.scrollHeight - list.scrollTop - list.clientHeight;

    const html2 = html.replace('without touching the screen', 'after final render revision');
    reactiveMessage.content = html2;
    reactiveMessage.blocks = [{ type: 'html-preview', content: html2, hash: id + '-final' }];
    reactiveMessage.renderRevision = now + 1;
    const completedFrame = await waitFor(() => {
      const frame = findFrame();
      return frame?.srcdoc?.includes('after final render revision') ? frame : null;
    });
    await new Promise((resolve) => setTimeout(resolve, 450));
    const finalBottomGap = list.scrollHeight - list.scrollTop - list.clientHeight;
    const result = {
      id,
      startingBottomGap,
      initialIframeMounted: Boolean(firstFrame),
      initialIframeHeight,
      initialBottomGap: firstBottomGap,
      completedIframeMounted: Boolean(completedFrame),
      completedIframeHeight: completedFrame?.getBoundingClientRect().height || 0,
      completedSrcdocRefreshed: Boolean(completedFrame?.srcdoc?.includes('after final render revision')),
      finalBottomGap
    };
    if (
      !result.initialIframeMounted ||
      result.initialIframeHeight <= 0 ||
      !result.completedIframeMounted ||
      !result.completedSrcdocRefreshed ||
      Math.abs(result.initialBottomGap) > 4 ||
      Math.abs(result.finalBottomGap) > 4
    ) {
      throw new Error('Latest reply no-touch regression failed: ' + JSON.stringify(result));
    }
    return result;
  })()`);
}

function runAdbInput(inputArgs) {
  return new Promise((resolve, reject) => {
    const child = spawn("adb", ["-s", device, "shell", "input", "touchscreen", ...inputArgs], {
      windowsHide: true,
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stderr = "";
    child.stderr.on("data", (chunk) => {
      stderr += chunk;
    });
    child.once("error", reject);
    child.once("exit", (code) => {
      if (code === 0) resolve();
      else reject(new Error(`adb input failed (${code}): ${stderr.trim()}`));
    });
  });
}

async function dispatchTap(client, x, y, dpr = 1) {
  if (inputMode === "adb") {
    await runAdbInput(["tap", String(Math.round(x * dpr)), String(Math.round(y * dpr))]);
    return;
  }
  await dispatchTouch(client, "touchStart", x, y);
  await dispatchTouch(client, "touchEnd", x, y);
}

async function runSwipe(client, direction) {
  await client.evaluate(
    selectorExpression(`
      const maxScrollTop = Math.max(0, element.scrollHeight - element.clientHeight);
      const desired = ${direction === "up" ? "Math.min(maxScrollTop - 240, Math.max(240, maxScrollTop * 0.35))" : "Math.min(maxScrollTop - 240, Math.max(240, maxScrollTop * 0.65))"};
      element.scrollTop = Math.max(0, desired);
      return element.scrollTop;
    `),
  );
  await sleep(600);
  const geometry = await client.evaluate(
    selectorExpression(`
      const rect = element.getBoundingClientRect();
      return {
        x: Math.round(rect.left + rect.width * 0.5),
        top: Math.round(rect.top + Math.max(48, rect.height * 0.22)),
        bottom: Math.round(rect.bottom - Math.max(48, rect.height * 0.22)),
        dpr: devicePixelRatio,
        maxScrollTop: Math.max(0, element.scrollHeight - element.clientHeight)
      };
    `),
  );
  const startY = direction === "up" ? geometry.bottom : geometry.top;
  const endY = direction === "up" ? geometry.top : geometry.bottom;
  return collectSwipe(client, direction, geometry.x, startY, endY, geometry.dpr);
}

async function collectSwipe(client, direction, x, startY, endY, dpr) {
  const samples = [await sampleScrollTop(client)];
  if (inputMode === "adb") {
    let finished = false;
    const swipe = runAdbInput([
      "swipe",
      String(Math.round(x * dpr)),
      String(Math.round(startY * dpr)),
      String(Math.round(x * dpr)),
      String(Math.round(endY * dpr)),
      "700",
    ]).finally(() => {
      finished = true;
    });
    while (!finished) {
      await sleep(16);
      samples.push(await sampleScrollTop(client));
    }
    await swipe;
  } else {
    await dispatchTouch(client, "touchStart", x, startY);
    await sleep(120);
    const steps = 32;
    for (let step = 1; step <= steps; step += 1) {
      const y = Math.round(startY + ((endY - startY) * step) / steps);
      await dispatchTouch(client, "touchMove", x, y);
      await sleep(12);
      samples.push(await sampleScrollTop(client));
    }
    await dispatchTouch(client, "touchEnd", x, endY);
  }
  for (let step = 0; step < 12; step += 1) {
    await sleep(30);
    samples.push(await sampleScrollTop(client));
  }

  const expectedSign = direction === "up" ? 1 : -1;
  let backwardSteps = 0;
  let movingSteps = 0;
  for (let index = 1; index < samples.length; index += 1) {
    const delta = samples[index] - samples[index - 1];
    if (Math.abs(delta) >= 0.75) movingSteps += 1;
    if (delta * expectedSign < -1.5) backwardSteps += 1;
  }
  return {
    direction,
    start: samples[0],
    end: samples.at(-1),
    distance: samples.at(-1) - samples[0],
    backwardSteps,
    movingSteps,
    samples,
  };
}

async function runIframeSwipe(client, direction) {
  const geometry = await client.evaluate(selectorExpression(`
    const frame = document.querySelector('iframe');
    if (!frame) return null;
    frame.scrollIntoView({ block: 'center' });
    return true;
  `));
  if (!geometry) return { available: false };
  await sleep(700);
  const settled = await client.evaluate(selectorExpression(`
    const frame = document.querySelector('iframe');
    if (!frame) return null;
    const frameRect = frame.getBoundingClientRect();
    const listRect = element.getBoundingClientRect();
    const top = Math.max(frameRect.top, listRect.top) + 56;
    const bottom = Math.min(frameRect.bottom, listRect.bottom) - 56;
    if (bottom - top < 120) return { error: 'iframe has insufficient visible height', top, bottom };
    return {
      x: Math.round(frameRect.left + frameRect.width * 0.5),
      top: Math.round(top),
      bottom: Math.round(bottom),
      dpr: devicePixelRatio,
      frameHeight: frameRect.height
    };
  `));
  if (!settled || settled.error) return { available: false, ...settled };
  const startY = direction === "up" ? settled.bottom : settled.top;
  const endY = direction === "up" ? settled.top : settled.bottom;
  return {
    available: true,
    frameHeight: settled.frameHeight,
    ...(await collectSwipe(client, direction, settled.x, startY, endY, settled.dpr)),
  };
}

async function runResizeStability(client) {
  const initial = await client.evaluate(selectorExpression(`
    const frame = document.querySelector('iframe');
    if (!frame) return null;
    frame.scrollIntoView({ block: 'center' });
    return true;
  `));
  if (!initial) return { available: false };
  await sleep(250);
  const samples = [];
  for (let index = 0; index < 32; index += 1) {
    samples.push(await client.evaluate(selectorExpression(`
      const frame = document.querySelector('iframe');
      return {
        scrollTop: element.scrollTop,
        maxScrollTop: Math.max(0, element.scrollHeight - element.clientHeight),
        frameHeight: frame?.getBoundingClientRect().height || 0
      };
    `)));
    await sleep(100);
  }
  return {
    available: true,
    start: samples[0],
    end: samples.at(-1),
    maxScrollTopObserved: Math.max(...samples.map((sample) => sample.scrollTop)),
    distinctFrameHeights: [...new Set(samples.map((sample) => sample.frameHeight))],
    samples,
  };
}

async function runShortResizeStability(client) {
  const positioned = await client.evaluate(selectorExpression(`
    element.scrollTop = element.scrollHeight;
    return element.scrollTop;
  `));
  await sleep(350);
  const geometry = await client.evaluate(selectorExpression(`
    const frame = document.querySelector('iframe');
    if (!frame) return null;
    const frameRect = frame.getBoundingClientRect();
    const listRect = element.getBoundingClientRect();
    const top = Math.max(frameRect.top, listRect.top) + 40;
    const bottom = Math.min(frameRect.bottom, listRect.bottom) - 40;
    if (bottom - top < 100) return null;
    const middle = (top + bottom) / 2;
    return {
      x: Math.round(frameRect.left + frameRect.width * 0.5),
      startY: Math.round(middle - 20),
      endY: Math.round(middle + 20),
      dpr: devicePixelRatio
    };
  `));
  if (!geometry) return { available: false, positioned };
  const gesture = await collectSwipe(
    client,
    "down",
    geometry.x,
    geometry.startY,
    geometry.endY,
    geometry.dpr,
  );
  const beforeResize = await client.evaluate(selectorExpression(`
    const spacer = document.createElement('div');
    spacer.id = 'android-short-resize-spacer';
    spacer.style.height = '320px';
    element.querySelector('.messages-inner-container')?.appendChild(spacer);
    return {
      scrollTop: element.scrollTop,
      maxScrollTop: Math.max(0, element.scrollHeight - element.clientHeight)
    };
  `));
  await sleep(700);
  const afterResize = await client.evaluate(selectorExpression(`
    const result = {
      scrollTop: element.scrollTop,
      maxScrollTop: Math.max(0, element.scrollHeight - element.clientHeight)
    };
    document.getElementById('android-short-resize-spacer')?.remove();
    return result;
  `));
  return {
    available: true,
    positioned,
    gesture,
    beforeResize,
    afterResize,
    scrollJump: afterResize.scrollTop - beforeResize.scrollTop,
  };
}

async function runReplyRefresh(client) {
  return client.evaluate(`(async () => {
    const app = document.querySelector('#app')?.__vue_app__;
    const pinia = app && [
      ...Object.getOwnPropertySymbols(app._context.provides)
        .map((symbol) => app._context.provides[symbol]),
      ...Object.values(app._context.provides)
    ].find((value) => value?._s instanceof Map);
    const store = pinia?._s?.get('chatHistory');
    if (!store) throw new Error('chatHistory Pinia store is unavailable');
    const now = Date.now();
    const id = 'codex_android_refresh_probe_' + now;
    const elementId = 'android-reply-refresh-probe-' + now;
    const previous = store.currentChatHistory.at(-1) || {};
    const message = {
      ...previous,
      id,
      role: 'assistant',
      name: 'Android Reply Refresh Probe',
      content: '<div id="' + elementId + '">frame 1</div>',
      timestamp: now,
      blocks: [{
        type: 'markdown',
        content: '<div id="' + elementId + '">frame 1</div>',
        hash: id + '-frame-1'
      }],
      tailBlock: null,
      renderRevision: now
    };
    store.currentChatHistory.push(message);
    const reactiveMessage = store.currentChatHistory.find((item) => item.id === id);
    if (!reactiveMessage) throw new Error('Inserted refresh probe is unavailable');
    const list = document.querySelector(${JSON.stringify(selector)});
    if (list) list.scrollTop = list.scrollHeight;
    const waitFor = async (predicate, timeout = 2500) => {
      const deadline = performance.now() + timeout;
      while (performance.now() < deadline) {
        const value = predicate();
        if (value) return value;
        await new Promise((resolve) => setTimeout(resolve, 40));
      }
      return null;
    };
    const first = await waitFor(() => document.getElementById(elementId)?.textContent === 'frame 1');
    const scrollBeforeUpdate = list?.scrollTop ?? null;
    reactiveMessage.content = '<div id="' + elementId + '">frame 2 complete</div>';
    reactiveMessage.blocks = [{
      type: 'markdown',
      content: reactiveMessage.content,
      hash: id + '-frame-2'
    }];
    reactiveMessage.renderRevision = now + 1;
    const second = await waitFor(() => document.getElementById(elementId)?.textContent === 'frame 2 complete');
    return {
      id,
      initialRendered: Boolean(first),
      completedRendered: Boolean(second),
      finalText: document.getElementById(elementId)?.textContent || null,
      reactiveState: {
        content: reactiveMessage.content,
        blockContent: reactiveMessage.blocks?.[0]?.content || null,
        renderRevision: reactiveMessage.renderRevision
      },
      domRenderRevision: document.querySelector('[data-message-id="' + id + '"]')?.getAttribute('data-render-revision') || null,
      scrollBeforeUpdate,
      scrollAfterUpdate: list?.scrollTop ?? null
    };
  })()`);
}

function flattenFrames(frameTree, output = []) {
  output.push(frameTree.frame);
  for (const child of frameTree.childFrames || []) flattenFrames(child, output);
  return output;
}

async function captureProbeAnimationState(client) {
  const tree = await client.send("Page.getFrameTree");
  const frames = flattenFrames(tree.frameTree).slice(1).reverse();
  for (const frame of frames) {
    try {
      const world = await client.send("Page.createIsolatedWorld", {
        frameId: frame.id,
        worldName: `vcp-regression-${Date.now()}`,
      });
      const result = await client.send("Runtime.evaluate", {
        contextId: world.executionContextId,
        expression: `(() => ({
          hasProbe: Boolean(document.getElementById('android-iframe-probe')),
          playStates: typeof document.getAnimations === 'function'
            ? document.getAnimations().map((animation) => animation.playState)
            : [],
          delayedImageLoaded: Boolean(document.getElementById('android-delayed-image')),
          hiddenClass: document.documentElement.classList.contains('vcp-preview-hidden')
        }))()`,
        returnByValue: true,
      });
      const value = result.result?.value;
      if (value?.hasProbe) return value;
    } catch {}
  }
  return null;
}

async function runAnimationVisibility(client) {
  const showLatestPreview = () => client.evaluate(selectorExpression(`
    const blocks = document.querySelectorAll('.html-preview-block');
    const block = blocks[blocks.length - 1];
    if (!block) return false;
    block.scrollIntoView({ block: 'start' });
    return true;
  `));
  if (!(await showLatestPreview())) return { available: false };
  await sleep(900);
  const visible = await captureProbeAnimationState(client);

  await client.evaluate(selectorExpression("element.scrollTop = 0; return element.scrollTop;"));
  await sleep(900);
  const offscreenIframeCount = await client.evaluate("document.querySelectorAll('iframe').length");
  const offscreenProbe = offscreenIframeCount
    ? await captureProbeAnimationState(client)
    : null;
  const offscreen = offscreenProbe || { destroyed: true };

  await showLatestPreview();
  await sleep(900);
  const restored = await captureProbeAnimationState(client);
  return {
    available: Boolean(visible),
    visible,
    offscreen,
    restored,
  };
}

async function runFold(client) {
  const locateDetails = () => client.evaluate(`(() => {
    const details = document.querySelector('details');
    if (!details) return null;
    details.scrollIntoView({ block: 'center' });
    const summary = details.querySelector('summary');
    const rect = summary?.getBoundingClientRect();
    return {
      open: details.open,
      summary: summary?.textContent?.trim().slice(0, 120) || '',
      x: rect ? Math.round(rect.left + Math.min(rect.width * 0.5, 160)) : 0,
      y: rect ? Math.round(rect.top + rect.height * 0.5) : 0,
      dpr: devicePixelRatio
    };
  })()`);
  let initial = await locateDetails();
  if (!initial) {
    await client.evaluate(selectorExpression(`
      const probes = document.querySelectorAll('[data-message-id^="codex_android_render_probe_"]');
      const probe = probes[probes.length - 1];
      if (probe) probe.scrollIntoView({ block: 'start' });
      else element.scrollTop = element.scrollHeight;
      return element.scrollTop;
    `));
    await sleep(900);
    initial = await locateDetails();
  }
  if (!initial) return { available: false };
  await sleep(120);
  await dispatchTap(client, initial.x, initial.y, initial.dpr);
  await sleep(160);
  const afterFirstTap = await client.evaluate("document.querySelector('details')?.open ?? null");
  await dispatchTap(client, initial.x, initial.y, initial.dpr);
  await sleep(160);
  const afterSecondTap = await client.evaluate("document.querySelector('details')?.open ?? null");
  return {
    available: true,
    initialOpen: initial.open,
    afterFirstTap,
    afterSecondTap,
    summary: initial.summary,
  };
}

async function main() {
  const page = await findPage();
  const client = new CdpClient(page.webSocketDebuggerUrl);
  await client.connect();
  try {
    let result;
    if (mode === "inspect") result = await inspect(client, page);
    else if (mode === "inject-probe") result = await injectRegressionProbe(client);
    else if (mode === "latest-reply") result = await runLatestReplyWithoutTouch(client);
    else if (mode === "swipe") {
      result = {
        inputMode,
        up: await runSwipe(client, "up"),
        down: await runSwipe(client, "down"),
      };
    } else if (mode === "fold") result = await runFold(client);
    else if (mode === "iframe-swipe") {
      result = {
        inputMode,
        up: await runIframeSwipe(client, "up"),
        down: await runIframeSwipe(client, "down"),
      };
    } else if (mode === "resize") result = await runResizeStability(client);
    else if (mode === "short-resize") result = await runShortResizeStability(client);
    else if (mode === "refresh") result = await runReplyRefresh(client);
    else if (mode === "animation") result = await runAnimationVisibility(client);
    else throw new Error(`Unsupported mode: ${mode}`);
    process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
  } finally {
    client.close();
  }
}

main().catch((error) => {
  console.error(error.stack || error.message || error);
  process.exitCode = 1;
});
