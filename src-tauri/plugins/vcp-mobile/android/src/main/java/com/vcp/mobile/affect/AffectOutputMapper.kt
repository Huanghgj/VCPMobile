package com.vcp.mobile.affect

import kotlin.math.exp

/** Pure functions kept free of Android/ORT types for inexpensive JVM tests. */
object AffectOutputMapper {
    val CANONICAL_LABELS = listOf(
        "neutral",
        "joy",
        "sadness",
        "anger",
        "confusion",
        "disgust",
        "surprise",
        "affection",
    )

    fun softmax(logits: FloatArray): FloatArray {
        require(logits.isNotEmpty()) { "Model returned empty logits" }
        val maximum = logits.maxOrNull() ?: 0.0f
        val exponentials = DoubleArray(logits.size) { index -> exp((logits[index] - maximum).toDouble()) }
        val denominator = exponentials.sum()
        require(denominator.isFinite() && denominator > 0.0) { "Model returned invalid logits" }
        return FloatArray(logits.size) { index -> (exponentials[index] / denominator).toFloat() }
    }

    fun mapLogits(
        logits: FloatArray,
        modelLabels: List<String>,
        configuredAliases: Map<String, String> = emptyMap(),
    ): LinkedHashMap<String, Float> {
        require(logits.size == modelLabels.size) {
            "Model returned ${logits.size} logits for ${modelLabels.size} configured labels"
        }
        val probabilities = softmax(logits)
        val scores = linkedMapOf<String, Float>()
        CANONICAL_LABELS.forEach { scores[it] = 0.0f }

        modelLabels.forEachIndexed { index, rawLabel ->
            val normalized = normalizeLabel(rawLabel)
            val configured = configuredAliases[rawLabel]
                ?: configuredAliases[normalized]
                ?: DEFAULT_ALIASES[normalized]
            val canonical = configured?.let(::normalizeLabel)?.takeIf(CANONICAL_LABELS::contains)
            val target = canonical ?: "neutral"
            scores[target] = (scores[target] ?: 0.0f) + probabilities[index]
        }
        return scores
    }

    fun extractLogits(value: Any?): FloatArray = when (value) {
        is FloatArray -> value
        is Array<*> -> {
            require(value.isNotEmpty()) { "Model returned an empty output tensor" }
            extractLogits(value[0])
        }
        else -> error("Unsupported logits tensor type: ${value?.javaClass?.name ?: "null"}")
    }

    private fun normalizeLabel(label: String): String = label.trim().lowercase().replace('-', '_').replace(' ', '_')

    private val DEFAULT_ALIASES = mapOf(
        "neutral" to "neutral",
        "other" to "neutral",
        "中性" to "neutral",
        "无情绪" to "neutral",
        "joy" to "joy",
        "happy" to "joy",
        "happiness" to "joy",
        "喜悦" to "joy",
        "高兴" to "joy",
        "sad" to "sadness",
        "sadness" to "sadness",
        "悲伤" to "sadness",
        "难过" to "sadness",
        "anger" to "anger",
        "angry" to "anger",
        "愤怒" to "anger",
        "生气" to "anger",
        "confusion" to "confusion",
        "confused" to "confusion",
        "疑问" to "confusion",
        "困惑" to "confusion",
        "disgust" to "disgust",
        "厌恶" to "disgust",
        "恶心" to "disgust",
        "surprise" to "surprise",
        "surprised" to "surprise",
        "惊讶" to "surprise",
        "惊喜" to "surprise",
        "affection" to "affection",
        "love" to "affection",
        "loving" to "affection",
        "爱" to "affection",
        "喜欢" to "affection",
    )
}
