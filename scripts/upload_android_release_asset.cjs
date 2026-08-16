const { spawnSync } = require("node:child_process");
const { createHash } = require("node:crypto");
const { existsSync, readFileSync, statSync } = require("node:fs");
const { basename, join, resolve } = require("node:path");

const root = resolve(__dirname, "..");

function readArg(name) {
  const direct = process.argv.find((arg) => arg.startsWith(`${name}=`));
  if (direct) return direct.slice(name.length + 1);
  const index = process.argv.indexOf(name);
  if (index >= 0 && process.argv[index + 1]) return process.argv[index + 1];
  return "";
}

function fail(message, status = 1) {
  console.error(`[github-release] ${message}`);
  process.exit(status);
}

function hashFile(path) {
  return createHash("sha256")
    .update(readFileSync(path))
    .digest("hex")
    .toLowerCase();
}

function runGitHub(args, captureOutput = false) {
  const command =
    String(process.env.VCPMOBILE_GH || "").trim() ||
    (process.platform === "win32" ? "gh.exe" : "gh");
  const result = spawnSync(command, args, {
    cwd: root,
    env: process.env,
    shell: false,
    stdio: captureOutput ? ["inherit", "pipe", "pipe"] : "inherit",
    maxBuffer: captureOutput ? 16 * 1024 * 1024 : undefined,
  });
  if (result.error) {
    fail(`Failed to start ${command}: ${result.error.message}`);
  }
  return {
    ...result,
    stdout: result.stdout ? result.stdout.toString() : "",
    stderr: result.stderr ? result.stderr.toString() : "",
  };
}

function readRelease(repo, tag) {
  const result = runGitHub(
    ["release", "view", tag, "--repo", repo, "--json", "assets,url"],
    true,
  );
  if (result.status !== 0) {
    if (result.stderr) process.stderr.write(result.stderr);
    fail(`Cannot read GitHub Release ${repo}@${tag}.`);
  }
  try {
    return JSON.parse(result.stdout);
  } catch (error) {
    fail(`GitHub returned invalid release metadata: ${error.message}`);
  }
}

const artifactArg = readArg("--artifact");
if (!artifactArg) fail("Missing required --artifact <apk-path> argument.", 2);

const artifactPath = resolve(root, artifactArg);
if (!existsSync(artifactPath) || statSync(artifactPath).size === 0) {
  fail(`APK does not exist or is empty: ${artifactPath}`, 2);
}
if (!artifactPath.toLowerCase().endsWith(".apk")) {
  fail(`Refusing to upload a non-APK artifact: ${artifactPath}`, 2);
}

const version = require(join(root, "src-tauri", "tauri.conf.json")).version;
const repo =
  readArg("--repo") ||
  String(process.env.VCPMOBILE_GITHUB_REPO || "").trim() ||
  "Huanghgj/VCPMobile";
const tag =
  readArg("--tag") ||
  String(process.env.VCPMOBILE_GITHUB_RELEASE_TAG || "").trim() ||
  `v${version}`;
const assetName = basename(artifactPath);
const assetSize = statSync(artifactPath).size;
const sha256 = hashFile(artifactPath);
const expectedDigest = `sha256:${sha256}`;

function verifyAsset(asset, context) {
  if (!asset) fail(`${context}: uploaded asset is missing.`);
  if (asset.state !== "uploaded") {
    fail(`${context}: unexpected asset state '${asset.state || "missing"}'.`);
  }
  if (asset.size !== assetSize) {
    fail(`${context}: size mismatch (${asset.size} != ${assetSize}).`);
  }
  if (String(asset.digest || "").toLowerCase() !== expectedDigest) {
    fail(`${context}: SHA-256 digest mismatch.`);
  }
}

console.log(`[github-release] repo: ${repo}`);
console.log(`[github-release] tag: ${tag}`);
console.log(`[github-release] asset: ${assetName}`);
console.log(`[github-release] sha256: ${sha256.toUpperCase()}`);

let release = readRelease(repo, tag);
let asset = release.assets.find((candidate) => candidate.name === assetName);
if (asset) {
  verifyAsset(asset, "Existing asset verification failed");
  console.log(`[github-release] already uploaded: ${asset.url}`);
  process.exit(0);
}

const label =
  readArg("--label") || `VCPMobile ${version} ARM64 formal signed build`;
const upload = runGitHub([
  "release",
  "upload",
  tag,
  `${artifactPath}#${label}`,
  "--repo",
  repo,
]);
if (upload.status !== 0) {
  fail(`Upload failed; local APK is unchanged at ${artifactPath}.`);
}

release = readRelease(repo, tag);
asset = release.assets.find((candidate) => candidate.name === assetName);
verifyAsset(asset, "Post-upload verification failed");
console.log(`[github-release] uploaded: ${asset.url}`);
