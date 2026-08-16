const { spawnSync } = require("node:child_process");
const {
  copyFileSync,
  existsSync,
  mkdirSync,
  readdirSync,
  statSync,
} = require("node:fs");
const { createHash } = require("node:crypto");
const { dirname, join, resolve } = require("node:path");

const root = resolve(__dirname, "..");
const androidDir = join(root, "src-tauri", "gen", "android");
const localSigningHelperGuard = "VCPMOBILE_RELEASE_HELPER_ACTIVE";
const githubUploadGuard = "VCPMOBILE_UPLOAD_GITHUB";
const expectedReleaseCertificateDigest =
  "35fa1397daa390653c67049f413b575ebd8bcb0c3ab76badcd3db21e794f6a1d";
const sccacheBin =
  process.env.SCCACHE ||
  (isWindowsPathCandidate()
    ? join(process.env.USERPROFILE || "", ".cargo", "bin", "sccache.exe")
    : "sccache");

const TARGETS = {
  aarch64: {
    rustTarget: "aarch64-linux-android",
    jniAbi: "arm64-v8a",
    apkFlavor: "arm64",
    gradleFlavor: "Arm64",
  },
  x86_64: {
    rustTarget: "x86_64-linux-android",
    jniAbi: "x86_64",
    apkFlavor: "x86_64",
    gradleFlavor: "X86_64",
  },
};

function readTargetArg() {
  const direct = process.argv.find((arg) => arg.startsWith("--target="));
  if (direct) return direct.slice("--target=".length);
  const index = process.argv.indexOf("--target");
  if (index >= 0 && process.argv[index + 1]) return process.argv[index + 1];
  return process.env.ANDROID_BUILD_TARGET || "aarch64";
}

function findLocalSigningHelper() {
  if (!isWindowsPathCandidate()) return null;

  const configured = String(process.env.VCPMOBILE_RELEASE_HELPER || "").trim();
  const userProfile = String(process.env.USERPROFILE || "").trim();
  const candidates = [
    configured,
    userProfile
      ? join(
          userProfile,
          ".android",
          "vcp-mobile-release",
          "build-vcpmobile-release.ps1",
        )
      : "",
  ].filter(Boolean);

  return candidates.find((candidate) => existsSync(candidate)) || null;
}

function delegateToLocalSigningHelper(helperPath, uploadToGitHub) {
  const windowsRoot =
    process.env.SystemRoot || process.env.WINDIR || "C:\\Windows";
  const bundledPowerShell = join(
    windowsRoot,
    "System32",
    "WindowsPowerShell",
    "v1.0",
    "powershell.exe",
  );
  const powershell = existsSync(bundledPowerShell)
    ? bundledPowerShell
    : "powershell.exe";

  console.log(
    `[android-release] using persisted local release-signing helper: ${helperPath}`,
  );
  const delegated = spawnSync(
    powershell,
    [
      "-NoProfile",
      "-ExecutionPolicy",
      "Bypass",
      "-File",
      helperPath,
      "-Repository",
      root,
    ],
    {
      cwd: root,
      stdio: "inherit",
      shell: false,
      env: {
        ...process.env,
        [localSigningHelperGuard]: "1",
        [githubUploadGuard]: uploadToGitHub ? "1" : "0",
      },
    },
  );
  if (delegated.error) {
    console.error(
      `[android-release] failed to start local signing helper: ${delegated.error.message}`,
    );
  }
  process.exit(delegated.status ?? 1);
}

const targetName = readTargetArg();
const targetConfig = TARGETS[targetName];
if (!targetConfig) {
  console.error(
    `[android-release] Unsupported target '${targetName}'. Supported: ${Object.keys(TARGETS).join(", ")}`,
  );
  process.exit(1);
}

const allowDebugSigning = process.argv.includes("--allow-debug-signing");
const uploadToGitHub =
  process.argv.includes("--upload-github") ||
  process.env[githubUploadGuard] === "1";
