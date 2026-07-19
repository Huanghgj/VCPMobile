package com.vcp.mobile.affect

import android.content.Context
import ai.onnxruntime.OnnxTensor
import ai.onnxruntime.OrtEnvironment
import ai.onnxruntime.OrtSession
import org.json.JSONObject
import java.io.Closeable
import java.io.File
import java.nio.LongBuffer
import java.security.MessageDigest
import java.util.concurrent.Executors
import java.util.concurrent.RejectedExecutionException
import java.util.concurrent.ScheduledExecutorService
import java.util.concurrent.ScheduledFuture
import java.util.concurrent.TimeUnit
import java.util.concurrent.TimeoutException
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference
import java.util.concurrent.ConcurrentHashMap

data class AffectClassificationResult(
    val scores: LinkedHashMap<String, Float>,
    val modelId: String,
    val modelVersion: String,
    val inferenceMs: Long,
    val truncated: Boolean,
)

class AffectClassifier(context: Context) : Closeable {
    private val appContext = context.applicationContext
    private val environment = OrtEnvironment.getEnvironment()
    private val inferenceExecutor = Executors.newSingleThreadExecutor { runnable ->
        Thread(runnable, "vcp-affect-inference").apply { priority = Thread.NORM_PRIORITY - 1 }
    }
    private val timeoutExecutor: ScheduledExecutorService = Executors.newSingleThreadScheduledExecutor { runnable ->
        Thread(runnable, "vcp-affect-timeout").apply { isDaemon = true }
    }
    private val runtimeLock = Any()
    private val closed = AtomicBoolean(false)
    private val pendingRequests = ConcurrentHashMap.newKeySet<PendingRequest>()
    @Volatile private var runtime: ModelRuntime? = null

    fun classifyAsync(
        text: String,
        timeoutMs: Long,
        onSuccess: (AffectClassificationResult) -> Unit,
        onFailure: (Throwable) -> Unit,
    ) {
        if (closed.get()) {
            onFailure(IllegalStateException("Affect classifier is closed"))
            return
        }
        if (text.isBlank()) {
            onFailure(IllegalArgumentException("Affect classification text is empty"))
            return
        }

        val boundedTimeoutMs = timeoutMs.coerceIn(MIN_TIMEOUT_MS, MAX_TIMEOUT_MS)
        val deadline = AffectDeadline.afterMillis(boundedTimeoutMs)
        val request = PendingRequest(onSuccess, onFailure)
        pendingRequests += request
        try {
            request.attachTimeout(
                timeoutExecutor.schedule(
                    {
                        request.fail(TimeoutException("Affect classification exceeded ${boundedTimeoutMs}ms"))
                        pendingRequests.remove(request)
                    },
                    boundedTimeoutMs,
                    TimeUnit.MILLISECONDS,
                ),
            )
            inferenceExecutor.execute {
                if (request.isCompleted()) {
                    pendingRequests.remove(request)
                    return@execute
                }
                try {
                    deadline.throwIfExpired()
                    request.succeed(classify(text, deadline))
                } catch (error: Throwable) {
                    request.fail(error)
                } finally {
                    pendingRequests.remove(request)
                }
            }
        } catch (error: RejectedExecutionException) {
            pendingRequests.remove(request)
            request.fail(IllegalStateException("Affect classifier is unavailable", error))
        }
    }

    private fun classify(text: String, deadline: AffectDeadline): AffectClassificationResult {
        check(!closed.get()) { "Affect classifier is closed" }
        synchronized(runtimeLock) {
            deadline.throwIfExpired()
            val current = getOrLoadRuntime(deadline)
            deadline.throwIfExpired()
            val boundedText = text.take(MAX_INPUT_CHARS)
            val encoded = current.tokenizer.encode(boundedText, WordPieceTokenizer.DEFAULT_MAX_LENGTH)
            deadline.throwIfExpired()
            val startedAt = System.nanoTime()
            val shape = longArrayOf(1L, WordPieceTokenizer.DEFAULT_MAX_LENGTH.toLong())

            OnnxTensor.createTensor(environment, LongBuffer.wrap(encoded.inputIds), shape).use { inputIds ->
                OnnxTensor.createTensor(environment, LongBuffer.wrap(encoded.attentionMask), shape).use { attentionMask ->
                    OnnxTensor.createTensor(environment, LongBuffer.wrap(encoded.tokenTypeIds), shape).use { tokenTypeIds ->
                        OrtSession.RunOptions().use { runOptions ->
                            val terminateRun = timeoutExecutor.schedule(
                                { runOptions.setTerminate(true) },
                                deadline.remainingNanos(),
                                TimeUnit.NANOSECONDS,
                            )
                            try {
                                deadline.throwIfExpired()
                                val inputs = linkedMapOf<String, OnnxTensor>()
                                inputs[current.config.inputIdsName] = inputIds
                                if (current.session.inputNames.contains(current.config.attentionMaskName)) {
                                    inputs[current.config.attentionMaskName] = attentionMask
                                }
                                if (current.session.inputNames.contains(current.config.tokenTypeIdsName)) {
                                    inputs[current.config.tokenTypeIdsName] = tokenTypeIds
                                }

                                current.session.run(inputs, runOptions).use { result ->
                                    deadline.throwIfExpired()
                                    val output = current.config.outputName
                                        ?.let { name -> result.get(name).orElse(null) }
                                        ?: result[0]
                                    val logits = AffectOutputMapper.extractLogits(output.value)
                                    val scores = AffectOutputMapper.mapLogits(
                                        logits,
                                        current.config.labels,
                                        current.config.labelAliases,
                                    )
                                    deadline.throwIfExpired()
                                    return AffectClassificationResult(
                                        scores = scores,
                                        modelId = current.config.modelId,
                                        modelVersion = current.config.modelVersion,
                                        inferenceMs = TimeUnit.NANOSECONDS.toMillis(System.nanoTime() - startedAt),
                                        truncated = encoded.truncated || boundedText.length < text.length,
                                    )
                                }
                            } finally {
                                terminateRun.cancel(false)
                            }
                        }
                    }
                }
            }
        }
    }

