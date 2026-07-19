package com.vcp.mobile.affect

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.concurrent.TimeUnit
import java.util.concurrent.TimeoutException

class AffectDeadlineTest {
    @Test
    fun deadlineIncludesTimeAlreadySpentBeforeInference() {
        val submittedAt = 10_000L
        val deadline = AffectDeadline.afterMillis(1_200L, submittedAt)
        val halfway = submittedAt + TimeUnit.MILLISECONDS.toNanos(700L)

        assertEquals(TimeUnit.MILLISECONDS.toNanos(500L), deadline.remainingNanos(halfway))
        assertFalse(deadline.isExpired(halfway))
    }

    @Test
    fun expiredDeadlineFailsInsteadOfReturningLateResult() {
        val submittedAt = 20_000L
        val deadline = AffectDeadline.afterMillis(200L, submittedAt)
        val expiredAt = submittedAt + TimeUnit.MILLISECONDS.toNanos(200L)

        assertTrue(deadline.isExpired(expiredAt))
        assertThrows(TimeoutException::class.java) {
            deadline.throwIfExpired(expiredAt)
        }
    }

    @Test
    fun deadlineCreationSaturatesWithoutOverflow() {
        val deadline = AffectDeadline.afterMillis(Long.MAX_VALUE, Long.MAX_VALUE - 10L)

        assertEquals(10L, deadline.remainingNanos(Long.MAX_VALUE - 10L))
        assertTrue(deadline.isExpired(Long.MAX_VALUE))
    }
}