if (uploadToGitHub && allowDebugSigning) {
  console.error(
    "[android-release] Refusing to upload an Android Debug-signed APK to GitHub.",
  );
  process.exit(2);
}
let usingDefaultDebugSigning = false;
if (allowDebugSigning) {
  const hasAnySigningEnvironment = [
    "ANDROID_KEYSTORE_PATH",
    "ANDROID_KEY_ALIAS",
    "ANDROID_KEYSTORE_PASSWORD",
    "ANDROID_KEY_PASSWORD",
  ].some((name) => String(process.env[name] || "").trim());
  if (!hasAnySigningEnvironment) {
    usingDefaultDebugSigning = true;
    process.env.ANDROID_KEYSTORE_PATH = join(
      process.env.USERPROFILE || "",
      ".android",
      "debug.keystore",
    );
    process.env.ANDROID_KEY_ALIAS = "androiddebugkey";
    process.env.ANDROID_KEYSTORE_PASSWORD = "android";
    process.env.ANDROID_KEY_PASSWORD = "android";
  }
}

const signingEnvironment = [
  "ANDROID_KEYSTORE_PATH",
  "ANDROID_KEY_ALIAS",
  "ANDROID_KEYSTORE_PASSWORD",
  "ANDROID_KEY_PASSWORD",
];
const missingSigningEnvironment = signingEnvironment.filter(
  (name) => !String(process.env[name] || "").trim(),
);
if (
  !allowDebugSigning &&
  !process.argv.includes("--verify-existing") &&
  missingSigningEnvironment.length > 0 &&
  process.env[localSigningHelperGuard] !== "1"
) {
  const localSigningHelper = findLocalSigningHelper();
  if (localSigningHelper) {
    delegateToLocalSigningHelper(localSigningHelper, uploadToGitHub);
  }
}
if (missingSigningEnvironment.length > 0) {
  console.error(
    `[android-release] Refusing to build a release APK without formal signing: missing ${missingSigningEnvironment.join(", ")}`,
  );
  process.exit(2);
}
const releaseKeystorePath = resolve(process.env.ANDROID_KEYSTORE_PATH);
if (usingDefaultDebugSigning && !existsSync(releaseKeystorePath)) {
  mkdirSync(dirname(releaseKeystorePath), { recursive: true });
  console.log(
    `[android-release] generating standard Android debug keystore: ${releaseKeystorePath}`,
  );
  const generated = spawnSync(
    "keytool",
    [
      "-genkeypair",
      "-v",
      "-keystore",
      releaseKeystorePath,
      "-storepass",
      "android",
      "-alias",
      "androiddebugkey",
      "-keypass",
      "android",
      "-keyalg",
      "RSA",
      "-keysize",
      "2048",
      "-validity",
      "10000",
      "-dname",
      "CN=Android Debug,O=Android,C=US",
    ],
    { stdio: "inherit", shell: false },
  );
  if (generated.error || generated.status !== 0) {
    console.error(
      "[android-release] failed to generate Android debug keystore.",
    );
    process.exit(generated.status || 1);
  }
}
if (
  !existsSync(releaseKeystorePath) ||
  statSync(releaseKeystorePath).size === 0
) {
  console.error(
    `[android-release] Release keystore does not exist or is empty: ${releaseKeystorePath}`,
  );
  process.exit(2);
}

const soSrc = join(
  root,
  "src-tauri",
  "target",
  targetConfig.rustTarget,
  "release",
  "libvcp_mobile_lib.so",
);
const soDstDir = join(
  root,
  "src-tauri",
  "gen",
  "android",
  "app",
  "src",
  "main",
  "jniLibs",
  targetConfig.jniAbi,
);
const soDst = join(soDstDir, "libvcp_mobile_lib.so");

const isWindows = process.platform === "win32";
const gradlew = join(androidDir, isWindows ? "gradlew.bat" : "gradlew");

function isWindowsPathCandidate() {
  return process.platform === "win32";
}

function canRunLocalExecutable(command) {
  if (!command) return false;
  if (command.includes("\\") || command.includes("/")) {
    return existsSync(command);
  }
  return true;
}

function pnpmExecArgs(args) {
  if (process.env.npm_execpath) {
    return {
      command: process.execPath,
      args: [process.env.npm_execpath, ...args],
    };
  }
  if (isWindows) {
    return {
      command: process.env.ComSpec || "cmd.exe",
      args: ["/d", "/s", "/c", "pnpm.cmd", ...args],
    };
  }
  return {
    command: "pnpm",
    args,
  };
}

