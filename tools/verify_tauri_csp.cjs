const { readFileSync } = require("node:fs");
const { resolve } = require("node:path");

const configPath = resolve(__dirname, "..", "src-tauri", "tauri.conf.json");
const config = JSON.parse(readFileSync(configPath, "utf8"));
const security = config.app?.security || {};
const csp = typeof security.csp === "string" ? security.csp : "";
const disabledDirectives = security.dangerousDisableAssetCspModification;

if (!/\bstyle-src\b[^;]*'unsafe-inline'/.test(csp)) {
  throw new Error(
    "Tauri CSP must explicitly allow inline styles for sanitized rich messages",
  );
}

if (
  !Array.isArray(disabledDirectives) ||
  !disabledDirectives.includes("style-src")
) {
  throw new Error(
    "Tauri asset CSP modification must be disabled for style-src; its generated nonce makes 'unsafe-inline' ineffective",
  );
}

console.log("Tauri rich-message CSP configuration is valid");
