package com.vcp.mobile

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Matrix
import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaExtractor
import android.media.MediaFormat
import android.media.MediaMetadataRetriever
import android.util.Log
import java.io.BufferedOutputStream
import java.io.ByteArrayOutputStream
import java.io.File
import java.io.FileOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.ArrayDeque
import java.util.UUID
import java.util.concurrent.Executors
import kotlin.math.abs
import kotlin.math.max

object MediaBridge {
    private const val TAG = "VCPMobile_MediaBridge"
    private val fileIoExecutor = Executors.newFixedThreadPool(4)

    /**
     * 异步图片缩放与 JPEG 压缩
     * 长边等比例缩放到 1120 包络框内，小图不放大。90% 质量。
     */
    fun processImageAsync(
        inputPath: String,
        context: Context,
        callback: (Result<String>) -> Unit
    ) {
        fileIoExecutor.execute {
            var rawBitmap: Bitmap? = null
            var scaledBitmap: Bitmap? = null
            var outputBitmap: Bitmap? = null
            try {
                val file = File(inputPath)
                if (!file.exists()) {
                    callback(Result.failure(FileNotFoundException("Input file not found: $inputPath")))
                    return@execute
                }

                // 1. 获取图片原始宽高而不加载入内存，防止 OOM
                val options = BitmapFactory.Options().apply {
                    inJustDecodeBounds = true
                }
                BitmapFactory.decodeFile(inputPath, options)
                val origW = options.outWidth
                val origH = options.outHeight

                if (origW <= 0 || origH <= 0) {
                    callback(Result.failure(Exception("Invalid image dimensions")))
                    return@execute
                }

                // 2. 计算缩放包络
                val maxDim = max(origW, origH)
                val scale = if (maxDim > 1120) {
                    1120f / maxDim
                } else {
                    1.0f
                }

                val targetW = (origW * scale).toInt()
                val targetH = (origH * scale).toInt()

                // 3. 计算合理 sampling size 减少内存开销
                val decodeOptions = BitmapFactory.Options().apply {
                    inSampleSize = calculateInSampleSize(origW, origH, targetW, targetH)
                }
                val decodedBitmap = BitmapFactory.decodeFile(inputPath, decodeOptions)
                    ?: throw Exception("Failed to decode image bitmap")
                rawBitmap = decodedBitmap

                // 4. 精确缩放 (filter = true 提供高保真插值)
                scaledBitmap = if (decodedBitmap.width != targetW || decodedBitmap.height != targetH) {
                    Bitmap.createScaledBitmap(decodedBitmap, targetW, targetH, true)
                } else {
                    decodedBitmap
                }
                val scaled = scaledBitmap ?: throw Exception("Failed to scale image bitmap")
                outputBitmap = if (scaled.hasAlpha()) {
                    Bitmap.createBitmap(scaled.width, scaled.height, Bitmap.Config.ARGB_8888).also { flattened ->
                        val canvas = Canvas(flattened)
                        canvas.drawColor(Color.WHITE)
                        canvas.drawBitmap(scaled, 0f, 0f, null)
                    }
                } else {
                    scaled
                }

                // 5. 写入 JPEG，避免部分 OpenAI 兼容上游拒绝 image/webp data URL。
                val uploadsDir = File(context.cacheDir, "uploads").apply { mkdirs() }
                val outFile = File(uploadsDir, "img_" + UUID.randomUUID().toString() + ".jpg")
                val bitmapForOutput = outputBitmap ?: throw Exception("Failed to prepare output bitmap")
                FileOutputStream(outFile).use { out ->
                    bitmapForOutput.compress(Bitmap.CompressFormat.JPEG, 90, out)
                }

                Log.d(TAG, "Image scale success: ${outFile.absolutePath} (${targetW}x${targetH})")
                callback(Result.success(outFile.absolutePath))
            } catch (e: Exception) {
                Log.e(TAG, "Image scale error", e)
                callback(Result.failure(e))
            } finally {
                // Bitmap 占用 native 内存，异常路径也要及时释放。
                if (scaledBitmap != null && scaledBitmap !== rawBitmap) {
                    scaledBitmap.recycle()
                }
                if (outputBitmap != null && outputBitmap !== scaledBitmap) {
                    outputBitmap.recycle()
                }
                rawBitmap?.recycle()
            }
        }
    }

