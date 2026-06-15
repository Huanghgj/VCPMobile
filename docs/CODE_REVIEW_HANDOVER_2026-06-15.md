# VCPMobile 代码审查与修复交接文档（2026-06-15）

> 本文档是根据本轮对话历史整理的归档交接资料。注意：本文档创建时曾标注“取消 VCP 流式 25 秒 idle timeout”尚未验证；后续已恢复 `vcp_client.rs` 流式成功分支的 SSE line 初始化、清理重复括号结构、移除 25 秒 idle timeout，并完成 Rust 与前端验证。

## 1. 项目与边界

项目根目录：

```text
/root/VCPMobile
```

项目类型：

```text
Tauri v2 + Vue 3 + Rust Android 应用
```

主要源码目录：

```text
/root/VCPMobile/src
/root/VCPMobile/src-tauri
```

`AGENTS.md` 中的关键约束：

- 默认只在 `/root/VCPMobile` 内工作。
- 不要修改 `/root/VCPToolBox`，除非用户明确要求服务端改动。
- App 应适配现有 VCPToolBox 协议与 payload。
- 用户通常期望中文回复。
- 不要在本地构建 APK，除非用户明确覆盖该限制。
- 避免运行本地 Android 打包命令，例如 `pnpm android:arm64:debug`、`pnpm android:arm64:release`、`tauri android build`。

环境约束：

```text
Platform: Linux
Shell: bash
Working Directory: /root/VCPMobile
Current Date: 2026-06-15
CLI Node.js PID: 2464860
```

进程安全约束：不要使用 `killall node`、`pkill node`、`pkill -f node`、`taskkill /IM node.exe` 等会杀死全部 Node.js 进程的命令。若必须终止 Node 进程，应先列出 PID，并排除 CLI 自身 PID `2464860`。

## 2. 原始目标与任务状态

原始用户需求：

```text
帮助我完整的审查一下全部代码。分析优化的可能以及修复存在的bug
```

后续继续请求：

```text
继续
```

最后一个功能性请求：

```text
取消25秒超时，太离谱了。工具调用需要时间！
```

本次归档请求：

```text
TASK: Create a comprehensive handover document from the conversation history above.
```

### 2.1 已完成的主要任务

第一阶段代码审查与低风险修复已完成，并报告如下验证通过：

```text
pnpm check 通过
pnpm build 通过
cargo fmt --check 通过
cargo clippy --all-targets --all-features -- -D warnings 通过
cargo test --all-targets --all-features 通过，结果为 17 passed; 0 failed
```

已完成的原始 TODO：

```text
todo-1781505693039_rmg04gp  梳理项目结构、技术栈和现有约束
todo-1781505693360_a6gwvj7  运行静态检查/构建/测试，收集可复现错误
todo-1781505693368_6hoe3lb  审查核心源码与潜在缺陷/性能问题
todo-1781505693374_mai0tyq  修复确认存在且低风险的 bug
todo-1781505693380_y3wmx15  复跑验证并记录重要代码库笔记
```

已完成的继续阶段 TODO：

```text
todo-1781509078922_n0257sf  核查并修复高速上传 X-Upload-Token 服务端校验缺失
todo-1781509078957_jvnuuil  核查并修复前端动态 selector / stream 引用计数 / WebSocket 生命周期问题
todo-1781509079235_1k9go52  核查并修复文件注册与高速上传落盘的路径/竞态风险
todo-1781509079241_91habgk  复跑前端与 Rust 校验，记录重要笔记
```

已完成的额外 Rust 测试修复 TODO：

```text
todo-1781511104252_qnvhptd  修复 Rust 测试依赖缺失外部 fixture/Windows 路径导致失败的问题
```

### 2.2 最新 timeout 任务状态

timeout 相关 TODO 已在后续处理中修复并验证：

```text
todo-1781511988645_56np3nf  取消 VCP 流式响应 25 秒 idle timeout，避免长时间工具调用被误杀
```

处理结果：