    private fun getOrLoadRuntime(deadline: AffectDeadline): ModelRuntime {
        runtime?.let { return it }
        check(!closed.get()) { "Affect classifier is closed" }
        deadline.throwIfExpired()

        val config = loadConfig()
        deadline.throwIfExpired()
        val vocabulary = readAsset(VOCAB_ASSET).lineSequence().filter(String::isNotEmpty).toList()
        check(vocabulary.isNotEmpty()) { "Affect vocabulary is empty: $VOCAB_ASSET" }
        deadline.throwIfExpired()
        val modelFile = materializeModel(config, deadline)
        deadline.throwIfExpired()
        val options = OrtSession.SessionOptions().apply {
            setIntraOpNumThreads(1)
            setInterOpNumThreads(1)
            setOptimizationLevel(OrtSession.SessionOptions.OptLevel.ALL_OPT)
        }
        try {
            val session = environment.createSession(modelFile.absolutePath, options)
            if (deadline.isExpired() || closed.get()) {
                session.close()
                deadline.throwIfExpired()
                error("Affect classifier is closed")
            }
            val loaded = ModelRuntime(
                config = config,
                tokenizer = WordPieceTokenizer(vocabulary, config.lowerCase),
                options = options,
                session = session,
            )
            runtime = loaded
            return loaded
        } catch (error: Throwable) {
            options.close()
            throw error
        }
    }

    private fun loadConfig(): AffectModelConfig {
        val json = try {
            JSONObject(readAsset(CONFIG_ASSET))
        } catch (error: Throwable) {
            throw IllegalStateException("Affect model config asset is missing or invalid: $CONFIG_ASSET", error)
        }
        val modelId = json.optString("modelId").trim()
        val modelVersion = json.optString("modelVersion", json.optString("version")).trim()
        check(modelId.isNotEmpty()) { "Affect model config must define modelId" }
        check(modelVersion.isNotEmpty()) { "Affect model config must define modelVersion" }

        val labelsJson = json.optJSONArray("labels")
        val labels = if (labelsJson == null) {
            AffectOutputMapper.CANONICAL_LABELS
        } else {
            List(labelsJson.length()) { index -> labelsJson.getString(index) }
        }
        check(labels.isNotEmpty()) { "Affect model config labels must not be empty" }

        val aliasesJson = json.optJSONObject("labelMap")
        val aliases = linkedMapOf<String, String>()
        aliasesJson?.keys()?.forEach { key -> aliases[key] = aliasesJson.getString(key) }

        return AffectModelConfig(
            modelId = modelId,
            modelVersion = modelVersion,
            labels = labels,
            labelAliases = aliases,
            inputIdsName = json.optString("inputIdsName", "input_ids"),
            attentionMaskName = json.optString("attentionMaskName", "attention_mask"),
            tokenTypeIdsName = json.optString("tokenTypeIdsName", "token_type_ids"),
            outputName = json.optString("outputName").trim().ifEmpty { null },
            lowerCase = json.optBoolean("lowerCase", true),
            sha256 = json.optString("sha256").trim().lowercase().ifEmpty { null },
        )
    }

