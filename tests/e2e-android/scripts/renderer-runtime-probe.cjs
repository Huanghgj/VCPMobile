const fs = require("fs");
const path = require("path");
const { ROOT, runAdb, ensureDir } = require("./adb-env.cjs");

const PACKAGE = process.env.E2E_PACKAGE || "com.vcp.avatar.debug";
const ACTIVITY = `${PACKAGE}/com.vcp.avatar.MainActivity`;

function parseArgs(argv) {
  const args = { port: 9222, label: "android" };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--port") args.port = Number(argv[++index]);
    else if (arg === "--label") args.label = argv[++index];
    else throw new Error(`Unknown argument: ${arg}`);
  }
  return args;
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function waitFor(read, description, timeoutMs = 20_000) {
  const deadline = Date.now() + timeoutMs;
  let lastError;
  while (Date.now() < deadline) {
    try {
      const value = await read();
      if (value) return value;
    } catch (error) {
      lastError = error;
    }
    await sleep(200);
  }
  throw new Error(
    `Timed out waiting for ${description}${lastError ? `: ${lastError}` : ""}`,
  );
}

class CdpClient {
  constructor(url) {
    this.url = url;
    this.sequence = 0;
    this.pending = new Map();
    this.listeners = new Map();
  }

  async connect() {
    this.socket = new WebSocket(this.url);
    this.socket.addEventListener("message", (event) => {
      const message = JSON.parse(String(event.data));
      if (message.id) {
        const pending = this.pending.get(message.id);
        if (!pending) return;
        this.pending.delete(message.id);
        clearTimeout(pending.timer);
        if (message.error)
          pending.reject(new Error(JSON.stringify(message.error)));
        else pending.resolve(message.result || {});
        return;
      }
      for (const listener of this.listeners.get(message.method) || []) {
        listener(message.params || {});
      }
    });
    await new Promise((resolve, reject) => {
      this.socket.addEventListener("open", resolve, { once: true });
      this.socket.addEventListener("error", reject, { once: true });
    });
  }

  on(method, listener) {
    const listeners = this.listeners.get(method) || [];
    listeners.push(listener);
    this.listeners.set(method, listeners);
  }

  call(method, params = {}, timeoutMs = 15_000) {
    const id = ++this.sequence;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`CDP timeout: ${method}`));
      }, timeoutMs);
      this.pending.set(id, { resolve, reject, timer });
      this.socket.send(JSON.stringify({ id, method, params }));
    });
  }

  close() {
    this.socket?.close();
  }
}

async function connectToWebView(port) {
  const pid = runAdb(["shell", "pidof", PACKAGE]).trim();
  if (!pid) throw new Error(`${PACKAGE} is not running`);
  runAdb([
    "forward",
    `tcp:${port}`,
    `localabstract:webview_devtools_remote_${pid}`,
  ]);
  const targets = await waitFor(async () => {
    const response = await fetch(`http://127.0.0.1:${port}/json`);
    const value = await response.json();
    return value.find((target) => target.type === "page") ? value : null;
  }, "Android WebView CDP target");
  const target = targets.find((item) => item.type === "page");
  const client = new CdpClient(target.webSocketDebuggerUrl);
  await client.connect();
  return { client, target, pid };
}

async function evaluate(client, expression, contextId) {
  const result = await client.call("Runtime.evaluate", {
    expression,
    contextId,
    awaitPromise: true,
    returnByValue: true,
  });
  if (result.exceptionDetails) {
    throw new Error(
      result.exceptionDetails.exception?.description ||
        result.exceptionDetails.text ||
        "Runtime.evaluate failed",
    );
  }
  return result.result?.value;
}

function flattenFrames(frameTree) {
  return [
    frameTree.frame,
    ...(frameTree.childFrames || []).flatMap(flattenFrames),
  ];
}