- `/root/VCPMobile/src-tauri/src/vcp_modules/infra/vcp_client.rs` 的流式成功分支已恢复 `resp.bytes_stream()` → `StreamReader` → `FramedRead<LinesCodec>` 的 `lines` 初始化。
- 已删除 25 秒 idle timeout 分支以及对应“超过 25 秒未收到服务器响应”的错误提示。
- 已清理此前替换导致的重复闭合块结构。
- `grep` 检查确认 `Stream idle timeout`、`超过 25 秒`、`timeout_duration`、`last_activity`、`sleep_until`、`Duration::from_secs(25)` 均无残留。
- `cargo fmt --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test --all-targets --all-features`、`pnpm check`、`pnpm build` 均已通过。

## 3. 文件创建与修改清单

### 3.1 新增文件

```text
/root/VCPMobile/src/core/utils/safeMarkdown.ts
/root/VCPMobile/docs/CODE_REVIEW_HANDOVER_2026-06-15.md
```

本交接文档即第二个新增文件。

### 3.2 已修改过的前端文件

```text
/root/VCPMobile/src/features/rag/RagPayloadDetail.vue
/root/VCPMobile/src/features/assistant/AssistantMessageCard.vue
/root/VCPMobile/src/features/chat/blocks/ToolBlock.vue
/root/VCPMobile/src/core/utils/renderLibraryPreloader.ts
/root/VCPMobile/src/features/chat/blocks/MermaidFullScreenViewer.vue
/root/VCPMobile/src/core/composables/useChatScroll.ts
/root/VCPMobile/src/features/rag/RagObserver.vue
/root/VCPMobile/src/core/stores/floatingAssistant.ts
/root/VCPMobile/src/core/stores/chatStreamStore.ts
```

### 3.3 已修改过的 Rust/Tauri 文件

```text
/root/VCPMobile/src-tauri/src/vcp_modules/chat/stream_block_parser.rs
/root/VCPMobile/src-tauri/src/vcp_modules/chat/aurora_pipeline.rs
/root/VCPMobile/src-tauri/src/vcp_modules/infra/high_speed_channel.rs
/root/VCPMobile/src-tauri/src/vcp_modules/infra/file_manager.rs
/root/VCPMobile/src-tauri/src/vcp_modules/chat/ast_diff.rs
/root/VCPMobile/src-tauri/src/vcp_modules/infra/vcp_client.rs
```

其中 `/root/VCPMobile/src-tauri/src/vcp_modules/infra/vcp_client.rs` 的最新 timeout 修改未验证且疑似存在语法/结构问题。

## 4. 已完成修复详情

### 4.1 统一安全 Markdown 渲染

问题：多个 Vue 组件直接 `marked.parse` 后通过 `v-html` 注入 DOM，存在 raw HTML 注入风险。

新增工具文件：

```text
/root/VCPMobile/src/core/utils/safeMarkdown.ts
```

核心导出：

```text
escapeHtml
sanitizeMarkdownHtml
renderSafeMarkdown
```

设计要点：

- `marked` 统一设置 `gfm: true` 与 `breaks: true`。
- `DOMPurify.sanitize` 使用 HTML profile。
- 禁止 `script`、`iframe`、`object`、`embed`、`applet`、`link`、`meta`。
- 禁止 `srcdoc` 与 `style`。
- URI 只允许 http、https、mailto、tel、blob、asset、data:image、相对路径和 hash。
- `marked.parse` 出错时回退到 `escapeHtml(text)`。

已接入文件：

```text
/root/VCPMobile/src/features/rag/RagPayloadDetail.vue
/root/VCPMobile/src/features/assistant/AssistantMessageCard.vue
/root/VCPMobile/src/features/chat/blocks/ToolBlock.vue
```

保留的 RAG 边界修复：

```text
<Tauri>
[AI]:
[USER]:
```

`RagPayloadDetail.vue` 仍会对 query 文本转义 `<` 与 `>`，并继续把行首 `[AI]:` / `[USER]:` 转义成 Markdown 文本，避免 marked 把它识别成隐藏链接定义。

### 4.2 ToolBlock Markdown 与图片 URL 安全

文件：

```text
/root/VCPMobile/src/features/chat/blocks/ToolBlock.vue
```

调整：

- `renderMarkdown` 改为调用 `renderSafeMarkdown`。
- 保留 `MAX_MARKDOWN_CACHE_SIZE = 50` 的有界缓存。
- 生成图片渲染路径改用 `safeImageUrl(item.value)` 绑定 `href` 与 `src`。

