package com.vcp.mobile.affect

import java.util.Locale

data class EncodedAffectText(
    val inputIds: LongArray,
    val attentionMask: LongArray,
    val tokenTypeIds: LongArray,
    val truncated: Boolean,
)

/** A small, dependency-free BERT BasicTokenizer + WordPiece implementation. */
class WordPieceTokenizer(
    vocabulary: List<String>,
    private val lowerCase: Boolean = true,
) {
    private val tokenToId = vocabulary.withIndex().associate { (index, token) -> token to index.toLong() }
    private val padId = requireToken("[PAD]")
    private val unknownId = requireToken("[UNK]")
    private val clsId = requireToken("[CLS]")
    private val separatorId = requireToken("[SEP]")

    fun encode(text: String, maxLength: Int = DEFAULT_MAX_LENGTH): EncodedAffectText {
        require(maxLength >= 2) { "maxLength must leave room for [CLS] and [SEP]" }

        val payloadLimit = maxLength - 2
        val pieces = mutableListOf<String>()
        var truncated = false

        for (token in basicTokenize(text)) {
            val tokenPieces = wordPiece(token)
            if (pieces.size + tokenPieces.size > payloadLimit) {
                val remaining = payloadLimit - pieces.size
                if (remaining > 0) pieces.addAll(tokenPieces.take(remaining))
                truncated = true
                break
            }
            pieces.addAll(tokenPieces)
        }

        val ids = LongArray(maxLength) { padId }
        val attention = LongArray(maxLength)
        val tokenTypes = LongArray(maxLength)
        var cursor = 0
        ids[cursor] = clsId
        attention[cursor++] = 1L
        for (piece in pieces) {
            ids[cursor] = tokenToId[piece] ?: unknownId
            attention[cursor++] = 1L
        }
        ids[cursor] = separatorId
        attention[cursor] = 1L

        return EncodedAffectText(ids, attention, tokenTypes, truncated)
    }

    /** Public and Android-free so JVM unit tests can verify token boundaries. */
    fun tokenize(text: String): List<String> = basicTokenize(text).flatMap(::wordPiece)

    private fun requireToken(token: String): Long =
        requireNotNull(tokenToId[token]) { "BERT vocabulary is missing required token $token" }

    private fun basicTokenize(text: String): List<String> {
        val normalized = if (lowerCase) text.lowercase(Locale.ROOT) else text
        val output = mutableListOf<String>()
        val current = StringBuilder()

        fun flush() {
            if (current.isNotEmpty()) {
                output += current.toString()
                current.setLength(0)
            }
        }

        var offset = 0
        while (offset < normalized.length) {
            val codePoint = normalized.codePointAt(offset)
            offset += Character.charCount(codePoint)
            when {
                isControl(codePoint) -> Unit
                Character.isWhitespace(codePoint) -> flush()
                isCjk(codePoint) || isPunctuation(codePoint) -> {
                    flush()
                    output += String(Character.toChars(codePoint))
                }
                else -> current.appendCodePoint(codePoint)
            }
        }
        flush()
        return output
    }

    private fun wordPiece(token: String): List<String> {
        if (tokenToId.containsKey(token)) return listOf(token)
        if (token.codePointCount(0, token.length) > MAX_INPUT_CHARS_PER_WORD) return listOf("[UNK]")

        val pieces = mutableListOf<String>()
        var start = 0
        while (start < token.length) {
            var end = token.length
            var match: String? = null
            while (end > start) {
                val raw = token.substring(start, end)
                val candidate = if (start == 0) raw else "##$raw"
                if (tokenToId.containsKey(candidate)) {
                    match = candidate
                    break
                }
                end = token.offsetByCodePoints(end, -1)
            }
            if (match == null) return listOf("[UNK]")
            pieces += match
            start = end
        }
        return pieces
    }

    private fun isControl(codePoint: Int): Boolean {
        if (codePoint == '\t'.code || codePoint == '\n'.code || codePoint == '\r'.code) return false
        return when (Character.getType(codePoint)) {
            Character.CONTROL.toInt(), Character.FORMAT.toInt() -> true
            else -> false
        }
    }

    private fun isPunctuation(codePoint: Int): Boolean {
        if (codePoint in 33..47 || codePoint in 58..64 || codePoint in 91..96 || codePoint in 123..126) {
            return true
        }
        return when (Character.getType(codePoint)) {
            Character.CONNECTOR_PUNCTUATION.toInt(),
            Character.DASH_PUNCTUATION.toInt(),
            Character.START_PUNCTUATION.toInt(),
            Character.END_PUNCTUATION.toInt(),
            Character.INITIAL_QUOTE_PUNCTUATION.toInt(),
            Character.FINAL_QUOTE_PUNCTUATION.toInt(),
            Character.OTHER_PUNCTUATION.toInt(), -> true
            else -> false
        }
    }

    private fun isCjk(codePoint: Int): Boolean =
        codePoint in 0x3400..0x4DBF ||
            codePoint in 0x4E00..0x9FFF ||
            codePoint in 0xF900..0xFAFF ||
            codePoint in 0x20000..0x2A6DF ||
            codePoint in 0x2A700..0x2B73F ||
            codePoint in 0x2B740..0x2B81F ||
            codePoint in 0x2B820..0x2CEAF ||
            codePoint in 0x2F800..0x2FA1F

    companion object {
        const val DEFAULT_MAX_LENGTH = 128
        private const val MAX_INPUT_CHARS_PER_WORD = 100
    }
}