async function findFrame(client, selector) {
  return waitFor(async () => {
    const { frameTree } = await client.call("Page.getFrameTree");
    const childFrames = flattenFrames(frameTree).slice(1);
    for (const frame of childFrames) {
      try {
        const { executionContextId } = await client.call(
          "Page.createIsolatedWorld",
          {
            frameId: frame.id,
            worldName: `vcp-probe-${frame.id}`,
            grantUniveralAccess: true,
          },
        );
        if (
          await evaluate(
            client,
            `Boolean(document.querySelector(${JSON.stringify(selector)}))`,
            executionContextId,
          )
        ) {
          return { frameId: frame.id, contextId: executionContextId };
        }
      } catch {}
    }
    return null;
  }, `iframe containing ${selector}`);
}

async function scrollFrameOwnerIntoView(client, frameId) {
  const { backendNodeId } = await client.call("DOM.getFrameOwner", { frameId });
  const { object } = await client.call("DOM.resolveNode", { backendNodeId });
  await client.call("Runtime.callFunctionOn", {
    objectId: object.objectId,
    functionDeclaration:
      'function(){ this.scrollIntoView({block:"center",inline:"center"}); }',
  });
  await sleep(300);
  const { model } = await client.call("DOM.getBoxModel", { backendNodeId });
  const xs = [
    model.content[0],
    model.content[2],
    model.content[4],
    model.content[6],
  ];
  const ys = [
    model.content[1],
    model.content[3],
    model.content[5],
    model.content[7],
  ];
  return { x: Math.min(...xs), y: Math.min(...ys) };
}

async function clickFrameControl(client, frame, selector) {
  await evaluate(
    client,
    `document.querySelector(${JSON.stringify(selector)}).scrollIntoView({block:'center',inline:'center'})`,
    frame.contextId,
  );
  await sleep(150);
  const origin = await scrollFrameOwnerIntoView(client, frame.frameId);
  const rect = await evaluate(
    client,
    `(() => { const rect = document.querySelector(${JSON.stringify(
      selector,
    )}).getBoundingClientRect(); return { x: rect.x, y: rect.y, width: rect.width, height: rect.height }; })()`,
    frame.contextId,
  );
  const x = origin.x + rect.x + rect.width / 2;
  const y = origin.y + rect.y + rect.height / 2;
  await client.call("Input.dispatchMouseEvent", {
    type: "mousePressed",
    x,
    y,
    button: "left",
    clickCount: 1,
  });
  await client.call("Input.dispatchMouseEvent", {
    type: "mouseReleased",
    x,
    y,
    button: "left",
    clickCount: 1,
  });
  await sleep(250);
  return { x, y };
}

function rootSnapshotExpression() {
  return `(() => {
    const root = document.querySelector('[data-testid="renderer-v2-probe"]');
    if (!root) return null;
    const cases = [...root.querySelectorAll('[data-case-id]')].map((section) => ({
      id: section.dataset.caseId,
      parserPass: section.dataset.parserPass,
      blockTypes: section.dataset.blockTypes,
      markerPolicy: section.dataset.markerPolicy,
      markerVisible: section.innerText.includes('<<<[TOOL_REQUEST]>>>'),
      toolBlockCount: section.querySelectorAll('.vcp-tool-block').length,
      iframeCount: section.querySelectorAll('iframe').length,
      text: section.innerText.slice(0, 600)
    }));
    return {
      ready: root.dataset.probeReady,
      parserPassCount: Number(root.dataset.parserPassCount),
      parserCaseCount: Number(root.dataset.parserCaseCount),
      blockSummary: root.dataset.blockSummary,
      aiActionCount: Number(root.dataset.aiActionCount),
      lastAiAction: root.dataset.lastAiAction,
      streamRenderCount: Number(root.dataset.streamRenderCount),
      identityPreserved: root.dataset.identityPreserved,
      reloadCount: Number(root.dataset.reloadCount),
      viewport: {
        width: innerWidth,
        height: innerHeight,
        scrollWidth: document.documentElement.scrollWidth,
        scrollHeight: document.documentElement.scrollHeight
      },
      selectors: {
        unclosed: Boolean(document.querySelector('[data-probe="unclosed-visible"]')),
        closed: Boolean(document.querySelector('[data-probe="closed-visible"]')),
        stuck: Boolean(document.querySelector('[data-probe="stuck-visible"]')),
        nested: Boolean(document.querySelector('[data-probe="nested-visible"]')),
        nestedTail: Boolean(document.querySelector('[data-probe="nested-tail"]')),
        malformed: Boolean(document.querySelector('[data-probe="malformed-visible"]'))
      },
      cases
    };
  })()`;
}

