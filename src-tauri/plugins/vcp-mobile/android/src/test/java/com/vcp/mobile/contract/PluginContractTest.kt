package com.vcp.mobile.contract

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

class PluginContractTest {
    private val pluginRoot = findPluginRoot()

    private fun findPluginRoot(): File {
        var dir = File(System.getProperty("user.dir") ?: ".").canonicalFile
        repeat(8) {
            val direct = File(dir, "src-tauri/plugins/vcp-mobile")
            if (File(direct, "src/lib.rs").exists()) {
                return direct
            }

            val androidModule = File(dir, "src/lib.rs")
            if (androidModule.exists() && File(dir, "guest-js/index.ts").exists()) {
                return dir
            }

            dir = dir.parentFile ?: return@repeat
        }
        error("无法定位 tauri-plugin-vcp-mobile 根目录，user.dir=${System.getProperty("user.dir")}")
    }

    @Test
    fun defaultPermissionContainsAllRegisteredPluginCommands() {
        val libRs = File(pluginRoot, "src/lib.rs").readText()
        val defaultToml = File(pluginRoot, "permissions/default.toml").readText()

        val registeredCommands = Regex("(?:screen|stream|system)::([a-zA-Z0-9_]+)")
            .findAll(libRs)
            .map { it.groupValues[1] }
            .toSet()

        val missing = registeredCommands.filter { command ->
            !defaultToml.contains("\"$command\"")
        }

        assertTrue("default.toml 缺少插件命令授权: $missing", missing.isEmpty())
    }

    @Test
    fun guestJsStopStreamServicePassesAgentNameArgument() {
        val guestJs = File(pluginRoot, "guest-js/index.ts").readText()

        assertTrue(
            "stopStreamService 应接收 agentName 参数",
            Regex("function\\s+stopStreamService\\s*\\(\\s*agentName\\s*:\\s*string\\s*\\)").containsMatchIn(guestJs),
        )
        assertTrue(
            "stopStreamService invoke 应传递 { agentName }",
            guestJs.contains("plugin:vcp-mobile|stop_streaming_service") && guestJs.contains("{ agentName }"),
        )
    }

    @Test
    fun rustRunMobilePluginMethodNamesExistInKotlinPlugin() {
        val rustSources = listOf("src/system.rs", "src/stream.rs")
            .map { File(pluginRoot, it).readText() }
            .joinToString("\n")
        val kotlinPlugin = File(
            pluginRoot,
            "android/src/main/java/com/vcp/mobile/VcpMobilePlugin.kt",
        ).readText()

        val methodNames = Regex("run_mobile_plugin(?:::<[^>]+>)?\\(\\s*\"([A-Za-z0-9_]+)\"")
            .findAll(rustSources)
            .map { it.groupValues[1] }
            .toSet()

        val missing = methodNames.filter { method ->
            !Regex("fun\\s+$method\\s*\\(").containsMatchIn(kotlinPlugin)
        }

        assertTrue("Rust run_mobile_plugin 方法在 Kotlin 中不存在: $missing", missing.isEmpty())
    }

