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

const gradleArgs = [
  "--project-dir",
  androidProjectDir,
  ":tauri-plugin-vcp-mobile:compileReleaseKotlin",
];
const command =
  process.platform === "win32" ? process.env.ComSpec || "cmd.exe" : gradlew;
const commandArgs =
  process.platform === "win32"
    ? ["/d", "/s", "/c", gradlew, ...gradleArgs]
    : gradleArgs;
const result = spawnSync(command, commandArgs, {
  cwd: repoRoot,
  env,
  stdio: "inherit",
});

if (result.error) {
  console.error(result.error.message);
}

process.exit(result.status ?? 1);