已确认的 `safeImageUrl` 行为：

```text
允许 http/https 图片 URL
允许 data:image/(png|jpeg|jpg|gif|webp);base64
其他值返回空字符串
```

### 4.3 Mermaid SVG 注入防护

文件：

```text
/root/VCPMobile/src/features/chat/blocks/MermaidFullScreenViewer.vue
/root/VCPMobile/src/core/utils/renderLibraryPreloader.ts
```

修复点：

- `MermaidFullScreenViewer.vue` 新增 `DOMPurify` SVG sanitize。
- 模板从 `v-html="svgHtml"` 改为 `v-html="sanitizedSvgHtml"`。
- 禁止 `script` 与 `foreignObject`。
- 禁止 `srcdoc`。
- `renderLibraryPreloader.ts` 初始化 Mermaid 时加入 `securityLevel: "strict"`。

### 4.4 Rust StreamBlockParser 未闭合思考块处理

文件：

```text
/root/VCPMobile/src-tauri/src/vcp_modules/chat/stream_block_parser.rs
/root/VCPMobile/src-tauri/src/vcp_modules/chat/aurora_pipeline.rs
```

问题：流结束时残留 tail 原先会被统一沉淀为 Markdown，导致未闭合 `<think>`、`<thinking>` 或 VCP “元思考链”被当作普通正文展示。

修复：

- `StreamBlockParser::finalize` 改为调用 `build_incomplete_tail_block`。
- 新增 `build_incomplete_semantic_tail_block`。
- 未闭合 `<think>` / `<thinking>` 生成 `StreamBlock::thought("思考过程", ..., is_complete=false, ...)`。
- 未闭合 VCP 元思考链生成对应主题 thought block。
- 普通 Markdown tail 保持原有 Markdown 解析路径。
- `aurora_pipeline.rs` 的 speculative tail 逻辑优先识别 semantic thought tail。

关键常量/标识：

```text
THINK_START
THOUGHT_START
MAX_SPECULATIVE_TAIL_AST_BYTES
"思考过程"
"元思考链"
```

### 4.5 高速上传 X-Upload-Token 服务端校验

文件：

```text
/root/VCPMobile/src-tauri/src/vcp_modules/infra/high_speed_channel.rs
```

问题：`prepare_vcp_upload` 返回 token，但上传 server 未校验 `X-Upload-Token` 请求头。任意本地 web 进程若猜到端口，可能在短窗口内向本地上传端口 POST 数据。

新增类型/函数：

```text
UploadRequestHeaders
parse_upload_request_headers
plain_http_response
```

请求处理新增逻辑：

- `OPTIONS` 预检返回 204，并允许 `X-Upload-Token, Content-Type`。
- 非 `POST` 返回 405。
- `X-Upload-Token` 不匹配返回 401。
- 请求头超过 `16 * 1024` 字节返回 431。
- 文件创建失败返回 500。
- 上传不完整返回 400。

前端确认：

```text
/root/VCPMobile/src/core/stores/attachmentStore.ts
```

其中大文件 XHR 已设置：

```text
xhr.setRequestHeader("X-Upload-Token", endpoint.token)
```

### 4.6 高速上传 finalization 与 no-overwrite 落盘

文件：

```text
/root/VCPMobile/src-tauri/src/vcp_modules/infra/high_speed_channel.rs
/root/VCPMobile/src-tauri/src/vcp_modules/infra/file_manager.rs
```

问题：附件按内容 hash 命名，多个并发路径可能写入相同目标。原先 `rename`、`copy`、`fs::write` 等路径存在覆盖/竞态风险。

修复：

- `finalize_high_speed_upload` 不再在目标存在时简单跳过并删除，而是统一使用 no-overwrite `safe_rename`。
- `dest_path.to_str().unwrap()` 改为 fallible `to_str().ok_or(...)`。
- `safe_rename` 采用 `hard_link` 或 `OpenOptions::create_new(true)` 语义。
- 目标已存在时删除临时源并返回 `Ok(())`。
- 复制失败或 sync 失败时清理目标。
- 新增 async helper `safe_move_no_overwrite_async`，同样使用 hard link 或 create_new 语义。

相关错误字符串：

