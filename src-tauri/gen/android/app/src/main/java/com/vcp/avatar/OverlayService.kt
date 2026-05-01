package com.vcp.avatar

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.graphics.Color
import android.graphics.PixelFormat
import android.graphics.drawable.GradientDrawable
import android.os.Build
import android.os.IBinder
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.view.WindowManager
import android.webkit.WebView
import android.widget.FrameLayout
import android.widget.TextView
import androidx.core.app.NotificationCompat
import kotlin.math.max
import kotlin.math.roundToInt

class OverlayService : Service() {
  companion object {
    const val ACTION_SHOW = "com.vcp.avatar.overlay.SHOW"
    const val ACTION_HIDE = "com.vcp.avatar.overlay.HIDE"
    const val EXTRA_TITLE = "title"
    const val EXTRA_HTML = "html"
    private const val CHANNEL_ID = "vcp_overlay"
    private const val NOTIFICATION_ID = 4301
  }

  private lateinit var windowManager: WindowManager
  private data class OverlayEntry(
    val root: View,
    val params: WindowManager.LayoutParams,
  )
  private val overlays = linkedMapOf<Int, OverlayEntry>()
  private var nextOverlayId = 1

  override fun onCreate() {
    super.onCreate()
    windowManager = getSystemService(Context.WINDOW_SERVICE) as WindowManager
    createNotificationChannel()
  }

