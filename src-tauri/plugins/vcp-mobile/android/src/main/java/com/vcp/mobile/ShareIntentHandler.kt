package com.vcp.mobile

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.util.Log
import android.webkit.WebView
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import java.io.File
import androidx.core.content.IntentCompat
import java.util.concurrent.atomic.AtomicLong

class ShareIntentHandler(private val plugin: VcpMobilePlugin) {

    companion object {
        private const val TAG = "ShareIntentHandler"
        private const val MAX_SHARED_FILE_BYTES = 100L * 1024L * 1024L
        private const val MAX_SHARED_BATCH_BYTES = 500L * 1024L * 1024L
        private const val MAX_SHARED_FILES = 20
    }

    // WebView 未就绪时缓存待注入数据
    private var pendingShareData: JSObject? = null
    private var pendingShareCacheFiles: List<File> = emptyList()
    private val shareGeneration = AtomicLong(0)
    @Volatile private var frontendReady = false

    private data class ExtractedShare(
        val payload: JSObject,
        val cacheFiles: List<File>,
    )

    /**
     * 入口：由 VcpMobilePlugin.onNewIntent 调用
     */
    fun handleShareIntent(intent: Intent) {
        val action = intent.action
        if (action != Intent.ACTION_SEND &&
            action != Intent.ACTION_SEND_MULTIPLE &&
            action != Intent.ACTION_PROCESS_TEXT) {
            Log.d(TAG, "[handleShareIntent] Ignoring non-share intent: $action")
            return
        }

        Log.i(TAG, "[handleShareIntent] Processing share intent: type=${intent.type}, action=$action")

        val generation = shareGeneration.incrementAndGet()
        val context = plugin.pluginActivity
        val accepted = plugin.executeFileIo {
            val extracted = extractSharedContent(intent, context)
            context.runOnUiThread {
                if (generation != shareGeneration.get()) {
                    extracted.cacheFiles.forEach { file ->
                        try {
                            file.delete()
                        } catch (_: Exception) {}
                    }
                    return@runOnUiThread
                }

                pendingShareCacheFiles.forEach(::deleteQuietly)
                pendingShareData = extracted.payload
                pendingShareCacheFiles = extracted.cacheFiles
                consumeShareIntent(intent)
                val webView = plugin.webViewRef
                if (webView != null && frontendReady) {
                    injectShareData(webView)
                } else {
                    Log.i(TAG, "[handleShareIntent] Frontend not ready, caching share data")
                }
            }
        }
        if (!accepted) {
            Log.w(TAG, "[handleShareIntent] Ignoring share intent after plugin shutdown")
        }
    }

    /**
     * 内部：提取文本和文件 URI
     */
    private fun extractSharedContent(intent: Intent, context: Context): ExtractedShare {
        val root = JSObject()
        val cacheFiles = mutableListOf<File>()

        // ACTION_PROCESS_TEXT: 浏览器/阅读器选中文字菜单
        val processText = if (intent.action == Intent.ACTION_PROCESS_TEXT) {
            intent.getCharSequenceExtra(Intent.EXTRA_PROCESS_TEXT)?.toString()
        } else null

        val text = intent.getStringExtra(Intent.EXTRA_TEXT)
        val subject = intent.getStringExtra(Intent.EXTRA_SUBJECT)

        // 合并来源文本：PROCESS_TEXT > EXTRA_SUBJECT + EXTRA_TEXT
        val combinedText = buildString {
            if (!processText.isNullOrBlank()) {
                append(processText)
            } else {
                if (!subject.isNullOrBlank()) {
                    append(subject)
                }
                if (!text.isNullOrBlank()) {
                    if (isNotEmpty() && !text.startsWith(subject ?: "")) {
                        append("\n")
                    }
                    append(text)
                }
            }
        }

        root.put("text", combinedText.ifBlank { "" })

        // 提取文件 URIs。部分提供方只填 ClipData，另一些只填 EXTRA_STREAM。
        val files = JSArray()
        val errors = JSArray()
        val uris = collectSharedUris(intent)
        if (uris.size > MAX_SHARED_FILES) {
            errors.put("一次最多分享 $MAX_SHARED_FILES 个文件，其余文件已跳过")
        }
        var totalBytes = 0L
        for (uri in uris.take(MAX_SHARED_FILES)) {
            try {
                val remainingBytes = MAX_SHARED_BATCH_BYTES - totalBytes
                if (remainingBytes <= 0L) {
                    errors.put("整批文件超过 500MB 限制，其余文件已跳过")
                    break
                }
                val copied = copyStreamToCache(uri, context, remainingBytes)
                files.put(copied.first)
                cacheFiles.add(copied.second)
                totalBytes += copied.second.length()
            } catch (error: Exception) {
                Log.e(TAG, "[extractSharedContent] Failed to copy shared URI: $uri", error)
                errors.put(error.message ?: "无法读取分享文件")
            }
        }

        root.put("files", files)
        root.put("errors", errors)
        val isDebug = (context.applicationInfo.flags and android.content.pm.ApplicationInfo.FLAG_DEBUGGABLE) != 0
        if (isDebug) {
            Log.i(TAG, "[extractSharedContent] text=${combinedText.take(120)}, fileCount=${files.length()}")
        } else {
            Log.i(TAG, "[extractSharedContent] textLength=${combinedText.length}, fileCount=${files.length()}")
        }
        return ExtractedShare(root, cacheFiles)
    }

