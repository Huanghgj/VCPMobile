# VCPMobile Runtime Diagnostic Test API

## 目的

移动端安装到模拟器或真机后，电脑侧不能直接调用前端 store，也不能伪造真实 VCP 流式回包。这个诊断接口用于从应用外部注入一条“已完成的 AI 回复”，复用真实消息持久化和渲染链路，验证：

- AI 回复是否立即出现在当前话题。
- 工具调用和工具结果预览是否附加在同一条 AI 回复中。
- Rust AST 预编译、SQLite 持久化、刷新/重启后的加载是否一致。
- Android 模拟器上 WebView 图片/Markdown/工具块渲染是否正常。

## 应用内入口

打开任意聊天话题后进入：

```text
设置 -> 诊断 -> Runtime Test API
```

按钮：

- `Inject Reply`：直接向当前话题注入一条带工具请求和工具结果的 AI 示例回复。
- `Start API`：启动本地 HTTP 诊断服务，默认绑定应用所在设备的 `127.0.0.1`，并生成一次性 token。
- `Stop API`：关闭诊断服务。

诊断服务不会随应用自动启动。服务启动后页面会显示设备侧端口、token 和一段可直接在电脑上执行的 `adb forward` / `curl.exe` 示例。

## 电脑侧注入流程

1. 在模拟器/真机中打开目标聊天话题。
2. 进入诊断页，点击 `Start API`。
3. 在电脑 PowerShell 执行端口转发，将电脑端口转到应用显示的设备端口：

```powershell
adb forward tcp:5897 tcp:<APP_PORT>
```

4. 调用注入接口：

```powershell
curl.exe -X POST "http://127.0.0.1:5897/inject/assistant?token=<TOKEN>" `
  -H "Content-Type: application/json" `
  --data '{"ownerId":"<AGENT_ID>","ownerType":"agent","topicId":"<TOPIC_ID>","name":"Diagnostic AI","content":"hello from host"}'
```

`ownerId`、`ownerType`、`topicId` 可以直接从诊断页的 `Target` 行读取。群聊不是当前重点，常规测试使用 `ownerType: "agent"`。

## 端点

```text
GET  /status?token=<TOKEN>
GET  /sample/assistant?token=<TOKEN>
POST /inject/assistant?token=<TOKEN>
```

`POST /inject/assistant` 请求体：

```json
{
  "ownerId": "agent_xxx",
  "ownerType": "agent",
  "topicId": "topic_xxx",
  "content": "AI reply markdown",
  "name": "Diagnostic AI",
  "messageId": "optional_stable_id",
  "agentId": "optional_agent_id",
  "timestamp": 1770000000000
}
```

只有 `ownerId`、`ownerType`、`topicId`、`content` 必填。未传 `messageId` 时后端会生成 `diag_ai_<timestamp>_<suffix>`。

## 当前实现边界

- 这是诊断入口，不是业务 API。它需要手动启动并携带 token。
- 默认只绑定设备本机回环地址；电脑访问模拟器/真机时使用 `adb forward`。
- 注入的是“完成态 AI 回复”，不模拟逐 token 流式回包。它覆盖渲染、持久化、刷新验证；若要测动画级流式行为，后续可再加 `POST /stream/start`、`POST /stream/chunk`、`POST /stream/end`。
- 注入会真实写入当前应用数据库。测试结束后可在聊天里手动删除对应消息。