```text
无效的目标路径字符
无效的缩略图目标路径字符
打开待移动文件失败: {}
创建目标文件失败: {}
复制文件到目标路径失败: {}
刷新目标文件失败: {}
```

### 4.7 本地文件注册路径与竞态修复

文件：

```text
/root/VCPMobile/src-tauri/src/vcp_modules/infra/file_manager.rs
```

修复点：

- 同 hash 目标已存在时删除源临时文件，避免覆盖。
- 不存在时使用 `safe_move_no_overwrite_async`。
- 缩略图源路径先经过 `ensure_safe_path` 校验。
- 缩略图目标路径 `to_str()` 改为 fallible。
- byte 写入路径从覆盖式写入改为 `OpenOptions::new().write(true).create_new(true)`，并 `sync_all()`。

重要函数/路径：

```text
register_local_file
register_attachment_internal
ensure_safe_path
get_attachments_root_dir
get_thumbnails_root_dir
safe_move_no_overwrite_async
```

### 4.8 前端动态 selector 安全

文件：

```text
/root/VCPMobile/src/core/composables/useChatScroll.ts
/root/VCPMobile/src/features/rag/RagObserver.vue
```

问题：动态值直接拼入 CSS selector，若 messageId 或 tab value 含特殊字符，可能破坏 selector 或造成异常。

`useChatScroll.ts` 修复：

- 新增 `findMessageElementById(list, messageId)`。
- 使用 `querySelectorAll("[data-message-id]")` 后比较 `el.dataset.messageId === messageId`。
- 不再拼接 `[data-message-id="..."]`。

`RagObserver.vue` 修复：

- 新增 `FilterValue` 类型。
- `activeFilter = ref<FilterValue>("all")`。
- watcher 使用 `querySelectorAll("[data-tab-value]")` 后比较 `el.dataset.tabValue === newVal`。

### 4.9 Floating Assistant WebSocket 生命周期修复

文件：

```text
/root/VCPMobile/src/core/stores/floatingAssistant.ts
```

问题：`initWebSocket` 可能重复创建 WebSocket，或旧 socket handler 在重连后继续改写状态。

修复：

- 已存在 `CONNECTING` 或 `OPEN` socket 时直接返回。
- 关闭旧 socket 前解绑 `onopen/onmessage/onerror/onclose`。
- 新 socket 的所有 handler 先检查 `ws.value === socket`。
- `onmessage` 对坏 JSON 做 try/catch，不再让异常冲击状态机。

### 4.10 chatStreamStore 引用计数修复

文件：

```text
/root/VCPMobile/src/core/stores/chatStreamStore.ts
```

问题：同一 owner/topic/messageId 在 thinking/data/aurora 等不同事件到达时，可能重复增加 `activeStreamRefCounts` 与 `activeStreamTotal`，造成屏幕常亮引用计数泄漏。

修复：

- `addSessionStream` 获取或创建 `sessionActiveStreams.value[key]`。
- 若 `streams.includes(messageId)`，只执行 `enforceStreamPoolLimit()` 后早退。
- 只有首次登记才增加引用计数、记录 context、调用 `acquireScreenKeep()`。

关键变量/函数：

```text
addSessionStream
sessionActiveStreams
activeStreamRefCounts
activeStreamContexts
activeStreamTotal
acquireScreenKeep
releaseScreenKeep
enforceStreamPoolLimit
```

### 4.11 Rust 测试 fixture fallback

文件：

```text
/root/VCPMobile/src-tauri/src/vcp_modules/chat/ast_diff.rs
/root/VCPMobile/src-tauri/src/vcp_modules/chat/stream_block_parser.rs
```

问题：`cargo test --all-targets --all-features` 曾因测试依赖缺失外部 fixture 或旧 Windows 绝对路径失败。

涉及路径：

```text
测试文档.txt
scripts/tail-test/测试文档.txt
../scripts/tail-test/测试文档.txt
g:\VCPMobile\scripts\tail-test\测试文档.txt
```

修复：

- `ast_diff.rs` 新增 `load_agent_stream_fixture()`。
- `stream_block_parser.rs` 新增 `load_tail_test_document()`。
- 两者均优先读取仓库内 fixture。
- fixture 缺失时使用内置确定性样本。
- `ast_diff.rs` fallback 样本规模从 `0..120` 增加到 `0..240`，以满足 mutation 数量断言。