    @Test
    fun galleryPickerSupportsMultipleResultsAsOneBatch() {
        val kotlinPlugin = File(
            pluginRoot,
            "android/src/main/java/com/vcp/mobile/VcpMobilePlugin.kt",
        ).readText()
        val rustSystem = File(pluginRoot, "src/system.rs").readText()

        assertTrue(kotlinPlugin.contains("MediaStore.ACTION_PICK_IMAGES"))
        assertTrue(kotlinPlugin.contains("MediaStore.EXTRA_PICK_IMAGES_MAX"))
        assertTrue(kotlinPlugin.contains("return Intent(MediaStore.ACTION_PICK_IMAGES).apply"))
        assertFalse(kotlinPlugin.contains("firstAvailablePickerIntent(photoPicker"))
        assertTrue(kotlinPlugin.contains("Intent(Intent.ACTION_OPEN_DOCUMENT)"))
        assertTrue(kotlinPlugin.contains("Intent(Intent.ACTION_GET_CONTENT)"))
        assertTrue(kotlinPlugin.contains("putExtra(Intent.EXTRA_ALLOW_MULTIPLE, true)"))
        assertTrue(kotlinPlugin.contains("addCategory(Intent.CATEGORY_OPENABLE)"))
        assertTrue(kotlinPlugin.contains("clipData.itemCount"))
        assertTrue(kotlinPlugin.contains("vcp-mobile-files-picked"))
        assertTrue(kotlinPlugin.contains("vcp-mobile-file-picker-dismissed"))
        assertTrue(rustSystem.contains("PickedFileBatch"))
        assertTrue(rustSystem.contains("NativePickFileResult"))
        assertTrue(kotlinPlugin.contains("MAX_PICKED_FILE_BYTES"))
        assertTrue(kotlinPlugin.contains("MAX_PICKED_BATCH_BYTES"))
        assertTrue(kotlinPlugin.contains("MAX_PICKED_FILES"))
        assertTrue(kotlinPlugin.contains("commitPickedTempFile"))
        assertTrue(kotlinPlugin.contains("requestId"))
        assertTrue(kotlinPlugin.contains("java.util.UUID.randomUUID()"))
        assertTrue(kotlinPlugin.contains("\"\${nativeId}_\$hash\$fileExtension\""))
        assertTrue(rustSystem.contains("request_id: Option<String>"))
    }

    @Test
    fun rootOomGuardCannotBlockNativeFileIoQueue() {
        val kotlinPlugin = File(
            pluginRoot,
            "android/src/main/java/com/vcp/mobile/VcpMobilePlugin.kt",
        ).readText()

        assertTrue(kotlinPlugin.contains("private val fileIoExecutor"))
        assertTrue(kotlinPlugin.contains("private val oomScoreExecutor"))
        assertTrue(
            Regex("private fun startOomScoreGuard\\(\\)\\s*\\{\\s*oomScoreExecutor\\.execute")
                .containsMatchIn(kotlinPlugin),
        )
    }

    @Test
    fun externalShareUsesBoundedBackgroundCopiesAndUniqueMovablePaths() {
        val kotlinPlugin = File(
            pluginRoot,
            "android/src/main/java/com/vcp/mobile/VcpMobilePlugin.kt",
        ).readText()
        val shareHandler = File(
            pluginRoot,
            "android/src/main/java/com/vcp/mobile/ShareIntentHandler.kt",
        ).readText()
        val rustSystem = File(pluginRoot, "src/system.rs").readText()

        assertTrue(shareHandler.contains("plugin.executeFileIo"))
        assertTrue(shareHandler.contains("MAX_SHARED_FILE_BYTES"))
        assertTrue(shareHandler.contains("MAX_SHARED_BATCH_BYTES"))
        assertTrue(shareHandler.contains("MAX_SHARED_FILES"))
        assertTrue(shareHandler.contains("intent.clipData"))
        assertTrue(shareHandler.contains("root.put(\"errors\", errors)"))
        assertTrue(shareHandler.contains("java.util.UUID.randomUUID()"))
        assertTrue(shareHandler.contains("sanitizeFileName"))
        assertTrue(shareHandler.contains("pendingShareCacheFiles.forEach(::deleteQuietly)"))
        assertTrue(kotlinPlugin.contains("Shared file path is outside the app share cache"))
        assertTrue(kotlinPlugin.contains("shared_\${nativeId}_\${hash}"))
        assertTrue(kotlinPlugin.contains("requireSupportedPickedFileSize(size, originalName)"))
        assertTrue(rustSystem.contains("canonical_path.starts_with(&canonical_cache)"))
        assertTrue(rustSystem.contains("Failed to process shared file"))
    }

    @Test
    fun coldStartShareWaitsForFrontendListenerReadiness() {
        val kotlinPlugin = File(
            pluginRoot,
            "android/src/main/java/com/vcp/mobile/VcpMobilePlugin.kt",
        ).readText()
        val shareHandler = File(
            pluginRoot,
            "android/src/main/java/com/vcp/mobile/ShareIntentHandler.kt",
        ).readText()
        val appVue = File(pluginRoot, "../../../src/App.vue").canonicalFile.readText()

        assertTrue(kotlinPlugin.contains("fun markShareIntentReady"))
        assertTrue(kotlinPlugin.contains("shareIntentHandler.onWebViewLoaded()"))
        assertTrue(!kotlinPlugin.contains("shareIntentHandler.injectShareData(webView)"))
        assertTrue(shareHandler.contains("frontendReady"))
        assertTrue(shareHandler.contains("consumeShareIntent(intent)"))
        assertTrue(appVue.contains("plugin:vcp-mobile|mark_share_intent_ready"))
        assertTrue(
            appVue.indexOf("window.addEventListener(\"vcp-share-intent\"") <
                appVue.indexOf("plugin:vcp-mobile|mark_share_intent_ready"),
        )
    }

