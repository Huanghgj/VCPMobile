package com.vcp.mobile.affect

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test

class WordPieceTokenizerTest {
    private val vocabulary = listOf(
        "[PAD]",
        "[UNK]",
        "[CLS]",
        "[SEP]",
        "我",
        "很",
        "生",
        "气",
        "play",
        "##ing",
    )

    @Test
    fun encodesChineseCharactersWithBertSpecialTokens() {
        val encoded = WordPieceTokenizer(vocabulary).encode("我很生气", maxLength = 8)
        assertArrayEquals(longArrayOf(2, 4, 5, 6, 7, 3, 0, 0), encoded.inputIds)
        assertArrayEquals(longArrayOf(1, 1, 1, 1, 1, 1, 0, 0), encoded.attentionMask)
        assertFalse(encoded.truncated)
    }

    @Test
    fun appliesWordPieceContinuation() {
        assertEquals(listOf("play", "##ing"), WordPieceTokenizer(vocabulary).tokenize("playing"))
    }
}