async function waitForProbe(client, minimumReloadCount = 1) {
  return waitFor(
    async () => {
      const snapshot = await evaluate(client, rootSnapshotExpression());
      return snapshot?.ready === "true" &&
        snapshot.reloadCount >= minimumReloadCount
        ? snapshot
        : null;
    },
    `renderer probe reload ${minimumReloadCount}`,
    30_000,
  );
}

async function inspectVisibleCase(client, caseId) {
  const selector = `[data-case-id=${JSON.stringify(caseId)}]`;
  const blockCount = await evaluate(
    client,
    `document.querySelector(${JSON.stringify(selector)}).querySelectorAll('.probe-block').length`,
  );
  const combined = {
    id: caseId,
    markerVisible: false,
    text: "",
    selectors: {
      unclosed: false,
      closed: false,
      stuck: false,
      nested: false,
      nestedTail: false,
      malformed: false,
    },
  };

  for (let index = 0; index < Math.max(1, blockCount); index += 1) {
    await evaluate(
      client,
      `(() => {
        const section = document.querySelector(${JSON.stringify(selector)});
        const target = section.querySelectorAll('.probe-block')[${index}] || section;
        target.scrollIntoView({block:'center',inline:'center'});
      })()`,
    );
    await sleep(300);
    const frame = await evaluate(
      client,
      `(() => {
        const section = document.querySelector(${JSON.stringify(selector)});
        const target = section.querySelectorAll('.probe-block')[${index}] || section;
        return {
          text: target.innerText || '',
          selectors: {
            unclosed: Boolean(target.querySelector('[data-probe="unclosed-visible"]')),
            closed: Boolean(target.querySelector('[data-probe="closed-visible"]')),
            stuck: Boolean(target.querySelector('[data-probe="stuck-visible"]')),
            nested: Boolean(target.querySelector('[data-probe="nested-visible"]')),
            nestedTail: Boolean(target.querySelector('[data-probe="nested-tail"]')),
            malformed: Boolean(target.querySelector('[data-probe="malformed-visible"]'))
          }
        };
      })()`,
    );
    combined.text += `${frame.text}\n`;
    combined.markerVisible ||= frame.text.includes("<<<[TOOL_REQUEST]>>>");
    for (const [key, value] of Object.entries(frame.selectors)) {
      combined.selectors[key] ||= value;
    }
  }
  return combined;
}