    /**
     * 异步视频帧提取与 JPEG 压缩
     * 时长决策 FPS：<=60s 1fps，>60s 0.5fps
     * 包络框尺寸：1280x720
     * 去重：< 1.5s
     * 最大帧数：300帧 (等距采样)
     * JPEG 质量 90%
     */
    fun processVideoAsync(
        inputPath: String,
        context: Context,
        callback: (Result<List<String>>) -> Unit
    ) {
        fileIoExecutor.execute {
            var retriever: MediaMetadataRetriever? = null
            try {
                val file = File(inputPath)
                if (!file.exists()) {
                    callback(Result.failure(FileNotFoundException("Video file not found: $inputPath")))
                    return@execute
                }

                val metadataRetriever = MediaMetadataRetriever()
                retriever = metadataRetriever
                metadataRetriever.setDataSource(inputPath)

                // 1. 获取视频基本元数据
                val durationStr = metadataRetriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_DURATION)
                    ?: throw Exception("Failed to retrieve video duration")
                val durationMs = durationStr.toLong()
                val durationSec = durationMs / 1000.0

                val origW = metadataRetriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_VIDEO_WIDTH)?.toInt() ?: 1280
                val origH = metadataRetriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_VIDEO_HEIGHT)?.toInt() ?: 720

                // 2. 决策采样率
                val fps = if (durationSec <= 60.0) 1.0 else 0.5

                // 3. 构建均匀采样时间戳队列 (单位：秒)
                val allTimes = ArrayList<Double>()
                var t = 0.0
                while (t < durationSec) {
                    allTimes.add(t)
                    t += 1.0 / fps
                }

                // 4. 进行去重 (双指针，这里本来还有场景切换，但原方案中主要也是按间隔去重，此处对齐 >= 1.5s 的绝对间隔)
                val dedupedTimes = ArrayList<Double>()
                val dedupThreshold = 1.5
                for (time in allTimes) {
                    if (dedupedTimes.isEmpty() || abs(time - dedupedTimes.last()) >= dedupThreshold) {
                        dedupedTimes.add(time)
                    }
                }

                // 5. 限制最大帧数 (等距重采样)
                var finalTimes = dedupedTimes
                val maxFrames = 300
                if (finalTimes.size > maxFrames) {
                    val sampled = ArrayList<Double>(maxFrames)
                    val step = finalTimes.size.toDouble() / maxFrames.toDouble()
                    var idx = 0.0
                    while (idx < finalTimes.size) {
                        sampled.add(finalTimes[idx.toInt()])
                        idx += step
                    }
                    finalTimes = sampled
                }

                // 6. 计算缩放包络框尺寸 (宽限1280，高限720)
                val aspect = origW.toFloat() / origH.toFloat()
                var targetW = origW
                var targetH = origH
                if (aspect > 1.777778f) { // 宽屏
                    if (origW > 1280) {
                        targetW = 1280
                        targetH = (1280 / aspect).toInt()
                    }
                } else { // 窄屏/竖屏
                    if (origH > 720) {
                        targetH = 720
                        targetW = (720 * aspect).toInt()
                    }
                }

                // 7. 循环提帧并压缩保存
                val outputPaths = ArrayList<String>()
                val uploadsDir = File(context.cacheDir, "uploads").apply { mkdirs() }
                val tempFolder = File(uploadsDir, "vid_" + UUID.randomUUID().toString())
                if (!tempFolder.exists()) tempFolder.mkdirs()