曾出现的失败诊断：

```text
Total mutations count was too low: 35
```

最终报告：

```text
cargo test --all-targets --all-features
17 passed; 0 failed
```

## 5. 已完成变更：取消 25 秒 SSE idle timeout

文件：

```text
/root/VCPMobile/src-tauri/src/vcp_modules/infra/vcp_client.rs
```

用户要求：

```text
取消25秒超时，太离谱了。工具调用需要时间！
```

旧问题：流式响应 loop 中存在 25 秒 idle timeout。长时间工具调用期间如果服务端 25 秒没有吐 token/data，客户端会主动关闭连接并发送“连接超时”错误。

处理结果：

- 删除流式响应读取 loop 中的 idle sleep timeout 分支。
- 保留请求发出前的 `abort_rx` 中止处理。
- 保留流式读取期间的 `abort_rx` 中止处理。
- 恢复成功响应分支的 SSE line 初始化：`resp.bytes_stream()` → `StreamReader::new(...)` → `FramedRead::new(..., LinesCodec::new())`。
- `bytes_stream()` 错误映射使用 `IoError::other`，满足 clippy `io_other_error` 要求。
- 保留 `[DONE]` 正常 finalize、真实读取错误 `流读取错误`、无 `[DONE]` 但已有内容时按正常结束、无内容断开时发送 `网络连接意外断开` 的行为。