async function captureScreenshot(client, outputPath) {
  try {
    const { data } = await client.call(
      "Page.captureScreenshot",
      {
        format: "png",
        fromSurface: true,
        captureBeyondViewport: false,
      },
      20_000,
    );
    fs.writeFileSync(outputPath, Buffer.from(data, "base64"));
    return "cdp";
  } catch (error) {
    if (!String(error).includes("CDP timeout: Page.captureScreenshot")) {
      throw error;
    }
    const data = runAdb(["exec-out", "screencap", "-p"], {
      encoding: "buffer",
      maxBuffer: 32 * 1024 * 1024,
    });
    const pngSignature = Buffer.from([0x89, 0x50, 0x4e, 0x47]);
    if (!Buffer.isBuffer(data) || !data.subarray(0, 4).equals(pngSignature)) {
      throw new Error("Android screencap did not return a PNG image");
    }
    fs.writeFileSync(outputPath, data);
    return "adb-screencap";
  }
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const artifactDir = ensureDir(path.join(ROOT, "test-artifacts", "emulator"));
  const reportPath = path.join(
    artifactDir,
    `renderer-runtime-${args.label}.json`,
  );
  const screenshotPath = path.join(
    artifactDir,
    `renderer-runtime-${args.label}.png`,
  );
  const landscapePath = path.join(
    artifactDir,
    `renderer-runtime-${args.label}-landscape.png`,
  );
  const checks = [];
  const failures = [];
  const runtimeExceptions = [];
  const consoleErrors = [];
  let client;

  function check(condition, name, details) {
    const entry = { name, passed: Boolean(condition), details };
    checks.push(entry);
    if (!condition) failures.push(entry);
  }

  try {
    const connection = await connectToWebView(args.port);
    client = connection.client;
    client.on("Runtime.exceptionThrown", (event) =>
      runtimeExceptions.push(event.exceptionDetails),
    );
    client.on("Runtime.consoleAPICalled", (event) => {
      if (event.type === "error") consoleErrors.push(event);
    });
    await Promise.all([
      client.call("Page.enable"),
      client.call("Runtime.enable"),
      client.call("DOM.enable"),
      client.call("Log.enable"),
    ]);

    await client.call("Page.navigate", {
      url: "http://tauri.localhost/#/renderer-v2-probe",
    });
    let snapshot = await waitForProbe(client);
    const initialReloadCount = snapshot.reloadCount;
    check(
      snapshot.parserPassCount === snapshot.parserCaseCount &&
        snapshot.parserCaseCount === 9,
      "all Rust parser fixtures passed required block contracts",
      `${snapshot.parserPassCount}/${snapshot.parserCaseCount}: ${snapshot.blockSummary}`,
    );
    check(
      snapshot.cases.every((item) => item.parserPass === "true"),
      "every parser case reports pass",
      snapshot.cases,
    );
    const visibleCases = [];
    for (const item of snapshot.cases) {
      visibleCases.push({
        ...item,
        ...(await inspectVisibleCase(client, item.id)),
      });
    }
    check(
      visibleCases
        .filter((item) => item.markerPolicy === "hidden")
        .every((item) => !item.markerVisible),
      "tool protocol markers do not leak into rendered text",
      visibleCases.map((item) => ({
        id: item.id,
        markerVisible: item.markerVisible,
      })),
    );
    check(
      visibleCases
        .filter((item) => item.markerPolicy === "literal")
        .every((item) => item.markerVisible),
      "literal marker inside pre/code remains visible and is not parsed as a tool",
      visibleCases.filter((item) => item.markerPolicy === "literal"),
    );
    const restoredSelectors = visibleCases.reduce(
      (all, item) => {
        for (const [key, value] of Object.entries(item.selectors)) {
          all[key] ||= value;
        }
        return all;
      },
      {
        unclosed: false,
        closed: false,
        stuck: false,
        nested: false,
        nestedTail: false,
        malformed: false,
      },
    );
    check(
      Object.values(restoredSelectors).every(Boolean),
      "all repaired rich HTML sentinels restore after scrolling into view",
      restoredSelectors,
    );
    check(
      snapshot.streamRenderCount === 3 && snapshot.identityPreserved === "true",
      "three simulated stream frames patch without replacing the scene root",
      {
        streamRenderCount: snapshot.streamRenderCount,
        identityPreserved: snapshot.identityPreserved,
      },
    );
    check(
      snapshot.viewport.scrollWidth <= snapshot.viewport.width + 1,
      "portrait probe has no document-level horizontal overflow",
      snapshot.viewport,
    );

    const actionFrame = await findFrame(client, "#local-toggle");
    await clickFrameControl(client, actionFrame, "#local-toggle");
    let localState = await evaluate(
      client,
      `({ count: document.querySelector('#local-count').textContent, hidden: document.querySelector('#toggle-target').hidden })`,
      actionFrame.contextId,
    );
    snapshot = await waitForProbe(client);
    check(
      localState.count === "1" && localState.hidden === false,
      "local show/hide button executes inside the sandbox",
      localState,
    );
    check(
      snapshot.aiActionCount === 0,
      "local show/hide button does not send a message to AI",
      snapshot.aiActionCount,
    );

    await clickFrameControl(client, actionFrame, "#cursor-card");
    localState = await evaluate(
      client,
      `({ count: document.querySelector('#local-count').textContent, hidden: document.querySelector('#toggle-target').hidden })`,
      actionFrame.contextId,
    );
    snapshot = await waitForProbe(client);
    check(
      localState.count === "2" && snapshot.aiActionCount === 0,
      "cursor-pointer card stays local and does not send to AI",
      { localState, aiActionCount: snapshot.aiActionCount },
    );

    await clickFrameControl(client, actionFrame, "#plain-button");
    snapshot = await waitForProbe(client);
    check(
      snapshot.aiActionCount === 0,
      "ordinary button without opt-in does not send to AI",
      snapshot.aiActionCount,
    );

    await clickFrameControl(client, actionFrame, "#ai-send");
    snapshot = await waitFor(async () => {
      const value = await evaluate(client, rootSnapshotExpression());
      return value?.aiActionCount === 1 ? value : null;
    }, "explicit AI action");
    check(
      snapshot.aiActionCount === 1 &&
        snapshot.lastAiAction.includes("explicit runtime action"),
      "only data-vcp-send dispatches the wrapped AI action",
      {
        aiActionCount: snapshot.aiActionCount,
        lastAiAction: snapshot.lastAiAction,
      },
    );
    const screenshotModes = {
      portrait: await captureScreenshot(client, screenshotPath),
      landscape: "pending",
    };

    runAdb([
      "shell",
      "settings",
      "put",
      "system",
      "accelerometer_rotation",
      "0",
    ]);
    runAdb(["shell", "settings", "put", "system", "user_rotation", "1"]);
    await sleep(1_500);
    const landscape = await evaluate(client, rootSnapshotExpression());
    check(
      landscape.viewport.width > landscape.viewport.height,
      "activity rotates to landscape",
      landscape.viewport,
    );
    check(
      landscape.viewport.scrollWidth <= landscape.viewport.width + 1,
      "landscape probe has no document-level horizontal overflow",
      landscape.viewport,
    );
    screenshotModes.landscape = await captureScreenshot(client, landscapePath);
    runAdb(["shell", "settings", "put", "system", "user_rotation", "0"]);
    await sleep(1_000);

    await client.call("Page.reload", { ignoreCache: true });
    snapshot = await waitForProbe(client, initialReloadCount + 1);
    check(
      snapshot.parserPassCount === 9 && snapshot.streamRenderCount === 3,
      "parser and renderer pass again after a full page reload",
      {
        reloadCount: snapshot.reloadCount,
        parserPassCount: snapshot.parserPassCount,
        streamRenderCount: snapshot.streamRenderCount,
      },
    );

    runAdb(["shell", "input", "keyevent", "3"]);
    await sleep(800);
    runAdb(["shell", "am", "start", "-n", ACTIVITY]);
    await sleep(1_200);
    snapshot = await waitForProbe(client, initialReloadCount + 1);
    check(
      snapshot.ready === "true",
      "probe survives an Android background/foreground cycle",
      { ready: snapshot.ready, reloadCount: snapshot.reloadCount },
    );

    const logcat = runAdb(["logcat", "-d", "-v", "brief"], {
      maxBuffer: 32 * 1024 * 1024,
    });
    const fatalLines = logcat
      .split(/\r?\n/)
      .filter((line) =>
        /FATAL EXCEPTION|ANR in com\.vcp\.avatar\.debug|Fatal signal|Process com\.vcp\.avatar\.debug .* died/i.test(
          line,
        ),
      );
    check(
      fatalLines.length === 0,
      "no app crash, fatal signal, or ANR appeared in logcat",
      fatalLines,
    );
    check(
      runtimeExceptions.length === 0,
      "no uncaught JavaScript exception occurred during the probe",
      runtimeExceptions,
    );

    const report = {
      label: args.label,
      package: PACKAGE,
      targetUrl: connection.target.url,
      pid: connection.pid,
      generatedAt: new Date().toISOString(),
      passed: failures.length === 0,
      checks,
      failures,
      runtimeExceptions,
      consoleErrorCount: consoleErrors.length,
      screenshotModes,
      visibleCases,
      finalSnapshot: snapshot,
      screenshotPath,
      landscapePath,
    };
    fs.writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`);
    console.log(JSON.stringify(report, null, 2));
    if (failures.length > 0) process.exitCode = 1;
  } finally {
    try {
      runAdb(["shell", "settings", "put", "system", "user_rotation", "0"], {
        allowFailure: true,
      });
      runAdb(
        ["shell", "settings", "put", "system", "accelerometer_rotation", "1"],
        {
          allowFailure: true,
        },
      );
    } catch {}
    client?.close();
  }
}

main().catch((error) => {
  console.error(error.stack || error);
  process.exitCode = 1;
});
