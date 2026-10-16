# VCPMobile iOS 兼容适配专项研究

> 修订：2026-10-09。最初研究基线：`4c6e25bb`；本轮 Phase 3 从 `8207ec70` 实施，VCPMobile `1.1.6`。历史执行结果保留其原基线。
>
> 目标：同一 Vue/Rust 核心的 iPhone/iPad 实验性客户端；开发采用源码与编译语义推演，产物供用户自行签名。

## 分期实施工作区（2026-10-08 启动）

2026-10-16：用户允许每阶段按需进行手动 GitHub Actions 诊断，已将现有 G1 入口扩展为 [iOS stage diagnostics](../../src-tauri/ios/README.md)，默认 Phase 3 / debug；G1/G2 的验收职责保留。本次准备入口不代表已取得新的 Apple 编译结果。

2026-10-09：沿既有 `plugin-os` 同步身份与 Vue 运行时分支进入 Phase 3，接入共同通知权限、本 App 本地通知/冷启动点击、设备/传感器、剪贴板与 iOS Distributed，并处理 Chat 输出接管、Sync/LAN 生命周期。具体源码结果和本地验证见阶段三记录。G1 Apple 构建仍未执行，不以源码或 host 检查宣称原生编译/运行已通过。

本目录 00–09 是后续施工使用的专题研究，按 [Phase 2 功能审批](./phase-2/09-功能权限与提示审议表.md) 更新范围与任务。每个 Phase 保存对应施工和证据；Phase 0/1 的完成记录属于各自基线，Phase 2 源码接入和平台简化已有记录、G1 待执行。Phase 3 授权源码链已接入、复审及 Vue/host 检查通过，领域结果见阶段记录；Phase 4–6 仍待实施。本轮不扩展为全部原生媒体/分享或 SSE 后台功能。

| 工作区 | 当前职责 |
|---|---|
| [Phase 0](./phase-0/README.md) | 已完成输入与目标源码审计；具体证据和后续问题见阶段验收记录 |
| [Phase 1](./phase-1/README.md) | 保存 cfg/ACL、目录与生命周期契约的阶段证据；当时的统一能力表已被本轮轻量平台身份替代 |
| [Phase 2](./phase-2/README.md) | Swift/Channel/生命周期与可重放工程已接入；复审、修正及平台简化见阶段记录，G1 Apple 构建待执行 |
| [Phase 3](./phase-3/README.md) | 本轮 Chat/Sync/LAN、共同通知权限、本地通知与设备/网络/Distributed 实施、独立复审和验证记录 |
| [Phase 4](./phase-4/README.md) | 原生文件媒体、拍照/相册、双向分享、WebKit 缓存与交互 |
| [Phase 5](./phase-5/README.md) | SSE 后台持续接收专项、其余尽力后台与单次终态 |
| [Phase 6](./phase-6/README.md) | G2 集中验收与用户自签产物 |

Phase 0 已同步 [移动端 UI 规范](../../docs/MOBILE_UI_COMPATIBILITY.md)；2026-10-09 用户明确将最低 iOS/iPadOS 提高到 26.0，`tauri.ios.conf.json` 与 Swift package 同步采用该目标，编译核验仍待 G1。原 15.0 是 Phase 0–2 历史基线。[跨平台开发规范](../../docs/CROSS_PLATFORM_DEVELOPMENT.md) 现规定共享 Vue 中直接使用运行时平台分支，不再要求统一能力状态表或逐层复制功能门禁。

## 研究目标与开发方式

本系列给出 iOS 适配的可施工方案。开发与验收固定采用 Windows 源码审计和阶段性 GitHub Actions；项目现在及后续均不建设 iOS 实机测试体系。研究从锁定依赖源码、框架模板、API 声明和语言规则恢复目标平台行为，把类型、注册、链接、生命周期与文件归属逐项推导到具体代码。

```text
锁定源码 / API 声明 / 工程模板
        → 目标 cfg、类型、FFI、链接与状态推演
        → 平台适配、Swift 插件、核心旅程与后台专项
        → 阶段性手动 GitHub Actions 构建、链接与设备架构打包
        → 实验性 .app.zip / .unsigned.ipa，用户自行签名
```

实现按模块依赖和源码审计结果推进；Apple 构建集中在插件接入 G1 与首版集成/打包 G2 两个验收点，具体安排见 07/08。研究中每个结论都附源码位置或 API 来源，每个待办都写明缺失信息、获取入口和影响的具体路径。环境与检查结果集中记录在 [00](./00-研究契约与证据账本.md)。

### 本次修订保留与纠正的内容

| 主题 | 当前处理 |
|---|---|
| 插件命令 | 自有插件当前 45 条；原 42 条的 `R3 / S7 / L15 / X17` 分类仅保留历史索引，另列本轮 3 条通知授权/设置命令 |
| 生命周期 | Swift producer、Rust subscribe 和 bootstrap 已接通；原 Android-only 订阅问题属于已修正的历史发现 |
| 平台身份 | `src/core/utils/platform.ts` 同步导出 `nativePlatform/isAndroid/isIOS`；已删除 Rust/前端统一能力表与 Swift 能力查询 |
| 后台专项 | SSE 持续接收是最新重点要求；其余任务尽力后台。静音音频等候选另行研究，不把 API 存在当作效果已满足 |
| 用户自签产物 | 恢复 D8；检查 device Mach-O、包结构、最终 Bundle ID 与 entitlement 配套，见 06 |
| 前端现状 | 保留 PermissionGate 已挂载、Router 仅 `/ → /chat`、overlay/page stack 承载业务页的修正 |
| 决策归属 | D1–D19 沿革保留；2026-10-09 逐项审批优先于旧延期/隐藏意见。源码审计与 G1/G2 方式不变 |

