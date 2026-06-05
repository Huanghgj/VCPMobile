# Agent Notes for VCPMobile

These instructions apply to this repository and all child directories.

## Project Boundary

- Work in `/root/VCPMobile` by default.
- Do not modify `/root/VCPToolBox` unless the user explicitly asks for server-side changes.
- The app should adapt to the existing VCPToolBox protocol and payloads.
- The user usually expects replies in Chinese unless they ask otherwise.

## VCPToolBox Integration Context

- VCPToolBox runs separately under PM2, usually with the name `vcptoolbox`.
- VCPMobile consumes the server from the app side.
- VCPInfo notifications use the WebSocket endpoint `/vcpinfo/VCP_Key=...`.
- The VCPInfo URL is derived from the existing VCPLog URL when possible.
- Keep VCPInfo handling tolerant of unknown payload fields; display extra details rather than discarding them.

## Notification Details

- RAG notifications may include retrieval details, sources, scores, distances, snippets, paths, and metadata.
- If the server provides `distance`, show that value.
- If `distance` is missing but `score` exists, the app may display an estimated distance as `dist≈(1/score)-1`.
- Other VCPInfo notification types should also expose useful structured details in the notification UI.

## Checks

Preferred project checks:

```bash
pnpm check
pnpm exec vue-tsc --noEmit
cd src-tauri && cargo fmt --check
cd src-tauri && cargo check
```

Avoid using `cargo check -p tauri-plugin-vcp-mobile` as the main verification command for this project layout; use full project checks instead.

## Android Builds and Release Packaging

- Do **not** build APKs locally in this workspace unless the user explicitly overrides this instruction.
- Do **not** run local Android packaging commands such as:

```bash
pnpm android:arm64:debug
pnpm android:arm64:release
tauri android build
```

- APK packaging should be done by pushing the full change set to GitHub and triggering GitHub Actions (`.github/workflows/release.yml`) via a release tag (`v*`), a published GitHub Release, or `workflow_dispatch`.
- When release packaging is needed, commit and push all relevant source/config changes first so GitHub builds from the complete repository state.
- Real release signing must be configured in GitHub repository secrets / workflow environment, including:

```text
ANDROID_KEYSTORE_BASE64
ANDROID_KEY_ALIAS
ANDROID_KEYSTORE_PASSWORD
ANDROID_KEY_PASSWORD
```

- Local validation should use non-APK checks only, for example the commands listed in the `Checks` section.

## Build Speed

- `sccache` is installed at `/root/.cargo/bin/sccache` in this environment.
- The project has `.cargo/config.toml` configured with `rustc-wrapper = "sccache"`.
- Check compiler cache usage with:

```bash
pnpm sccache:stats
```