                // 为了速度考虑，MediaMetadataRetriever 在 API 27+ 支持指定大小获取，但兼容性较差，
                // 我们在获取后统一由 Matrix 或 Bitmap.createScaledBitmap 高保真缩小。
                for ((idx, timeSec) in finalTimes.withIndex()) {
                    val timeUs = (timeSec * 1_000_000).toLong()
                    var frameBmp: Bitmap? = null
                    var scaledBmp: Bitmap? = null
                    try {
                        // OPTION_CLOSEST_SYNC (更安全，防黑屏) 或 OPTION_CLOSEST
                        frameBmp = metadataRetriever.getFrameAtTime(timeUs, MediaMetadataRetriever.OPTION_CLOSEST_SYNC)

                        if (frameBmp == null) {
                            // 兜底尝试任何最近帧
                            frameBmp = metadataRetriever.getFrameAtTime(timeUs, MediaMetadataRetriever.OPTION_NEXT_SYNC)
                        }

                        if (frameBmp != null) {
                            // 高保真等比例缩放
                            scaledBmp = if (frameBmp.width != targetW || frameBmp.height != targetH) {
                                Bitmap.createScaledBitmap(frameBmp, targetW, targetH, true)
                            } else {
                                frameBmp
                            }
                            val outputBitmap = scaledBmp

                            val frameFile = File(tempFolder, String.format("frame_%04d.jpg", idx + 1))
                            FileOutputStream(frameFile).use { out ->
                                outputBitmap.compress(Bitmap.CompressFormat.JPEG, 90, out)
                            }
                            outputPaths.add(frameFile.absolutePath)
                        }
                    } finally {
                        // 每帧都要用完就释放，别让 300 张 Bitmap 在内存里开无遮大会喵♡
                        if (scaledBmp != null && scaledBmp !== frameBmp) {
                            scaledBmp.recycle()
                        }
                        frameBmp?.recycle()
                    }
                }

