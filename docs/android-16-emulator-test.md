# Android 16 Emulator Test Notes

记录时间：2026-05-02
项目：VCP Mobile / Project Avatar

## 目标

这份文档记录在本机创建 Android 16 完整系统镜像模拟器，并用它对 VCP Mobile 做一次启动、渲染、交互和基础性能测试的结果。

用户真机环境是小米 17 Pro Max，Android 16。当前模拟器不能完全替代真机测试，但可以覆盖 Android 16 API、WebView、Tauri Android shell 和基础兼容性问题。

## 本机环境

Android SDK：

```powershell
C:\Android\Sdk
```

Java：

```powershell
C:\Program Files\Eclipse Adoptium\jdk-17.0.18.8-hotspot
```

常用环境变量：

```powershell
$env:Path = [Environment]::GetEnvironmentVariable('Path','Machine') + ';' + [Environment]::GetEnvironmentVariable('Path','User')
$env:ANDROID_HOME = 'C:\Android\Sdk'
$env:ANDROID_SDK_ROOT = 'C:\Android\Sdk'
$env:JAVA_HOME = 'C:\Program Files\Eclipse Adoptium\jdk-17.0.18.8-hotspot'
```

Android 工具：

```powershell
C:\Android\Sdk\platform-tools\adb.exe
C:\Android\Sdk\emulator\emulator.exe
C:\Android\Sdk\cmdline-tools\latest\bin\sdkmanager.bat
```

已确认安装的关键包：

```text
emulator                                      36.5.11
platform-tools                               37.0.0
platforms;android-36                         2
system-images;android-36;google_apis;x86_64  7
```

## Android 16 系统镜像

使用的完整系统镜像：

```text
system-images;android-36;google_apis;x86_64
```

镜像来源：

```text
https://dl.google.com/android/repository/sys-img/google_apis/x86_64-36_r07.zip
```

SHA1：

```text
c6bf44bdcd885bb902b4ba752d111a073ad7a817
```

本机缓存：

```text
C:\Android\Sdk\.temp\google_apis_x86_64-36_r07.zip
```

解压位置：

```text
C:\Android\Sdk\system-images\android-36\google_apis\x86_64
```

备注：最初直接用 `sdkmanager` 安装时出现损坏/零字节下载问题，所以这次是手动下载、校验 SHA1、再解压到 SDK system image 目录。

## AVD

已创建的 Android 16 完整镜像 AVD：

```text
vcp_api36_full_x86_64
```

设备 profile：

```text
pixel_7
```

模拟器启动后确认：

```text
ro.build.version.sdk      36
ro.build.version.release  16
ro.product.model          sdk_gphone64_x86_64
ro.product.cpu.abi        x86_64
wm size                   1080x2400
wm density                420
```

本机当前还有一个旧的 API 35 ATD AVD：

```text
vcp_api35_atd_x86_64
```

API 35 ATD 曾经可以启动应用，但截图黑屏且 `gfxinfo` 没有有效帧，不适合作为这次 Android 16 视觉和性能验证的主环境。

## 启动模拟器

```powershell
& 'C:\Android\Sdk\emulator\emulator.exe' -avd vcp_api36_full_x86_64
```

如果需要干净启动：

```powershell
& 'C:\Android\Sdk\emulator\emulator.exe' -avd vcp_api36_full_x86_64 -no-snapshot -no-boot-anim
```

如果遇到渲染异常，可以尝试不同 GPU 后端：

```powershell
adb emu kill
& 'C:\Android\Sdk\emulator\emulator.exe' -avd vcp_api36_full_x86_64 -no-snapshot -no-boot-anim -gpu host
```

或：

```powershell
adb emu kill
& 'C:\Android\Sdk\emulator\emulator.exe' -avd vcp_api36_full_x86_64 -no-snapshot -no-boot-anim -gpu angle_indirect
```

## 构建 x86_64 Debug APK

Android 16 模拟器是 `x86_64` ABI。之前 arm64-only APK 会安装失败：

```text
INSTALL_FAILED_NO_MATCHING_ABIS
```

需要先安装 Rust Android x86_64 target：

```powershell
rustup target add x86_64-linux-android
```

构建 x86_64 debug APK：

```powershell
pnpm tauri android build --debug --target x86_64 --apk --ci
```

本次测试安装的 APK：

```text
D:\VCPMobile\src-tauri\gen\android\app\build\outputs\apk\universal\debug\app-universal-debug.apk
```

安装：

```powershell
adb install -r "D:\VCPMobile\src-tauri\gen\android\app\build\outputs\apk\universal\debug\app-universal-debug.apk"
```

启动：

```powershell
adb shell am start -W -n com.vcp.avatar/.MainActivity
```

## 本次测试结果

测试日期：2026-05-02
模拟器：`vcp_api36_full_x86_64`
设备：`emulator-5554`
应用：`com.vcp.avatar/.MainActivity`

启动结果：

```text
Status: ok
Activity: com.vcp.avatar/.MainActivity
TotalTime: 658ms
WaitTime: 660ms
```

前端生命周期日志显示应用完成启动：