验证结果：

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
pnpm check
pnpm build
```

上述命令均已通过。`cargo test --all-targets --all-features` 结果为：

```text
17 passed; 0 failed
```

残留检查也已通过：

```bash
grep -RInE "Stream idle timeout|超过 25 秒|timeout_duration|last_activity|sleep_until|Duration::from_secs\\(25\\)" src-tauri/src/vcp_modules/infra/vcp_client.rs
```

该检查无结果，表示 25 秒 idle timeout 相关逻辑已从该文件移除。

## 6. 验证历史

本轮对话中使用过或报告过的验证命令：

```bash
pnpm check
pnpm exec vue-tsc --noEmit
pnpm exec vue-tsc --noEmit && pnpm build
pnpm build
pnpm check && pnpm build
cd src-tauri && cargo fmt --check
cd src-tauri && cargo check
cd src-tauri && cargo clippy --all-targets --all-features -- -D warnings
cd src-tauri && cargo fmt
cd src-tauri && cargo fmt && cargo fmt --check && cargo clippy --all-targets --all-features -- -D warnings
cd src-tauri && cargo test --all-targets --all-features
cd src-tauri && cargo test test_incomplete_think_tail_builds_thought_block -- --nocapture
cargo test --lib vcp_modules::chat::ast_diff::tests::test_real_agent_stream_simulation --all-features -- --nocapture
cargo test --lib vcp_modules::chat::ast_diff::tests::test_real_agent_stream_simulation --all-features
```

非阻塞构建警告：

```text
UnoCSS `virtual:uno.css` 被重复导入。
useRenderedImageViewer.ts 同时被动态和静态导入，导致无法拆 chunk。
部分 Vite chunk 超过 500KB。
```

这些警告未在本轮修复。

## 7. 已记录 notebook 笔记

已添加的关键笔记内容如下：

```text
新增统一的 renderSafeMarkdown/sanitizeMarkdownHtml，所有直接 marked.parse 后接 v-html 的非 AST 渲染路径应复用它，避免工具结果/RAG/助手消息中的 HTML 被未净化注入。
```

```text
StreamBlockParser::build_incomplete_tail_block 用于 finalize 未闭合 tail；未闭合 <think>/<thinking> 或 VCP 元思考链必须沉淀为 is_complete=false 的 thought 块，避免最终消息把思考标记当普通 Markdown 展示。
```

```text
Mermaid 全屏预览不应直接 v-html 原始 svgHtml；使用 DOMPurify 计算 sanitizedSvgHtml 后注入。renderLibraryPreloader 同步将 Mermaid securityLevel 设为 strict。
```

```text
prepare_vcp_upload 的高速上传 POST 必须校验 X-Upload-Token；前端 attachmentStore.ts 大文件 XHR 需设置同名 header。OPTIONS 预检允许 X-Upload-Token, Content-Type，非 POST 或 token 不匹配应返回错误。
```

```text
附件按内容 hash 落盘时必须使用 no-overwrite 语义：safe_rename/safe_move_no_overwrite_async 用 hard_link 或 create_new，目标已存在时删除临时源文件，避免并发同 hash 上传/注册覆盖已落盘文件。
```

```text
流式 AST diff 压测优先读取 scripts/tail-test/测试文档.txt；fixture 缺失时使用内置确定性长样本，避免 Linux/CI 因本地文件或旧 Windows 绝对路径缺失导致 cargo test 失败。
```

```text
addSessionStream 同一 owner/topic/messageId 只允许首次登记增加 activeStreamRefCounts 和 activeStreamTotal；thinking/data/aurora 多类事件重复到达时应早退，避免屏幕常亮引用计数泄漏。
```

```text
initWebSocket 需要避免重复 CONNECTING/OPEN socket，并让 onopen/onmessage/onerror/onclose 先校验 ws.value === socket；关闭旧 socket 前要解绑 handler，防止重连后的陈旧事件改写悬浮助手状态。
```

```text
滚动锚点不要把 messageId 拼进 CSS selector；使用 querySelectorAll('[data-message-id]') 后比较 el.dataset.messageId，可避免特殊字符破坏 selector。
```

尚未看到 timeout 修复对应 notebook note。

## 8. 重要命令与搜索模式

曾用于审查的代表性搜索模式：

```text
TODO|FIXME|unwrap\(|expect\(|panic!|eval\(|innerHTML|outerHTML|v-html|setInterval\(|addEventListener\(|fetch\(|invoke\(
marked\.parse|v-html|innerHTML|DOMPurify|sanitizeMarkdownHtml|sanitizeHighlightedCodeHtml
prepare_vcp_upload|X-Upload-Token|finalize_high_speed_upload|register_local_file|safeImageUrl|addSessionStream|initWebSocket|querySelector\(|querySelectorAll\(|CSS\.escape
tokio::fs::copy\(|tokio::fs::rename\(|dest_thumb_path\.to_str\(\)\.unwrap|dest_path\.to_str\(\)\.unwrap|safe_rename\(
25000|25_000|25\s*\*\s*1000|25s|25 秒|25秒|timeout|Timeout|setTimeout
Stream idle timeout|timeout_duration|超过 25 秒|Duration::from_secs\(25\)|read_timeout
```

曾用 fixture 查找命令：

```bash
find /root/VCPMobile -path '*/target' -prune -o \( -name '测试文档.txt' -o -path '*/scripts/tail-test*' \) -print
```

曾用单测诊断命令：

```bash
cargo test --lib vcp_modules::chat::ast_diff::tests::test_real_agent_stream_simulation --all-features -- --nocapture > /tmp/vcp_ast_diff_test.log 2>&1; status=$?; echo STATUS:$status; grep -nE 'panicked|Total mutations|assertion|FAILED|failures:' /tmp/vcp_ast_diff_test.log | tail -40; tail -60 /tmp/vcp_ast_diff_test.log
```

## 9. 已检查过的重要源码文件

前端：

```text
/root/VCPMobile/src/features/assistant/AssistantMessageCard.vue
/root/VCPMobile/src/features/assistant/AssistantView.vue
/root/VCPMobile/src/features/rag/RagPayloadDetail.vue
/root/VCPMobile/src/features/rag/RagObserver.vue
/root/VCPMobile/src/components/ui/UpdatePrompt.vue
/root/VCPMobile/src/components/ui/RenderedImageViewer.vue
/root/VCPMobile/src/features/chat/MessageRenderer.vue
/root/VCPMobile/src/features/chat/blocks/ToolBlock.vue
/root/VCPMobile/src/features/chat/blocks/ThoughtBlock.vue
/root/VCPMobile/src/features/chat/blocks/HtmlPreviewBlock.vue
/root/VCPMobile/src/features/chat/blocks/MermaidFullScreenViewer.vue
/root/VCPMobile/src/features/chat/attachment/AttachmentPreview.vue
/root/VCPMobile/src/features/chat/ChatView.vue
/root/VCPMobile/src/core/utils/astRenderer.ts
/root/VCPMobile/src/core/utils/astExecutor.ts
/root/VCPMobile/src/core/utils/renderedImage.ts
/root/VCPMobile/src/core/utils/renderLibraryPreloader.ts
/root/VCPMobile/src/core/utils/safeMarkdown.ts
/root/VCPMobile/src/core/composables/useMessageStyleInjector.ts
/root/VCPMobile/src/core/composables/useMessageEvents.ts
/root/VCPMobile/src/core/composables/useRenderedImageViewer.ts
/root/VCPMobile/src/core/composables/useDocumentProcessor.ts
/root/VCPMobile/src/core/composables/useChatScroll.ts
/root/VCPMobile/src/core/stores/chatHistoryStore.ts
/root/VCPMobile/src/core/stores/chatStreamStore.ts
/root/VCPMobile/src/core/stores/floatingAssistant.ts
/root/VCPMobile/src/core/stores/topicListManager.ts
/root/VCPMobile/src/core/stores/attachmentStore.ts
/root/VCPMobile/src/core/stores/syncSession.ts
/root/VCPMobile/src/core/stores/appLifecycle.ts
/root/VCPMobile/src/core/stores/assistant.ts
/root/VCPMobile/src/core/stores/chatSessionStore.ts
/root/VCPMobile/src/core/types/chat.ts
/root/VCPMobile/src/App.vue
```

Rust/Tauri：

```text
/root/VCPMobile/src-tauri/src/lib.rs
/root/VCPMobile/src-tauri/src/vcp_modules/chat/aurora_pipeline.rs
/root/VCPMobile/src-tauri/src/vcp_modules/chat/content_parser.rs
/root/VCPMobile/src-tauri/src/vcp_modules/chat/stream_block_parser.rs
/root/VCPMobile/src-tauri/src/vcp_modules/chat/ast_diff.rs
/root/VCPMobile/src-tauri/src/vcp_modules/chat/topic_summary_service.rs
/root/VCPMobile/src-tauri/src/vcp_modules/infra/local_server.rs
/root/VCPMobile/src-tauri/src/vcp_modules/infra/high_speed_channel.rs
/root/VCPMobile/src-tauri/src/vcp_modules/infra/file_manager.rs
/root/VCPMobile/src-tauri/src/vcp_modules/infra/file_extractor.rs
/root/VCPMobile/src-tauri/src/vcp_modules/infra/vcp_client.rs
/root/VCPMobile/src-tauri/src/vcp_modules/infra/utils.rs
/root/VCPMobile/src-tauri/src/vcp_modules/updater/frontend_update_manager.rs
/root/VCPMobile/src-tauri/src/vcp_modules/sync/sync_service.rs
/root/VCPMobile/src-tauri/src/vcp_modules/sync/sync_types.rs
/root/VCPMobile/src-tauri/src/distributed/tools/cpu_info.rs
```

注意：曾出现过一个可能错误的路径引用：

```text
/root/VCPMobile/src-tauri/src/vcp_modules/content_parser.rs
```

实际多次使用的路径是：

```text
/root/VCPMobile/src-tauri/src/vcp_modules/chat/content_parser.rs
```

## 10. 后续建议优先级

第一优先级：已完成 `/root/VCPMobile/src-tauri/src/vcp_modules/infra/vcp_client.rs` 的 25 秒 idle timeout 移除与验证；后续无需再把该项视为阻塞问题。

第二优先级：根据非阻塞构建警告做性能优化，包括 UnoCSS 重复导入、`useRenderedImageViewer.ts` 动静态混合导入、Vite 大 chunk 拆分。

第四优先级：如需发布 Android APK，不要本地打包，应按 `AGENTS.md` 要求通过 GitHub Actions release/workflow_dispatch。

## 11. 安全注意事项

- 不要对 `/root/VCPToolBox` 做任何修改，除非用户明确要求。
- 不要运行 Git rollback，除非用户明确同意。
- 不要运行本地 Android APK 打包命令，除非用户明确覆盖限制。
- 不要使用广泛 Node.js kill 命令；CLI 自身是 Node.js 进程。
- 后续修复 `vcp_client.rs` 时必须先完整读取函数边界，避免再次出现 fuzzy edit 越界或括号不匹配。
