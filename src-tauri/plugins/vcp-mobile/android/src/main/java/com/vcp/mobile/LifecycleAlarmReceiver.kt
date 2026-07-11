package com.vcp.mobile

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.PowerManager
import androidx.core.app.NotificationCompat

class LifecycleAlarmReceiver : BroadcastReceiver() {
    companion object {
        const val ACTION_WAKEUP = "com.vcp.mobile.LIFECYCLE_WAKEUP"
        private const val CHANNEL_ID = "vcp_lifecycle_wakeup"
        private const val NOTIFICATION_ID = 0x4C494645
    }

    override fun onReceive(context: Context, intent: Intent?) {
        if (intent?.action == Intent.ACTION_BOOT_COMPLETED ||
            intent?.action == Intent.ACTION_MY_PACKAGE_REPLACED
        ) {
            LifecycleAlarmManager.reschedulePersisted(context)
            com.vcp.mobile.service.StreamKeepaliveService.restoreRequestedKeepalive(context)
            return
        }
        if (intent?.action != ACTION_WAKEUP) return
        LifecycleAlarmManager.clearPersisted(context)
        val powerManager = context.getSystemService(Context.POWER_SERVICE) as PowerManager
        val wakeLock = powerManager.newWakeLock(
            PowerManager.PARTIAL_WAKE_LOCK,
            "VcpMobile::LifecycleAlarmWakeLock",
        ).apply {
            acquire(60_000L)
        }
        Handler(Looper.getMainLooper()).postDelayed({
            if (wakeLock.isHeld) wakeLock.release()
        }, 60_000L)
        val plugin = VcpMobilePlugin.getInstance()
        if (plugin != null) {
            plugin.emitLifecycleWakeup()
        } else {
            showWakeupNotification(context)
            if (wakeLock.isHeld) wakeLock.release()
        }
    }

    private fun showWakeupNotification(context: Context) {
        val notificationManager =
            context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            notificationManager.createNotificationChannel(
                NotificationChannel(
                    CHANNEL_ID,
                    "AI 生命周期任务",
                    NotificationManager.IMPORTANCE_HIGH,
                ).apply {
                    description = "提醒用户运行已到期的 AI 生命周期任务"
                },
            )
        }
        val launchIntent = context.packageManager.getLaunchIntentForPackage(context.packageName)
        val pendingIntent = launchIntent?.let {
            PendingIntent.getActivity(
                context,
                0,
                it.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP),
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
        }
        val notification = NotificationCompat.Builder(context, CHANNEL_ID)
            .setSmallIcon(context.applicationInfo.icon)
            .setContentTitle("VCP 有一项到期任务")
            .setContentText("点击打开应用并继续 AI 生命周期任务")
            .setAutoCancel(true)
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setContentIntent(pendingIntent)
            .build()
        notificationManager.notify(NOTIFICATION_ID, notification)
    }
}
