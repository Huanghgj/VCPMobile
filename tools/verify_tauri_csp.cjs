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

for (const requirement of [
  /\bscript-src\b[^;]*'unsafe-inline'/,
  /\bscript-src\b[^;]*https:/,
  /\bframe-src\b[^;]*https:/,
  /\bworker-src\b[^;]*blob:/,
]) {
  if (!requirement.test(csp)) {
    throw new Error(
      "Tauri CSP must allow executable remote HTML previews, frames and workers",
    );
  }
}

if (
  !Array.isArray(disabledDirectives) ||
  !disabledDirectives.includes("style-src") ||
  !disabledDirectives.includes("script-src")
) {
  throw new Error(
    "Tauri asset CSP modification must be disabled for style-src and script-src; generated nonces make 'unsafe-inline' ineffective",
  );
}

console.log("Tauri rich-message CSP configuration is valid");