    private fun materializeModel(config: AffectModelConfig, deadline: AffectDeadline): File {
        val safeVersion = config.modelVersion.replace(Regex("[^A-Za-z0-9._-]"), "_")
        val modelDir = File(appContext.noBackupFilesDir, "affect-model/$safeVersion")
        val modelFile = File(modelDir, "model.onnx")
        if (modelFile.isFile && modelFile.length() > 0L && verifySha256(modelFile, config.sha256, deadline)) {
            return modelFile
        }

        deadline.throwIfExpired()
        modelDir.mkdirs()
        val temporary = File(modelDir, "model.onnx.tmp")
        try {
            appContext.assets.open(MODEL_ASSET).use { source ->
                temporary.outputStream().use { target ->
                    val buffer = ByteArray(DEFAULT_BUFFER_SIZE)
                    while (true) {
                        deadline.throwIfExpired()
                        val count = source.read(buffer)
                        if (count < 0) break
                        target.write(buffer, 0, count)
                    }
                }
            }
        } catch (error: Throwable) {
            temporary.delete()
            throw IllegalStateException(
                "Affect model asset is missing: $MODEL_ASSET; install a packaged ONNX model or use heuristic fallback",
                error,
            )
        }
        check(temporary.length() > 0L) { "Affect model asset is empty: $MODEL_ASSET" }
        check(verifySha256(temporary, config.sha256, deadline)) { "Affect model SHA-256 mismatch" }
        deadline.throwIfExpired()
        if (modelFile.exists() && !modelFile.delete()) {
            temporary.delete()
            error("Unable to replace cached affect model")
        }
        check(temporary.renameTo(modelFile)) { "Unable to commit cached affect model" }
        return modelFile
    }

    private fun verifySha256(file: File, expected: String?, deadline: AffectDeadline): Boolean {
        if (expected == null) return true
        val digest = MessageDigest.getInstance("SHA-256")
        file.inputStream().buffered().use { input ->
            val buffer = ByteArray(DEFAULT_BUFFER_SIZE)
            while (true) {
                deadline.throwIfExpired()
                val count = input.read(buffer)
                if (count < 0) break
                digest.update(buffer, 0, count)
            }
        }
        val actual = digest.digest().joinToString("") { byte -> "%02x".format(byte) }
        return actual == expected
    }

    private fun readAsset(path: String): String = try {
        appContext.assets.open(path).bufferedReader(Charsets.UTF_8).use { it.readText() }
    } catch (error: Throwable) {
        throw IllegalStateException("Affect model asset is missing: $path", error)
    }

    override fun close() {
        if (!closed.compareAndSet(false, true)) return
        pendingRequests.toList().forEach {
            it.fail(IllegalStateException("Affect classifier was unloaded"))
        }
        pendingRequests.clear()
        inferenceExecutor.shutdownNow()
        timeoutExecutor.shutdownNow()
        synchronized(runtimeLock) {
            runtime?.close()
            runtime = null
        }
    }

    private data class AffectModelConfig(
        val modelId: String,
        val modelVersion: String,
        val labels: List<String>,
        val labelAliases: Map<String, String>,
        val inputIdsName: String,
        val attentionMaskName: String,
        val tokenTypeIdsName: String,
        val outputName: String?,
        val lowerCase: Boolean,
        val sha256: String?,
    )

    private data class ModelRuntime(
        val config: AffectModelConfig,
        val tokenizer: WordPieceTokenizer,
        val options: OrtSession.SessionOptions,
        val session: OrtSession,
    ) : Closeable {
        override fun close() {
            session.close()
            options.close()
        }
    }

    private class PendingRequest(
        private val onSuccess: (AffectClassificationResult) -> Unit,
        private val onFailure: (Throwable) -> Unit,
    ) {
        private val completed = AtomicBoolean(false)
        private val timeoutFuture = AtomicReference<ScheduledFuture<*>?>()

        fun attachTimeout(future: ScheduledFuture<*>) {
            timeoutFuture.set(future)
            if (completed.get()) future.cancel(false)
        }

        fun isCompleted(): Boolean = completed.get()

        fun succeed(result: AffectClassificationResult) {
            if (completed.compareAndSet(false, true)) {
                timeoutFuture.get()?.cancel(false)
                onSuccess(result)
            }
        }

        fun fail(error: Throwable) {
            if (completed.compareAndSet(false, true)) {
                timeoutFuture.get()?.cancel(false)
                onFailure(error)
            }
        }
    }

    companion object {
        private const val MODEL_ASSET = "affect/model.onnx"
        private const val VOCAB_ASSET = "affect/vocab.txt"
        private const val CONFIG_ASSET = "affect/model.json"
        private const val MAX_INPUT_CHARS = 4_000
        private const val MIN_TIMEOUT_MS = 200L
        private const val MAX_TIMEOUT_MS = 5_000L
    }
}

internal class AffectDeadline private constructor(private val deadlineNanos: Long) {
    fun remainingNanos(nowNanos: Long = System.nanoTime()): Long =
        (deadlineNanos - nowNanos).coerceAtLeast(0L)

    fun isExpired(nowNanos: Long = System.nanoTime()): Boolean = remainingNanos(nowNanos) == 0L

    fun throwIfExpired(nowNanos: Long = System.nanoTime()) {
        if (isExpired(nowNanos)) throw TimeoutException("Affect classification deadline expired")
    }

    companion object {
        fun afterMillis(timeoutMs: Long, nowNanos: Long = System.nanoTime()): AffectDeadline {
            val durationNanos = TimeUnit.MILLISECONDS.toNanos(timeoutMs.coerceAtLeast(0L))
            val deadline = if (Long.MAX_VALUE - nowNanos < durationNanos) Long.MAX_VALUE else nowNanos + durationNanos
            return AffectDeadline(deadline)
        }
    }
}
