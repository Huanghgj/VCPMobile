package com.vcp.mobile

import android.app.AlarmManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.os.Build

object LifecycleAlarmManager {
    private const val PREFS = "vcp_lifecycle_scheduler"
    private const val KEY_TRIGGER_AT = "trigger_at"
    private const val REQUEST_CODE = 0x4C494645

    fun canScheduleExact(context: Context): Boolean {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S) return true
        val alarmManager = context.getSystemService(Context.ALARM_SERVICE) as AlarmManager
        return alarmManager.canScheduleExactAlarms()
    }

    fun schedule(context: Context, triggerAtMs: Long): Boolean {
        val alarmManager = context.getSystemService(Context.ALARM_SERVICE) as AlarmManager
        val pendingIntent = createPendingIntent(context)
        val effectiveAt = triggerAtMs.coerceAtLeast(System.currentTimeMillis() + 1_000L)
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            .edit()
            .putLong(KEY_TRIGGER_AT, effectiveAt)
            .apply()
        return try {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
                if (canScheduleExact(context)) {
                    alarmManager.setExactAndAllowWhileIdle(
                        AlarmManager.RTC_WAKEUP,
                        effectiveAt,
                        pendingIntent,
                    )
                } else {
                    alarmManager.setAndAllowWhileIdle(
                        AlarmManager.RTC_WAKEUP,
                        effectiveAt,
                        pendingIntent,
                    )
                }
            } else {
                alarmManager.setExact(AlarmManager.RTC_WAKEUP, effectiveAt, pendingIntent)
            }
            true
        } catch (_: Exception) {
            false
        }
    }

    fun cancel(context: Context) {
        val alarmManager = context.getSystemService(Context.ALARM_SERVICE) as AlarmManager
        alarmManager.cancel(createPendingIntent(context))
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            .edit()
            .remove(KEY_TRIGGER_AT)
            .apply()
    }

    fun reschedulePersisted(context: Context) {
        val triggerAt = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            .getLong(KEY_TRIGGER_AT, 0L)
        if (triggerAt > 0L) {
            schedule(context, triggerAt.coerceAtLeast(System.currentTimeMillis() + 5_000L))
        }
    }

    fun clearPersisted(context: Context) {
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            .edit()
            .remove(KEY_TRIGGER_AT)
            .apply()
    }

    private fun createPendingIntent(context: Context): PendingIntent {
        val intent = Intent(context, LifecycleAlarmReceiver::class.java).apply {
            action = LifecycleAlarmReceiver.ACTION_WAKEUP
        }
        return PendingIntent.getBroadcast(
            context,
            REQUEST_CODE,
            intent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
    }
}
