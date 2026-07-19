package com.vcp.mobile.affect

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class AffectOutputMapperTest {
    @Test
    fun mapsModelLabelsIntoCanonicalScores() {
        val scores = AffectOutputMapper.mapLogits(
            floatArrayOf(0.0f, 4.0f, -1.0f),
            listOf("neutral", "anger", "疑问"),
        )
        assertEquals(AffectOutputMapper.CANONICAL_LABELS, scores.keys.toList())
        assertTrue((scores["anger"] ?: 0.0f) > 0.95f)
        assertEquals(1.0, scores.values.sum().toDouble(), 1e-5)
    }

    @Test
    fun extractsBatchedLogits() {
        val logits = AffectOutputMapper.extractLogits(arrayOf(floatArrayOf(1.0f, 2.0f)))
        assertEquals(listOf(1.0f, 2.0f), logits.toList())
    }
}