    private fun String.countOccurrences(needle: String): Int =
        windowed(needle.length).count { it == needle }

    @Test
    fun nativeLifecycleEventIsWiredIntoTheRustApplication() {
        val appLib = File(pluginRoot, "../../src/lib.rs").canonicalFile.readText()
        val infraMod = File(pluginRoot, "../../src/vcp_modules/infra/mod.rs").canonicalFile.readText()

        assertTrue(appLib.contains("vcp-mobile://lifecycle"))
        assertTrue(appLib.contains("set_app_foreground_state_internal"))
        assertTrue(infraMod.contains("pub mod lifecycle_controller;"))
        assertTrue(infraMod.contains("pub mod lifecycle_state;"))
    }

    @Test
    fun notificationListenerServiceIsDeclaredInThePluginManifest() {
        val manifest = File(pluginRoot, "android/src/main/AndroidManifest.xml").readText()

        assertTrue(manifest.contains(".service.VcpNotificationListenerService"))
        assertTrue(manifest.contains("android.permission.BIND_NOTIFICATION_LISTENER_SERVICE"))
        assertTrue(manifest.contains("android.service.notification.NotificationListenerService"))
    }

    @Test
    fun appFileProviderDoesNotExposeTheSharedExternalStorageRoot() {
        val paths = File(
            pluginRoot,
            "../../../src-tauri/gen/android/app/src/main/res/xml/file_paths.xml",
        ).canonicalFile.readText()

        assertTrue(!paths.contains("<external-path"))
        assertTrue(paths.contains("<files-path"))
        assertTrue(paths.contains("<cache-path"))
        assertTrue(paths.contains("<external-files-path"))
        assertTrue(paths.contains("<external-cache-path"))
    }

    @Test
    fun modernAndroidUsesSafWithoutBroadMediaPermission() {
        val plugin = File(
            pluginRoot,
            "android/src/main/java/com/vcp/mobile/VcpMobilePlugin.kt",
        ).readText()
        val pluginManifest = File(pluginRoot, "android/src/main/AndroidManifest.xml").readText()
        val appManifest = File(
            pluginRoot,
            "../../../src-tauri/gen/android/app/src/main/AndroidManifest.xml",
        ).canonicalFile.readText()

        assertTrue(plugin.contains("Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q"))
        assertTrue(plugin.contains("ACTION_OPEN_DOCUMENT"))
        assertTrue(!plugin.contains("alias = \"storage\""))
        assertTrue(!pluginManifest.contains("READ_MEDIA_IMAGES"))
        assertTrue(pluginManifest.contains("READ_EXTERNAL_STORAGE\" android:maxSdkVersion=\"28\""))
        assertTrue(pluginManifest.contains("WRITE_EXTERNAL_STORAGE\" android:maxSdkVersion=\"28\""))
        assertTrue(!appManifest.contains("READ_EXTERNAL_STORAGE"))
        assertTrue(!appManifest.contains("WRITE_EXTERNAL_STORAGE"))
    }

    @Test
    fun batteryOptimizationFlowUsesUserVisibleSettingsWithoutRestrictedPermission() {
        val plugin = File(
            pluginRoot,
            "android/src/main/java/com/vcp/mobile/VcpMobilePlugin.kt",
        ).readText()
        val pluginManifest = File(pluginRoot, "android/src/main/AndroidManifest.xml").readText()

        assertTrue(!plugin.contains("ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS"))
        assertTrue(plugin.contains("ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS"))
        assertTrue(!pluginManifest.contains("REQUEST_IGNORE_BATTERY_OPTIMIZATIONS"))
    }