function gradleExecArgs(args) {
  if (isWindows) {
    return {
      command: process.env.ComSpec || "cmd.exe",
      args: ["/c", gradlew, ...args],
    };
  }
  return {
    command: gradlew,
    args,
  };
}

function formatMs(ms) {
  const seconds = ms / 1000;
  if (seconds < 60) return `${seconds.toFixed(1)}s`;
  const minutes = Math.floor(seconds / 60);
  return `${minutes}m ${(seconds - minutes * 60).toFixed(1)}s`;
}

function run(label, command, args, options = {}) {
  const start = Date.now();
  console.log(`\n[android-release] ${label}`);
  console.log(`[android-release] > ${command} ${args.join(" ")}`);
  const result = spawnSync(command, args, {
    cwd: options.cwd || root,
    stdio: options.captureOutput ? ["inherit", "pipe", "pipe"] : "inherit",
    maxBuffer: options.captureOutput ? 64 * 1024 * 1024 : undefined,
    shell: false,
    env: {
      ...process.env,
      CARGO_INCREMENTAL: process.env.CARGO_INCREMENTAL || "0",
    },
  });
  const stdout = result.stdout ? result.stdout.toString() : "";
  const stderr = result.stderr ? result.stderr.toString() : "";
  if (options.captureOutput) {
    if (stdout) process.stdout.write(stdout);
    if (stderr) process.stderr.write(stderr);
  }
  const elapsed = Date.now() - start;
  if (result.error) {
    console.error(
      `[android-release] ${label} failed to start: ${result.error.message}`,
    );
  }
  console.log(`[android-release] ${label} elapsed: ${formatMs(elapsed)}`);
  return { ...result, elapsed, stdout, stderr };
}

function isWindowsSymlinkFailure(result) {
  if (!isWindows || result.status === 0) return false;
  const output = `${result.stdout || ""}\n${result.stderr || ""}`.toLowerCase();
  return (
    output.includes("symlink") ||
    output.includes("symbolic link") ||
    output.includes("os error 1314") ||
    output.includes("a required privilege is not held by the client") ||
    output.includes("??????????")
  );
}

function runOptional(label, command, args, options = {}) {
  if (!canRunLocalExecutable(command)) {
    console.log(`\n[android-release] ${label}`);
    console.log(`[android-release] ${command} not found; skipping.`);
    return;
  }
  const result = run(label, command, args, options);
  if (result.error || result.status !== 0) {
    console.log(`[android-release] ${label} skipped or failed; continuing.`);
  }
}

function findReleaseApk() {
  const apkDir = join(
    root,
    "src-tauri",
    "gen",
    "android",
    "app",
    "build",
    "outputs",
    "apk",
    targetConfig.apkFlavor,
    "release",
  );
  if (!existsSync(apkDir)) return null;
  const apks = readdirSync(apkDir)
    .filter((name) => name.endsWith(".apk"))
    .map((name) => join(apkDir, name))
    .sort((a, b) => statSync(b).mtimeMs - statSync(a).mtimeMs);
  return apks[0] || null;
}

function isFreshApk(path, startedAt) {
  if (!path || !existsSync(path)) return false;
  return statSync(path).mtimeMs >= startedAt;
}

function isUsableApk(path) {
  return Boolean(path && existsSync(path) && statSync(path).size > 0);
}

function hashFile(path) {
  const { readFileSync } = require("node:fs");
  return createHash("sha256")
    .update(readFileSync(path))
    .digest("hex")
    .toUpperCase();
}

function findAndroidBuildTool(toolName) {
  const sdkRoot = process.env.ANDROID_HOME || process.env.ANDROID_SDK_ROOT;
  if (!sdkRoot) return null;
  const buildToolsRoot = join(sdkRoot, "build-tools");
  if (!existsSync(buildToolsRoot)) return null;
  const executables = isWindows
    ? [`${toolName}.bat`, `${toolName}.exe`, toolName]
    : [toolName];
  const versions = readdirSync(buildToolsRoot, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => entry.name)
    .sort((left, right) =>
      right.localeCompare(left, undefined, { numeric: true }),
    );
  for (const version of versions) {
    for (const executable of executables) {
      const candidate = join(buildToolsRoot, version, executable);
      if (existsSync(candidate)) return candidate;
    }
  }
  return null;
}

