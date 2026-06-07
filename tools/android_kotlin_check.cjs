#!/usr/bin/env node
const { existsSync } = require("node:fs");
const { spawnSync } = require("node:child_process");
const { join } = require("node:path");

const repoRoot = process.cwd();
const androidProjectDir = join(repoRoot, "src-tauri", "gen", "android");
const gradlew = join(
  androidProjectDir,
  process.platform === "win32" ? "gradlew.bat" : "gradlew",
);

const env = { ...process.env };
const sdkDir = [
  env.ANDROID_HOME,
  env.ANDROID_SDK_ROOT,
  "/root/Android/Sdk",
  join(env.HOME || "", "Android", "Sdk"),
]
  .filter(Boolean)
  .find((candidate) => existsSync(candidate));

if (sdkDir) {
  env.ANDROID_HOME = sdkDir;
  env.ANDROID_SDK_ROOT = sdkDir;
}

const result = spawnSync(
  gradlew,
  ["--project-dir", androidProjectDir, ":tauri-plugin-vcp-mobile:compileReleaseKotlin"],
  { cwd: repoRoot, env, stdio: "inherit" },
);

process.exit(result.status ?? 1);