    @Test
    fun galleryImageReadsAreBoundedAcrossAllSources() {
        val plugin = File(
            pluginRoot,
            "android/src/main/java/com/vcp/mobile/VcpMobilePlugin.kt",
        ).readText()

        assertTrue(plugin.contains("MAX_GALLERY_IMAGE_BYTES"))
        assertTrue(plugin.contains("MAX_DATA_URL_CHARS"))
        assertTrue(plugin.contains("connection.contentLengthLong > MAX_GALLERY_IMAGE_BYTES"))
        assertTrue(plugin.contains("file.inputStream().use { readBytesLimited(it) }"))
        assertTrue(plugin.contains("bytes.size > MAX_GALLERY_IMAGE_BYTES"))
    }

    @Test
    fun wakeLocksUseRenewableFiniteLeases() {
        val guardian = File(
            pluginRoot,
            "android/src/main/java/com/vcp/mobile/service/ForegroundGuardian.kt",
        ).readText()
        val proxy = File(
            pluginRoot,
            "android/src/main/java/com/vcp/mobile/service/SseProxyService.kt",
        ).readText()

        for (source in listOf(guardian, proxy)) {
            assertTrue(source.contains("WAKE_LOCK_LEASE_MS"))
            assertTrue(source.contains("WAKE_LOCK_RENEWAL_MS"))
            assertTrue(source.contains("acquire(WAKE_LOCK_LEASE_MS)"))
            assertTrue(source.contains("removeCallbacks(wakeLockRenewal)"))
        }
    }

    @Test
    fun sseKeepaliveNeverOpensAnAudioPlaybackSession() {
        val proxy = File(
            pluginRoot,
            "android/src/main/java/com/vcp/mobile/service/SseProxyService.kt",
        ).readText()
        val plugin = File(
            pluginRoot,
            "android/src/main/java/com/vcp/mobile/VcpMobilePlugin.kt",
        ).readText()
        val appManifest = File(
            pluginRoot,
            "../../../src-tauri/gen/android/app/src/main/AndroidManifest.xml",
        ).canonicalFile.readText()

        assertFalse(proxy.contains("MediaPlayer"))
        assertFalse(proxy.contains("silent.wav"))
        assertFalse(proxy.contains("startSilentPlayback"))
        assertFalse(appManifest.contains("MODIFY_AUDIO_SETTINGS"))
        val initBlock = Regex("init\\s*\\{([\\s\\S]*?)\\n\\s*}").find(plugin)?.groupValues?.get(1).orEmpty()
        assertFalse(initBlock.contains("startHelperServiceInternal"))
    }

    @Test
    fun mainWebViewIsCreatedOnlyAfterDatabaseStateIsManaged() {
        val appLib = File(pluginRoot, "../../src/lib.rs").canonicalFile.readText()
        val tauriConfig = File(pluginRoot, "../../tauri.conf.json").canonicalFile.readText()

        assertTrue(Regex("\\\"label\\\"\\s*:\\s*\\\"main\\\"[\\s\\S]*?\\\"create\\\"\\s*:\\s*false").containsMatchIn(tauriConfig))
        val manageIndex = appLib.indexOf("app.manage(DbState")
        val createIndex = appLib.indexOf("WebviewWindowBuilder::from_config")
        assertTrue("DbState must be managed before the main WebView is created", manageIndex >= 0)
        assertTrue("main WebView creation is missing", createIndex >= 0)
        assertTrue("main WebView was created before DbState", manageIndex < createIndex)
    }

    @Test
    fun floatingWebViewCannotNavigateTheJavascriptBridgeOffLocalhost() {
        val manager = File(
            pluginRoot,
            "android/src/main/java/com/vcp/mobile/FloatingWindowManager.kt",
        ).readText()

        assertTrue(manager.contains("allowFileAccess = false"))
        assertTrue(manager.contains("allowContentAccess = false"))
        assertTrue(manager.contains("WebSettings.MIXED_CONTENT_NEVER_ALLOW"))
        assertTrue(manager.contains("request.isForMainFrame"))
        assertTrue(manager.contains("uri.host == \"127.0.0.1\""))
        assertTrue(manager.contains("uri.port == LOCAL_SERVER_PORT"))
    }
}