    /**
     * 内部：将 content:// URI 复制到 app cache 目录
     */
    private fun collectSharedUris(intent: Intent): List<Uri> {
        val uris = mutableListOf<Uri>()
        if (intent.action == Intent.ACTION_SEND_MULTIPLE) {
            IntentCompat.getParcelableArrayListExtra(
                intent,
                Intent.EXTRA_STREAM,
                Uri::class.java,
            )?.let(uris::addAll)
        } else {
            IntentCompat.getParcelableExtra(
                intent,
                Intent.EXTRA_STREAM,
                Uri::class.java,
            )?.let(uris::add)
        }
        intent.clipData?.let { clipData ->
            for (index in 0 until clipData.itemCount) {
                clipData.getItemAt(index).uri?.let(uris::add)
            }
        }
        return uris.distinct()
    }

    private fun copyStreamToCache(
        uri: Uri,
        context: Context,
        remainingBatchBytes: Long,
    ): Pair<JSObject, File> {
        var targetFile: File? = null
        try {
            val contentResolver = context.contentResolver

            // 获取文件名和 MIME
            var fileName = "shared_file"
            val mimeType = contentResolver.getType(uri) ?: "application/octet-stream"

            contentResolver.query(uri, null, null, null, null)?.use { cursor ->
                val nameIndex = cursor.getColumnIndex(android.provider.OpenableColumns.DISPLAY_NAME)
                if (nameIndex != -1 && cursor.moveToFirst()) {
                    val name = cursor.getString(nameIndex)
                    if (name != null) fileName = name
                }
            }

            fileName = sanitizeFileName(fileName)
            val sharedDir = File(context.cacheDir, "shared").apply { mkdirs() }
            val outputFile = File(sharedDir, "shared_${java.util.UUID.randomUUID()}_$fileName")
            targetFile = outputFile

            contentResolver.openInputStream(uri)?.use { input ->
                outputFile.outputStream().use { output ->
                    val buffer = ByteArray(65536)
                    var total = 0L
                    while (true) {
                        val read = input.read(buffer)
                        if (read < 0) break
                        total += read
                        if (total > MAX_SHARED_FILE_BYTES) {
                            throw IllegalArgumentException("$fileName exceeds the 100MB attachment limit")
                        }
                        if (total > remainingBatchBytes) {
                            throw IllegalArgumentException("整批分享文件超过 500MB 限制")
                        }
                        output.write(buffer, 0, read)
                    }
                }
            } ?: run {
                throw IllegalArgumentException("无法读取分享文件: $fileName")
            }

            val fileInfo = JSObject()
            fileInfo.put("cachePath", outputFile.absolutePath)
            fileInfo.put("mimeType", mimeType)
            fileInfo.put("fileName", fileName)
            fileInfo.put("size", outputFile.length())

            Log.i(TAG, "[copyStreamToCache] Copied: $fileName -> ${outputFile.absolutePath} (size=${outputFile.length()})")
            return fileInfo to outputFile
        } catch (e: Exception) {
            try {
                targetFile?.delete()
            } catch (_: Exception) {}
            Log.e(TAG, "[copyStreamToCache] Failed to copy stream", e)
            throw e
        }
    }

    fun onWebViewLoaded() {
        frontendReady = false
    }

    fun markFrontendReady(webView: WebView?) {
        frontendReady = true
        injectShareData(webView)
    }

    fun onWebViewDestroyed() {
        frontendReady = false
    }

    private fun consumeShareIntent(intent: Intent) {
        intent.action = null
        intent.type = null
        intent.clipData = null
        intent.removeExtra(Intent.EXTRA_TEXT)
        intent.removeExtra(Intent.EXTRA_SUBJECT)
        intent.removeExtra(Intent.EXTRA_STREAM)
        intent.removeExtra(Intent.EXTRA_PROCESS_TEXT)
    }

    private fun sanitizeFileName(rawName: String): String {
        val baseName = File(rawName.replace('\\', '/')).name
        return baseName
            .replace(Regex("[\\u0000-\\u001f\\u007f/\\\\]"), "_")
            .trim()
            .ifEmpty { "shared_file" }
            .take(180)
    }

    private fun deleteQuietly(file: File) {
        try {
            if (file.exists() && !file.delete()) {
                Log.w(TAG, "[deleteQuietly] Failed to remove stale share cache: ${file.absolutePath}")
            }
        } catch (error: Exception) {
            Log.w(TAG, "[deleteQuietly] Failed to remove stale share cache: ${file.absolutePath}", error)
        }
    }

    /**
     * 通过 evaluateJavascript 注入 WebView
     */
    fun injectShareData(webView: WebView?) {
        if (webView == null) return

        val data = pendingShareData
        if (data == null) {
            Log.d(TAG, "[injectShareData] No pending share data")
            return
        }

        try {
            @Suppress("DEPRECATION")
            val dataJson = data.toString()
            val safeJson = escapeJsonForJsString(dataJson)
            val script = "window.dispatchEvent(new CustomEvent('vcp-share-intent', { detail: JSON.parse(\"$safeJson\") }))"
            webView.evaluateJavascript(script, null)

            Log.i(TAG, "[injectShareData] Share data injected into WebView successfully")
            pendingShareData = null
            // The frontend owns these cache paths after the event is dispatched.
            pendingShareCacheFiles = emptyList()
        } catch (e: Exception) {
            Log.e(TAG, "[injectShareData] Failed to inject share data", e)
        }
    }

    /**
     * JSON 字符串转义，安全嵌入 JavaScript 字符串
     */
    private fun escapeJsonForJsString(json: String): String {
        return json
            .replace("\\", "\\\\")
            .replace("\"", "\\\"")
            .replace("'", "\\'")
            .replace("\n", "\\n")
            .replace("\r", "\\r")
    }
}