function androidToolExecArgs(executable, args) {
  if (isWindows && /\.(?:bat|cmd)$/i.test(executable)) {
    return {
      command: process.env.ComSpec || "cmd.exe",
      args: ["/d", "/s", "/c", executable, ...args],
    };
  }
  return { command: executable, args };
}

function normalizeCertificateDigest(value) {
  return String(value || "")
    .replace(/:/g, "")
    .trim()
    .toLowerCase();
}

const totalStart = Date.now();
const verifyExisting = process.argv.includes("--verify-existing");
console.log(`[android-release] target: ${targetName}`);
let apk;

if (verifyExisting) {
  apk = findReleaseApk();
  console.log(
    `[android-release] verifying existing APK: ${apk || "not found"}`,
  );
} else {
  runOptional("sccache stats before", sccacheBin, ["--show-stats"]);

  const pnpmTauri = pnpmExecArgs([
    "exec",
    "tauri",
    "android",
    "build",
    "--target",
    targetName,
    "--apk",
    "--split-per-abi",
    "--ci",
  ]);
  const tauri = run(
    `tauri android ${targetName} release`,
    pnpmTauri.command,
    pnpmTauri.args,
    { captureOutput: true },
  );

  apk = findReleaseApk();
  const needsGradleFallback =
    isWindowsSymlinkFailure(tauri) && !isFreshApk(apk, totalStart);

  if (needsGradleFallback) {
    if (!existsSync(soSrc) || statSync(soSrc).size === 0) {
      console.error(
        `[android-release] Rust library not found or empty after failed Tauri build: ${soSrc}`,
      );
      process.exit(tauri.status || 1);
    }

    console.log(
      "\n[android-release] Windows symlink step failed; copying release .so and continuing with Gradle.",
    );
    mkdirSync(soDstDir, { recursive: true });
    copyFileSync(soSrc, soDst);
    console.log(`[android-release] copied ${soSrc}`);
    console.log(`[android-release]     -> ${soDst}`);

    const gradleCommand = gradleExecArgs([
      `:app:assemble${targetConfig.gradleFlavor}Release`,
      "-x",
      `:app:rustBuild${targetConfig.gradleFlavor}Release`,
      "--stacktrace",
    ]);
    const gradle = run(
      `gradle assemble ${targetConfig.apkFlavor} release`,
      gradleCommand.command,
      gradleCommand.args,
      { cwd: androidDir },
    );
    if (gradle.error || gradle.status !== 0) {
      process.exit(gradle.status || 1);
    }
    apk = findReleaseApk();
  } else if (tauri.error || tauri.status !== 0) {
    console.error(
      "[android-release] Tauri build failed for a non-symlink reason; refusing stale artifact fallback.",
    );
    process.exit(tauri.status || 1);
  }

  runOptional("sccache stats after", sccacheBin, ["--show-stats"]);
}

if (!isUsableApk(apk)) {
  console.error("[android-release] Release APK was not produced or is empty.");
  process.exit(1);
}

const apkSigner = findAndroidBuildTool("apksigner");
const aapt = findAndroidBuildTool("aapt");
if (!apkSigner || !aapt) {
  console.error(
    "[android-release] apksigner or aapt was not found in Android build-tools.",
  );
  process.exit(1);
}

