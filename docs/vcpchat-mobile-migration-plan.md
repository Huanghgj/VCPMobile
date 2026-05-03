# VCPChat Mobile Migration Plan

## Context

- Desktop reference: `D:\VCPChat-upstream`, upstream `lioensky/VCPChat`, checked at `d9424722ea48f0cea5681ea8aba66c8ec39e9add`.
- Mobile target: this repository, `VCPMobile`.
- Goal: build mobile-appropriate parity with VCPChat. Do not copy Electron window behavior verbatim; convert useful desktop windows into mobile routes, sheets, overlays, and persistent surfaces.
- Group chat is not a priority for now.

## Priority 1: Message Rendering Parity

The first work item is making assistant replies render the same classes of embedded content as desktop:

- VCP tool request blocks: `<<<[TOOL_REQUEST]>>>...<<<[END_TOOL_REQUEST]>>>`.
- VCP tool result blocks: `[[VCP调用结果信息汇总:...VCP调用结果结束]]`.
- Tool result preview fields with Markdown, images, video, and audio.
- Thought chains, role dividers, HTML previews, diary blocks, and desktop-push/surface blocks.

Acceptance criteria:

- During streaming, complete tool result blocks appear inline inside the assistant bubble.
- After streaming completes, the same inline tool result survives AST precompile, database persistence, reload, and copy/edit operations.
- Tool result media links render as tappable previews rather than raw URLs where safe.
- Incomplete streaming result blocks show a pending state and are replaced by the complete preview once the end marker arrives.
- VCP tool injection settings are read from the same flattened settings shape that the settings UI writes.

Implementation notes:

- Desktop protects tool result blocks before Markdown and restores them after Markdown rendering.
- Mobile should keep the AST path: split special blocks during streaming, parse with Rust after finalization, and render through `ToolBlock.vue`.
- The Rust VCP client must not drop tool-result text while normalizing OpenAI/Gemini/Anthropic stream chunks.

## Priority 2: Memo / Note

Desktop modules:

- `Memomodules/*`
- `Notemodules/*`

Mobile target:

- A mobile Memo/Note workspace exposed as a route or overlay from the toolbox/settings area.
- Keep note search, creation, editing, and AI-assisted memo operations.
- Reuse the existing mobile attachment and Markdown rendering stack.
- Prefer local SQLite-backed storage with sync hooks rather than desktop file-window coupling.

Acceptance criteria:

- Create, edit, delete, and search notes/memos.
- Trigger the existing VCP memo tools and render their results inline when called from chat.
- Notes can be opened from chat/tool results without leaving broken back-stack state.

## Priority 3: Selective Standalone Modules

Migrate only modules that have clear mobile value.

High ROI:

- Translator: convert to a compact full-screen panel or bottom sheet.
- Canvas: migrate as a touch-first drawing/surface tool.
- Dice: lightweight modal/tool panel.
- Music: keep playback state injection and basic controls first; defer complex desktop playlist UI.

Conditional:

- Voice: depends on Android/iOS WebView and native permission behavior.
- Forum: useful, but lower priority unless it is part of the current workflow.

Acceptance criteria:

- Each migrated module has a mobile navigation entry, close/back behavior, persistence, and a VCP tool integration path.
- Each module avoids desktop-only assumptions such as draggable Electron windows or Node filesystem access from the renderer.

## Priority 4: Distributed Plugin Runtime

Desktop reference:

- `VCPDistributedServer/*`
- `VCPHumanToolBox/*`

Mobile direction:

- Treat mobile as a safe plugin client and selective runtime host.
- Prefer manifest-driven remote tools, mobile-native prompt tools, and sandboxed HTTP/WebSocket integrations.
- Do not assume arbitrary local stdio/node plugin execution is acceptable on phone.

Acceptance criteria:

- Mobile can discover remote distributed tools.
- Mobile can execute approved mobile-safe tools and return results.
- Prompt/approval interactions are touch-friendly and survive app backgrounding where possible.

## Deferred

- Group chat feature work beyond keeping existing behavior from regressing.
- One-to-one Electron window parity.
- Large desktop-only admin surfaces unless a mobile workflow needs them.

## Current Sprint

1. Fix inline tool preview parity in chat rendering.
2. Verify streaming and final persisted rendering with representative tool result content.
3. Keep a short implementation note in this document as each desktop feature migrates.
4. Use the runtime diagnostic test API (`docs/runtime-diagnostic-test-api.md`) to inject representative AI replies into an installed Android build from the host computer.