                Log.d(TAG, "Video frame extraction success: ${outputPaths.size} frames extracted.")
                callback(Result.success(outputPaths))
            } catch (e: Exception) {
                Log.e(TAG, "Video processing error", e)
                callback(Result.failure(e))
            } finally {
                // Retriever 抓着解码器句柄，异常路径也得拔出来，不然后台耗电会偷偷顶上去喵♡
                try {
                    retriever?.release()
                } catch (ignored: Exception) {}
            }
        }
    }

    /**
     * 异步音频转码至 AAC-LC 16kHz Mono 32kbps
     * 自动处理重采样、声道合并以及 ADTS 帧构建封装。
     */
    fun processAudioAsync(
        inputPath: String,
        context: Context,
        callback: (Result<String>) -> Unit
    ) {
        fileIoExecutor.execute {
            var extractor: MediaExtractor? = null
            var decoder: MediaCodec? = null
            var encoder: MediaCodec? = null
            val uploadsDir = File(context.cacheDir, "uploads").apply { mkdirs() }
            val outFile = File(uploadsDir, "aud_" + UUID.randomUUID().toString() + ".aac")
            var fos: FileOutputStream? = null
            var bos: BufferedOutputStream? = null

            try {
                val file = File(inputPath)
                if (!file.exists()) {
                    callback(Result.failure(FileNotFoundException("Audio file not found: $inputPath")))
                    return@execute
                }

                extractor = MediaExtractor()
                extractor.setDataSource(inputPath)

                var audioTrackIndex = -1
                var inputFormat: MediaFormat? = null
                for (i in 0 until extractor.trackCount) {
                    val format = extractor.getTrackFormat(i)
                    val mime = format.getString(MediaFormat.KEY_MIME) ?: ""
                    if (mime.startsWith("audio/")) {
                        audioTrackIndex = i
                        inputFormat = format
                        break
                    }
                }

                if (audioTrackIndex == -1 || inputFormat == null) {
                    throw Exception("No audio track found in $inputPath")
                }

                extractor.selectTrack(audioTrackIndex)

                // 1. 初始化解码器
                val mimeType = inputFormat.getString(MediaFormat.KEY_MIME)!!
                decoder = MediaCodec.createDecoderByType(mimeType)
                decoder.configure(inputFormat, null, null, 0)
                decoder.start()

                // 2. 初始化 AAC 编码器
                val outFormat = MediaFormat.createAudioFormat(MediaFormat.MIMETYPE_AUDIO_AAC, 16000, 1)
                outFormat.setInteger(MediaFormat.KEY_AAC_PROFILE, MediaCodecInfo.CodecProfileLevel.AACObjectLC)
                outFormat.setInteger(MediaFormat.KEY_BIT_RATE, 32000)
                outFormat.setInteger(MediaFormat.KEY_MAX_INPUT_SIZE, 16384)

                encoder = MediaCodec.createEncoderByType(MediaFormat.MIMETYPE_AUDIO_AAC)
                encoder.configure(outFormat, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
                encoder.start()

                fos = FileOutputStream(outFile)
                bos = BufferedOutputStream(fos, 32768)

                // 转码循环变量
                var isDecoderInputEOS = false
                var isDecoderOutputEOS = false
                var isEncoderInputEOS = false
                var isEncoderOutputEOS = false

                val decoderInputBuffers = decoder.inputBuffers
                val decoderOutputBuffers = decoder.outputBuffers
                val encoderInputBuffers = encoder.inputBuffers
                val encoderOutputBuffers = encoder.outputBuffers

                val decoderBufferInfo = MediaCodec.BufferInfo()
                val encoderBufferInfo = MediaCodec.BufferInfo()
                var encodedPcmBytes = 0L

                // 获取输入的源音频参数
                val srcSampleRate = inputFormat.getInteger(MediaFormat.KEY_SAMPLE_RATE)
                val srcChannelCount = inputFormat.getInteger(MediaFormat.KEY_CHANNEL_COUNT)

                val pendingPcm = ArrayDeque<ByteArray>()
                var pendingPcmBytes = 0
                var pendingChunkOffset = 0
                val encoderChunkBytes = 4096

                // 最大压制时长硬截断: 3500s -> 3500,000,000 Us
                val maxDurationUs = 3500L * 1_000_000L

                fun encoderPtsForOffset(offsetBytes: Long): Long {
                    val samples = offsetBytes / 2L // 16-bit mono after processPcmData()
                    return samples * 1_000_000L / 16000L
                }

                fun feedEncoderPending() {
                    if (isEncoderInputEOS) return
                    if (pendingPcmBytes < encoderChunkBytes && !isDecoderOutputEOS) return

                    while (pendingPcmBytes >= encoderChunkBytes ||
                        (isDecoderOutputEOS && pendingPcmBytes > 0) ||
                        (isDecoderOutputEOS && pendingPcmBytes == 0)
                    ) {
                        val encInputBufIndex = encoder.dequeueInputBuffer(10000)
                        if (encInputBufIndex < 0) break

                        val sizeToFeed = Math.min(encoderChunkBytes, pendingPcmBytes)
                        val encBuffer = encoderInputBuffers[encInputBufIndex]
                        encBuffer.clear()
                        var bytesCopied = 0
                        while (bytesCopied < sizeToFeed && pendingPcm.isNotEmpty()) {
                            val pendingChunk = pendingPcm.peek()
                            val bytesFromChunk = Math.min(sizeToFeed - bytesCopied, pendingChunk.size - pendingChunkOffset)
                            encBuffer.put(pendingChunk, pendingChunkOffset, bytesFromChunk)
                            bytesCopied += bytesFromChunk
                            pendingChunkOffset += bytesFromChunk
                            pendingPcmBytes -= bytesFromChunk
                            if (pendingChunkOffset >= pendingChunk.size) {
                                pendingPcm.remove()
                                pendingChunkOffset = 0
                            }
                        }

                        val flags = if (isDecoderOutputEOS && pendingPcmBytes == 0) {
                            MediaCodec.BUFFER_FLAG_END_OF_STREAM
                        } else {
                            0
                        }
                        val presentationTimeUs = encoderPtsForOffset(encodedPcmBytes)
                        encoder.queueInputBuffer(
                            encInputBufIndex, 0, sizeToFeed,
                            presentationTimeUs, flags
                        )
                        encodedPcmBytes += sizeToFeed.toLong()
                        if ((flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM) != 0) {
                            // EOS 也要温柔射进编码器，避免空尾巴音频把循环吊死喵♡
                            isEncoderInputEOS = true
                            break
                        }
                    }
                }

                while (!isEncoderOutputEOS) {
                    // Feed 解码器
                    if (!isDecoderInputEOS) {
                        val inputBufIndex = decoder.dequeueInputBuffer(10000)
                        if (inputBufIndex >= 0) {
                            val dstBuffer = decoderInputBuffers[inputBufIndex]
                            dstBuffer.clear()
                            val sampleSize = extractor.readSampleData(dstBuffer, 0)
                            val presentationTimeUs = extractor.sampleTime

                            if (sampleSize < 0 || presentationTimeUs > maxDurationUs) {
                                decoder.queueInputBuffer(
                                    inputBufIndex, 0, 0, 0,
                                    MediaCodec.BUFFER_FLAG_END_OF_STREAM
                                )
                                isDecoderInputEOS = true
                            } else {
                                decoder.queueInputBuffer(
                                    inputBufIndex, 0, sampleSize, presentationTimeUs, 0
                                )
                                extractor.advance()
                            }
                        }
                    }

                    // 从解码器拿 PCM
                    if (!isDecoderOutputEOS) {
                        val res = decoder.dequeueOutputBuffer(decoderBufferInfo, 10000)
                        if (res >= 0) {
                            val pcmBuffer = decoderOutputBuffers[res]
                            pcmBuffer.position(decoderBufferInfo.offset)
                            pcmBuffer.limit(decoderBufferInfo.offset + decoderBufferInfo.size)

                            val chunk = ByteArray(decoderBufferInfo.size)
                            pcmBuffer.get(chunk)
                            decoder.releaseOutputBuffer(res, false)

                            // 将解码 PCM 进行重采样和降声道
                            val processedPcm = processPcmData(chunk, srcSampleRate, srcChannelCount)
                            if (processedPcm.isNotEmpty()) {
                                pendingPcm.add(processedPcm)
                                pendingPcmBytes += processedPcm.size
                            }

                            if ((decoderBufferInfo.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM) != 0) {
                                isDecoderOutputEOS = true
                            }

                            // Feed 编码器：输入缓冲可用就立刻推进，别让 PCM 尾巴湿漉漉地堆在内存里喵♡
                            feedEncoderPending()
                        } else if (res == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED) {
                            Log.d(TAG, "Decoder output format changed")
                        }
                    }

                    // 从编码器拿 AAC 帧，加上 ADTS 头写入文件
                    val encOutBufIndex = encoder.dequeueOutputBuffer(encoderBufferInfo, 10000)
                    if (encOutBufIndex >= 0) {
                        val encodedBuffer = encoderOutputBuffers[encOutBufIndex]
                        encodedBuffer.position(encoderBufferInfo.offset)
                        encodedBuffer.limit(encoderBufferInfo.offset + encoderBufferInfo.size)

                        if ((encoderBufferInfo.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG) == 0) {
                            val outPacketSize = encoderBufferInfo.size + 7
                            val packet = ByteArray(outPacketSize)
                            // AAC profile LC = 2, 16kHz = index 8, channel Mono = 1
                            addADTStoPacket(packet, outPacketSize)
                            encodedBuffer.get(packet, 7, encoderBufferInfo.size)
                            bos.write(packet, 0, outPacketSize)
                        }
                        encoder.releaseOutputBuffer(encOutBufIndex, false)

                        if ((encoderBufferInfo.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM) != 0) {
                            isEncoderOutputEOS = true
                        }
                    }

                    // 背压兜底：释放编码输出后再补喂一次，避免输入缓冲刚解套却没人继续推进。
                    feedEncoderPending()
                }

                bos.flush()
                Log.d(TAG, "Audio transcode success: ${outFile.absolutePath}")
                callback(Result.success(outFile.absolutePath))
            } catch (e: Exception) {
                Log.e(TAG, "Audio transcode error", e)
                callback(Result.failure(e))
            } finally {
                try { extractor?.release() } catch (ignored: Exception) {}
                try { decoder?.stop(); decoder?.release() } catch (ignored: Exception) {}
                try { encoder?.stop(); encoder?.release() } catch (ignored: Exception) {}
                try { bos?.close() } catch (ignored: Exception) {}
                try { fos?.close() } catch (ignored: Exception) {}
            }
        }
    }

    // =============================================================================
    // 辅助函数
    // =============================================================================

    private fun calculateInSampleSize(origW: Int, origH: Int, reqW: Int, reqH: Int): Int {
        var inSampleSize = 1
        if (origH > reqH || origW > reqW) {
            val halfHeight = origH / 2
            val halfWidth = origW / 2
            while ((halfHeight / inSampleSize) >= reqH && (halfWidth / inSampleSize) >= reqW) {
                inSampleSize *= 2
            }
        }
        return inSampleSize
    }

    /**
     * 重采样 PCM 数据：多声道平均合并为单声道，并重采样为 16000Hz 16-bit PCM。
     */
    private fun processPcmData(srcBytes: ByteArray, srcSampleRate: Int, srcChannels: Int): ByteArray {
        if (srcBytes.isEmpty()) return srcBytes

        // 1. 将 ByteArray 解析为 ShortArray (16-bit PCM)
        val srcBuffer = ByteBuffer.wrap(srcBytes).order(ByteOrder.LITTLE_ENDIAN)
        val srcShorts = ShortArray(srcBytes.size / 2)
        srcBuffer.asShortBuffer().get(srcShorts)

        // 2. 双声道合并为单声道
        val monoShorts = if (srcChannels > 1) {
            val out = ShortArray(srcShorts.size / srcChannels)
            for (i in out.indices) {
                var sum = 0
                for (c in 0 until srcChannels) {
                    sum += srcShorts[i * srcChannels + c]
                }
                out[i] = (sum / srcChannels).toShort()
            }
            out
        } else {
            srcShorts
        }

        // 3. 重采样为 16000Hz
        if (srcSampleRate == 16000) {
            val outBuffer = ByteBuffer.allocate(monoShorts.size * 2).order(ByteOrder.LITTLE_ENDIAN)
            outBuffer.asShortBuffer().put(monoShorts)
            return outBuffer.array()
        }

        // 线性插值重采样
        val ratio = srcSampleRate.toDouble() / 16000.0
        val targetSize = (monoShorts.size / ratio).toInt()
        val destShorts = ShortArray(targetSize)

        for (i in 0 until targetSize) {
            val srcIndex = i * ratio
            val index = srcIndex.toInt()
            val fraction = srcIndex - index

            if (index >= monoShorts.size - 1) {
                destShorts[i] = monoShorts[monoShorts.size - 1]
            } else {
                val s0 = monoShorts[index].toInt()
                val s1 = monoShorts[index + 1].toInt()
                destShorts[i] = (s0 + fraction * (s1 - s0)).toInt().toShort()
            }
        }

        val outBuffer = ByteBuffer.allocate(destShorts.size * 2).order(ByteOrder.LITTLE_ENDIAN)
        outBuffer.asShortBuffer().put(destShorts)
        return outBuffer.array()
    }


    /**
     * 写入 ADTS 头部：AAC Profile = LC(2)，采样率 16000 (Index 8)，单声道 = 1
     */
    private fun addADTStoPacket(packet: ByteArray, packetLen: Int) {
        val profile = 2 // AAC LC
        val freqIdx = 8 // 16000Hz
        val chanCfg = 1 // Mono

        // fill in ADTS data
        packet[0] = 0xFF.toByte()
        packet[1] = 0xF9.toByte()
        packet[2] = (((profile - 1) shl 6) + (freqIdx shl 2) + (chanCfg shr 2)).toByte()
        packet[3] = (((chanCfg and 3) shl 6) + (packetLen shr 11)).toByte()
        packet[4] = ((packetLen and 0x7FF) shr 3).toByte()
        packet[5] = (((packetLen and 7) shl 5) + 0x1F).toByte()
        packet[6] = 0xFC.toByte()
    }

    // 快捷自定义异常
    class FileNotFoundException(message: String) : Exception(message)
}