const signatureCommand = androidToolExecArgs(apkSigner, ["verify", "-v", apk]);
const signatureCheck = run(
  "verify APK signature",
  signatureCommand.command,
  signatureCommand.args,
  {
    captureOutput: true,
  },
);
if (signatureCheck.error || signatureCheck.status !== 0) {
  process.exit(signatureCheck.status || 1);
}
const certificateCommand = androidToolExecArgs(apkSigner, [
  "verify",
  "--print-certs",
  apk,
]);
const apkCertificate = run(
  "read APK certificate",
  certificateCommand.command,
  certificateCommand.args,
  {
    captureOutput: true,
  },
);
if (apkCertificate.error || apkCertificate.status !== 0) {
  process.exit(apkCertificate.status || 1);
}
const isDebugCertificate = /android debug/i.test(
  `${apkCertificate.stdout}\n${apkCertificate.stderr}`,
);
if (isDebugCertificate && !allowDebugSigning) {
  console.error(
    "[android-release] APK is signed with Android Debug key; refusing artifact.",
  );
  process.exit(1);
}
if (isDebugCertificate) {
  console.warn(
    "[android-release] Explicit fallback enabled: accepting Android Debug certificate for this release build.",
  );
}
const apkDigest = normalizeCertificateDigest(
  apkCertificate.stdout.match(
    /certificate SHA-256 digest:\s*([0-9a-f:]+)/i,
  )?.[1],
);
const keystoreCertificate = run(
  "read release keystore certificate",
  "keytool",
  [
    "-list",
    "-v",
    "-keystore",
    releaseKeystorePath,
    "-alias",
    process.env.ANDROID_KEY_ALIAS,
    "-storepass:env",
    "ANDROID_KEYSTORE_PASSWORD",
  ],
  { captureOutput: true },
);
if (keystoreCertificate.error || keystoreCertificate.status !== 0) {
  process.exit(keystoreCertificate.status || 1);
}
const keystoreDigest = normalizeCertificateDigest(
  keystoreCertificate.stdout.match(/SHA256:\s*([0-9A-F:]+)/i)?.[1],
);
if (!apkDigest || !keystoreDigest || apkDigest !== keystoreDigest) {
  console.error(
    "[android-release] APK certificate does not match the configured release keystore.",
  );
  process.exit(1);
}
if (!isDebugCertificate && apkDigest !== expectedReleaseCertificateDigest) {
  console.error(
    "[android-release] APK certificate does not match the pinned VCPMobile release certificate.",
  );
  process.exit(1);
}

const badging = run(
  "inspect APK manifest and ABI",
  aapt,
  ["dump", "badging", apk],
  {
    captureOutput: true,
  },
);
if (badging.error || badging.status !== 0) {
  process.exit(badging.status || 1);
}
if (/application-debuggable/i.test(badging.stdout)) {
  console.error("[android-release] APK is debuggable; refusing artifact.");
  process.exit(1);
}
const nativeCodeLine = badging.stdout.match(/^native-code:.*$/m)?.[0] || "";
const packagedAbis = Array.from(
  nativeCodeLine.matchAll(/'([^']+)'/g),
  (match) => match[1],
);
if (packagedAbis.length !== 1 || packagedAbis[0] !== targetConfig.jniAbi) {
  console.error(
    `[android-release] Unexpected native ABI set: ${nativeCodeLine || "missing"}`,
  );
  process.exit(1);
}

const version = require(join(root, "src-tauri", "tauri.conf.json")).version;
const artifactDir = join(root, "release-artifacts");
const signingSuffix = isDebugCertificate ? "_release-debug-signed" : "";
let artifactPath = join(
  artifactDir,
  `VCPMobile_v${version}_${targetConfig.jniAbi}${signingSuffix}.apk`,
);
if (existsSync(artifactPath)) {
  const buildStamp = new Date().toISOString().replace(/[-:]/g, "").slice(0, 15);
  artifactPath = join(
    artifactDir,
    `VCPMobile_v${version}_${targetConfig.jniAbi}${signingSuffix}-${buildStamp}.apk`,
  );
}
mkdirSync(artifactDir, { recursive: true });
copyFileSync(apk, artifactPath);

console.log("\n[android-release] APK ready");
console.log(`[android-release] path: ${artifactPath}`);
console.log(`[android-release] sha256: ${hashFile(artifactPath)}`);
if (uploadToGitHub) {
  const uploaderPath = join(
    root,
    "scripts",
    "upload_android_release_asset.cjs",
  );
  const upload = run("upload APK to GitHub Release", process.execPath, [
    uploaderPath,
    "--artifact",
    artifactPath,
  ]);
  if (upload.error || upload.status !== 0) {
    console.error(
      `[android-release] Local APK remains available at: ${artifactPath}`,
    );
    process.exit(upload.status || 1);
  }
}
console.log(
  `[android-release] total elapsed: ${formatMs(Date.now() - totalStart)}`,
);