```text
[Lifecycle] -> READY | 应用就绪
```

从启动命令到 READY 大约 4 秒。

交互烟测：

```powershell
adb shell input tap 540 1590
adb shell input swipe 540 1800 540 700 500
adb shell input keyevent BACK
```

交互后 Activity 状态：

```text
topResumedActivity = com.vcp.avatar/.MainActivity
ResumedActivity    = com.vcp.avatar/.MainActivity
mCurrentFocus      = com.vcp.avatar/com.vcp.avatar.MainActivity
```

截图正常，首屏可以渲染：

```text
D:\VCPMobile\.test-artifacts\emulator-api36-vcp-screenshot.png
D:\VCPMobile\.test-artifacts\emulator-api36-vcp-after-smoke.png
```

## 性能数据

交互后内存：

```text
TOTAL PSS       142653 KB
TOTAL RSS       312028 KB
TOTAL SWAP PSS      21 KB
WebViews             1
Activities           1
```

交互后图形性能：

```text
Total frames rendered       6937
Janky frames                  63
Janky frames                0.91%
50th percentile              28ms
90th percentile              34ms
95th percentile              36ms
99th percentile              46ms
50th gpu percentile          20ms
90th gpu percentile          23ms
95th gpu percentile          23ms
99th gpu percentile          25ms
Pipeline                     Skia (OpenGL)
```

结论：在 Android 16 x86_64 模拟器上应用可以正常启动、渲染和保持前台，没有发现崩溃或 ANR。帧耗时在模拟器上偏高，P50 约 28ms，P90 约 34ms。这个结果需要和小米 17 Pro Max 真机的 arm64 数据对比后再决定是否做专项性能优化。

## 日志发现

未发现以下严重错误：

```text
AndroidRuntime crash
FATAL EXCEPTION
Fatal signal
ANR
```

已知配置问题：

```text
Failed to fetch models: VCP Server URL is not configured.
```

这说明模型列表请求依赖的 VCP Server URL 当前没有配置。它会造成模型拉取失败，但本次没有导致应用崩溃。

WebView/Chromium 警告：

```text
WARNING: tile memory limits exceeded, some content may not draw
```

目前截图和前台状态都正常，没有观察到白屏或黑屏。但这个警告说明首屏图片或 WebView tile 绘制压力偏高，后续如果真机也出现卡顿、局部不绘制或内存压力，需要优先检查首屏大图尺寸、缓存策略和渲染层数量。

首次 WebView cache 日志：

```text
Failed reading seed file
Could not reconstruct index from disk
```

这些通常是首次启动 WebView cache 不存在导致的日志，本次未观察到功能影响。

## 常用回归命令

设备检查：

```powershell
adb devices -l
adb shell getprop ro.build.version.sdk
adb shell getprop ro.build.version.release
adb shell getprop ro.product.model
adb shell getprop ro.product.cpu.abi
adb shell wm size
adb shell wm density
```

启动并采集冷启动时间：

```powershell
adb shell am force-stop com.vcp.avatar
adb shell am start -W -n com.vcp.avatar/.MainActivity
```

采集内存：

```powershell
adb shell dumpsys meminfo com.vcp.avatar
```

采集帧数据：

```powershell
adb shell dumpsys gfxinfo com.vcp.avatar
```

筛选关键日志：

```powershell
adb logcat -d -v time | Select-String 'AndroidRuntime|FATAL EXCEPTION|Fatal signal|ANR|com\.vcp\.avatar|Tauri/Console|chromium|VCP Server URL'
```

截图：

```powershell
$dir = 'D:\VCPMobile\.test-artifacts'
New-Item -ItemType Directory -Force -Path $dir | Out-Null
$out = Join-Path $dir 'emulator-api36-vcp-screenshot.png'
cmd /c "adb exec-out screencap -p > `"$out`""
```

## 真机测试备注

Android 16 模拟器是 `x86_64`，小米 17 Pro Max 真机应使用 `arm64-v8a`。模拟器结果只能代表 Android 16 API/WebView 兼容性，不能代表小米真机的最终性能、权限策略、后台策略、通知策略和厂商系统行为。

真机连接后建议执行：

```powershell
adb devices -l
adb install -r "D:\VCPMobile\src-tauri\gen\android\app\build\outputs\apk\universal\release\app-universal-release.apk"
adb shell am force-stop com.vcp.avatar
adb shell am start -W -n com.vcp.avatar/.MainActivity
adb shell dumpsys meminfo com.vcp.avatar
adb shell dumpsys gfxinfo com.vcp.avatar
adb logcat -d -v time | Select-String 'AndroidRuntime|FATAL|Exception|ANR|com\.vcp\.avatar|tauri|vcp'
```

真机重点验证：

- arm64 release APK 是否正常安装和启动。
- 首屏是否有局部不绘制、白屏、黑屏。
- 点击、滑动、返回、输入法弹出和收起是否稳定。
- 小米系统后台限制、权限弹窗、通知权限和省电策略是否影响核心服务。
- 与 VCP Server 配置后的模型拉取、对话和同步流程。