  override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
    startForeground(NOTIFICATION_ID, buildNotification())
    when (intent?.action) {
      ACTION_HIDE -> {
        val overlayId = intent.getIntExtra("overlay_id", -1).takeIf { it >= 0 }
        hideOverlay(overlayId)
        stopSelf()
      }
      ACTION_SHOW -> showOverlay(
        intent.getStringExtra(EXTRA_TITLE).orEmpty().ifBlank { "VCP Surface" },
        intent.getStringExtra(EXTRA_HTML).orEmpty(),
      )
    }
    return START_STICKY
  }

  override fun onDestroy() {
    hideOverlay(null)
    super.onDestroy()
  }

  override fun onBind(intent: Intent?): IBinder? = null

  private fun showOverlay(title: String, html: String) {
    val overlayId = nextOverlayId++
    if (overlays.size >= 4) {
      hideOverlay(overlays.keys.firstOrNull())
    }

    val root = FrameLayout(this).apply {
      background = GradientDrawable(
        GradientDrawable.Orientation.TL_BR,
        intArrayOf(Color.argb(238, 18, 24, 38), Color.argb(232, 44, 31, 66)),
      ).apply {
        cornerRadius = 18 * resources.displayMetrics.density
        setStroke((1 * resources.displayMetrics.density).roundToInt(), Color.argb(42, 255, 255, 255))
      }
      elevation = 18 * resources.displayMetrics.density
    }
    val density = resources.displayMetrics.density
    val titleBarHeight = (36 * density).roundToInt()
    val resizeHandleSize = (34 * density).roundToInt()

    val titleBar = TextView(this).apply {
      text = "$title    ×"
      setTextColor(Color.WHITE)
      textSize = 12f
      setPadding((12 * density).roundToInt(), 0, (12 * density).roundToInt(), 0)
      gravity = Gravity.CENTER_VERTICAL
      setBackgroundColor(Color.argb(74, 0, 0, 0))
      setOnClickListener {
        hideOverlay(overlayId)
        if (overlays.isEmpty()) stopSelf()
      }
    }

    val webView = WebView(this).apply {
      settings.javaScriptEnabled = true
      settings.domStorageEnabled = true
      settings.javaScriptCanOpenWindowsAutomatically = false
      setBackgroundColor(Color.TRANSPARENT)
      isClickable = true
      isFocusable = true
      isFocusableInTouchMode = true
      loadDataWithBaseURL(
        null,
        wrapHtml(html),
        "text/html",
        "UTF-8",
        null,
      )
    }

    val resizeHandle = TextView(this).apply {
      text = "↘"
      setTextColor(Color.argb(220, 255, 255, 255))
      textSize = 16f
      gravity = Gravity.CENTER
      setBackgroundColor(Color.argb(46, 255, 255, 255))
    }

    root.addView(titleBar, FrameLayout.LayoutParams(
      FrameLayout.LayoutParams.MATCH_PARENT,
      titleBarHeight,
    ))
    root.addView(webView, FrameLayout.LayoutParams(
      FrameLayout.LayoutParams.MATCH_PARENT,
      FrameLayout.LayoutParams.MATCH_PARENT,
    ).apply {
      topMargin = titleBarHeight
    })
    root.addView(resizeHandle, FrameLayout.LayoutParams(
      resizeHandleSize,
      resizeHandleSize,
      Gravity.BOTTOM or Gravity.END,
    ))

    val windowType = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
      WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY
    } else {
      @Suppress("DEPRECATION")
      WindowManager.LayoutParams.TYPE_PHONE
    }

    val params = WindowManager.LayoutParams(
      (320 * density).roundToInt(),
      (260 * density).roundToInt(),
      windowType,
      WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or WindowManager.LayoutParams.FLAG_LAYOUT_NO_LIMITS,
      PixelFormat.TRANSLUCENT,
    ).apply {
      gravity = Gravity.TOP or Gravity.START
      val offset = (overlays.size % 5) * (18 * density).roundToInt()
      x = (16 * density).roundToInt() + offset
      y = (86 * density).roundToInt() + offset
    }

    attachDrag(titleBar, root, params)
    attachResize(resizeHandle, root, params)
    windowManager.addView(root, params)
    overlays[overlayId] = OverlayEntry(root, params)
  }

  private fun attachDrag(handle: View, root: View, params: WindowManager.LayoutParams) {
    var startX = 0
    var startY = 0
    var touchX = 0f
    var touchY = 0f
    var moved = false

    handle.setOnTouchListener { _, event ->
      when (event.actionMasked) {
        MotionEvent.ACTION_DOWN -> {
          startX = params.x
          startY = params.y
          touchX = event.rawX
          touchY = event.rawY
          moved = false
          true
        }
        MotionEvent.ACTION_MOVE -> {
          val dx = (event.rawX - touchX).roundToInt()
          val dy = (event.rawY - touchY).roundToInt()
          if (kotlin.math.abs(dx) > 4 || kotlin.math.abs(dy) > 4) moved = true
          params.x = startX + dx
          params.y = startY + dy
          windowManager.updateViewLayout(root, params)
          true
        }
        MotionEvent.ACTION_UP -> {
          if (!moved) handle.performClick()
          true
        }
        else -> false
      }
    }
  }

  private fun attachResize(handle: View, root: View, params: WindowManager.LayoutParams) {
    val density = resources.displayMetrics.density
    val minWidth = (220 * density).roundToInt()
    val minHeight = (180 * density).roundToInt()
    val edgePadding = (8 * density).roundToInt()
    var startWidth = 0
    var startHeight = 0
    var touchX = 0f
    var touchY = 0f

    handle.setOnTouchListener { _, event ->
      when (event.actionMasked) {
        MotionEvent.ACTION_DOWN -> {
          startWidth = params.width
          startHeight = params.height
          touchX = event.rawX
          touchY = event.rawY
          true
        }
        MotionEvent.ACTION_MOVE -> {
          val dx = (event.rawX - touchX).roundToInt()
          val dy = (event.rawY - touchY).roundToInt()
          val metrics = resources.displayMetrics
          val maxWidth = max(minWidth, metrics.widthPixels - params.x - edgePadding)
          val maxHeight = max(minHeight, metrics.heightPixels - params.y - edgePadding)
          params.width = (startWidth + dx).coerceIn(minWidth, maxWidth)
          params.height = (startHeight + dy).coerceIn(minHeight, maxHeight)
          windowManager.updateViewLayout(root, params)
          true
        }
        MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> true
        else -> false
      }
    }
  }

  private fun hideOverlay(overlayId: Int? = null) {
    val targets = if (overlayId == null) {
      overlays.keys.toList()
    } else {
      listOf(overlayId)
    }
    for (target in targets) {
      overlays.remove(target)?.let {
        runCatching { windowManager.removeView(it.root) }
      }
    }
  }

  private fun wrapHtml(content: String): String {
    return """
      <!doctype html>
      <html>
      <head>
        <meta name="viewport" content="width=device-width, initial-scale=1.0">
        <style>
          html, body { margin: 0; min-height: 100%; background: transparent; color: #f8fafc; font-family: sans-serif; }
          * { box-sizing: border-box; }
          img, video, canvas, svg { max-width: 100%; }
          #vcp-overlay-toast {
            position: fixed;
            left: 12px;
            right: 12px;
            bottom: 12px;
            z-index: 2147483647;
            padding: 10px 12px;
            border-radius: 12px;
            background: rgba(15, 23, 42, .94);
            color: #fff;
            font-size: 13px;
            line-height: 1.45;
            box-shadow: 0 12px 32px rgba(0,0,0,.32);
            opacity: 0;
            transform: translateY(8px);
            transition: opacity .18s ease, transform .18s ease;
            pointer-events: none;
          }
          #vcp-overlay-toast[data-show="true"] { opacity: 1; transform: translateY(0); }
        </style>
        <script>
          window.alert = function(message) {
            var toast = document.getElementById("vcp-overlay-toast");
            if (!toast) {
              toast = document.createElement("div");
              toast.id = "vcp-overlay-toast";
              document.body.appendChild(toast);
            }
            toast.textContent = String(message == null ? "" : message);
            toast.dataset.show = "true";
            window.clearTimeout(window.__vcpOverlayToastTimer);
            window.__vcpOverlayToastTimer = window.setTimeout(function() {
              toast.dataset.show = "false";
            }, 2200);
          };
        </script>
      </head>
      <body>$content</body>
      </html>
    """.trimIndent()
  }

  private fun createNotificationChannel() {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
    val channel = NotificationChannel(
      CHANNEL_ID,
      "VCP Surface",
      NotificationManager.IMPORTANCE_LOW,
    )
    getSystemService(NotificationManager::class.java).createNotificationChannel(channel)
  }

  private fun buildNotification() =
    NotificationCompat.Builder(this, CHANNEL_ID)
      .setSmallIcon(R.mipmap.ic_launcher)
      .setContentTitle("VCP Surface 已就绪")
      .setContentText("AI 调用桌面功能时显示系统悬浮窗")
      .setOngoing(true)
      .setContentIntent(PendingIntent.getActivity(
        this,
        0,
        Intent(this, MainActivity::class.java),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
      ))
      .addAction(
        0,
        "关闭",
        PendingIntent.getService(
          this,
          1,
          Intent(this, OverlayService::class.java).setAction(ACTION_HIDE),
          PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        ),
      )
      .build()
}