### 首个实验产物

按 D15 及最新功能审批组织目标，具体依赖顺序见 08：

- iPhone/iPad 共用 Chat、Diary、Settings、SQLite 与手动 Sync；
- 原生 Files/PHPicker、security-scoped staging、大图/HEIC 处理与附件 CAS；
- safe area、键盘、返回、文本选择与当前窗口宽度响应；
- SSE 后台持续接收专项，以及 iOS 26+ continued-task 等候选与前台常亮回退；其余任务接受尽力后台，尚无已完成的 iOS 后台 adapter；
- 独立的 iOS 构建与打包路径，保留 Android arm64 正式链路。

最新审批已纳入 incoming/outgoing share、本 App 系统通知、相机、PhotoKit 保存、网络遥测与 Distributed，以及电量、空间、常亮、缓存和共同权限引导。当前 P3 已有通知/设备/工具源码；P4 文件媒体/分享/缓存与 P5 常亮/后台资源仍待实现。系统通知只接本 App 本地通知，不自动等同 APNs。语音、Root/APK 和 Android CLI 的既定边界保持；minimum 26.0 以上新增 API 仍核对 availability，系统授权、资源获得和任务激活仍取真实结果。每项完成度按实际源码和对应证据记录，不从批准范围推定完成。

### 关键施工任务

1. 从锁文件逐依赖审查 Apple cfg、build script、原生源码与链接符号，形成 01 的依赖账本。
2. 使用已接入的同步平台身份，在现有 Vue owner 内完成显示、说明、自动启动和监听分支；权限与任务状态继续读取实际操作结果。
3. 保持已拆开的 Android/iOS/host cfg；后续原生实现替换对应 iOS 分支，不借用 host 模拟成功。
4. 沿已接通的 `ios_path`、Swift Package、binding 和 Channel 完成 G1 编译核验；事件订阅接收与取消语义保持。
5. 按依赖完成前台核心、原生附件、键盘、共同权限、分享/通知与设备工具，再闭合后台专项和终态。
6. 随具体实现更新产品规范、实际所需命令授权和工程输入；当前源码、本地检查与 Apple 结果分别记录。

### 原 42 条命令的迁移基线

| 类别 | 数量 | iOS 动作 |
|---|---:|---|
| `R` | 3 | 复用 Rust/跨平台实现，明确 Apple container 路径与实际副作用 |
| `S` | 7 | 需要 Swift，以公开 Apple API 实现接近语义 |
| `L` | 15 | 必须改 DTO、生命周期或交互承诺后才能实现 |
| `X` | 17 | 当前命令机制属于 Android；iOS 是否保留说明入口按最新审批，不能由此推导统一隐藏 |
| **合计** | **42** | 文件名中的“41”仅为历史链接兼容；分类是迁移索引，不是运行时能力表 |

Phase 3 另新增 `get_notification_permission`、`request_notification_permission`、`open_notification_settings`，当前公开面共 45 条；不重新编造分类矩阵。详见 [02-平台能力与41条命令兼容矩阵.md](./02-平台能力与41条命令兼容矩阵.md)。产品范围由最新逐项审批与主线 09 决策沿革确定，当前执行情况由实际代码确定。

### 阅读与完成标准

实现结论使用“源码/声明 → 条件推演 → 改动位置 → 检查方法”的结构。日常逐条记录入口覆盖、分支条件、缺陷和修复回扫；阶段验收记录 GitHub Actions 的输入、诊断与产物。完成度以具体路径和问题是否闭环衡量，性能沿用 D16 的静态预算范围。

## 文档导航

| 文档 | 当前职责 |
|---|---|
| [00-研究契约与证据账本.md](./00-研究契约与证据账本.md) | 当前基线、源码推演方法、检查记录与来源 |
| [01-架构现状与跨平台改造边界.md](./01-架构现状与跨平台改造边界.md) | Rust/Tauri 可复用面、平台 owner 与 cfg 边界 |
| [02-平台能力与41条命令兼容矩阵.md](./02-平台能力与41条命令兼容矩阵.md) | 原 42 条迁移索引与新增 3 条通知命令、内部原生面、同步平台身份及实际 ACL |
| [03-WebKit与iPhone-iPad交互适配.md](./03-WebKit与iPhone-iPad交互适配.md) | WKWebView、键盘、安全区、手势、可访问性 |
| [04-生命周期后台网络与同步策略.md](./04-生命周期后台网络与同步策略.md) | 生命周期、尽力后台、Chat/Sync、LAN/ATS |
| [05-文件媒体分享通知与设备能力.md](./05-文件媒体分享通知与设备能力.md) | staging、媒体、分享、通知与设备事实 |
| [06-安全隐私签名与App-Store发布.md](./06-安全隐私签名与App-Store发布.md) | Tauri ACL、隐私清单、用户自签产物与打包 |
| [07-CI测试矩阵与无设备验证边界.md](./07-CI测试矩阵与无设备验证边界.md) | 日常源码审计、缺陷闭环与阶段性 GitHub Actions 验收 |
| [08-分期实施路线图与验收门禁.md](./08-分期实施路线图与验收门禁.md) | 实施顺序、模块依赖与完成条件 |
| [09-Magi综合裁决与待决策清单.md](./09-Magi综合裁决与待决策清单.md) | 三方审查、D1–D19 决策沿革与具体待办 |
