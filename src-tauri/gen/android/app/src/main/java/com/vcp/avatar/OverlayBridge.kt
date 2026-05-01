package com.vcp.avatar

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.provider.Settings
import androidx.core.content.ContextCompat

object OverlayBridge {
  private const val PREFS_NAME = "vcp_overlay"
  private const val PENDING_TITLE = "pending_title"
  private const val PENDING_HTML = "pending_html"

  @JvmStatic
  fun showSurface(activity: Activity, title: String, html: String) {
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M && !Settings.canDrawOverlays(activity)) {
      activity.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
        .edit()
        .putString(PENDING_TITLE, title)
        .putString(PENDING_HTML, html)
        .apply()

      val intent = Intent(
        Settings.ACTION_MANAGE_OVERLAY_PERMISSION,
        Uri.parse("package:${activity.packageName}"),
      )
      activity.startActivity(intent)
      return
    }

    val intent = Intent(activity, OverlayService::class.java).apply {
      action = OverlayService.ACTION_SHOW
      putExtra(OverlayService.EXTRA_TITLE, title)
      putExtra(OverlayService.EXTRA_HTML, html)
    }
    ContextCompat.startForegroundService(activity, intent)
  }

  @JvmStatic
  fun showPendingSurfaceIfPermitted(activity: Activity) {
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M && !Settings.canDrawOverlays(activity)) return

    val prefs = activity.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
    val html = prefs.getString(PENDING_HTML, null) ?: return
    val title = prefs.getString(PENDING_TITLE, "VCP Surface") ?: "VCP Surface"
    prefs.edit()
      .remove(PENDING_TITLE)
      .remove(PENDING_HTML)
      .apply()

    showSurface(activity, title, html)
  }
}
