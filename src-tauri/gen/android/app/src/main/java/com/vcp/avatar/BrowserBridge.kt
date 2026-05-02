package com.vcp.avatar

import android.app.Activity
import android.content.Intent
import android.os.Looper
import org.json.JSONObject
import java.lang.ref.WeakReference
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

object BrowserBridge {
  private var currentActivity = WeakReference<BrowserActivity?>(null)

  fun attach(activity: BrowserActivity) {
    currentActivity = WeakReference(activity)
  }

  fun detach(activity: BrowserActivity) {
    if (currentActivity.get() === activity) {
      currentActivity = WeakReference(null)
    }
  }

  @JvmStatic
  fun execute(activity: Activity, actionJson: String): String {
    return try {
      val action = JSONObject(actionJson.ifBlank { "{}" })
      val actionName = action.optString("action", action.optString("command", "")).lowercase()
      val isMainThread = Looper.myLooper() == Looper.getMainLooper()
      val browser = ensureBrowserActivity(activity, action, isMainThread)
        ?: return launchSnapshot(actionName, action.optString("url")).toString()

      if (isMainThread) {
        browser.handleAction(action) { }
        browser.currentSnapshot(actionName.ifBlank { "accepted" }).toString()
      } else {
        val latch = CountDownLatch(1)
        var result: JSONObject? = null
        activity.runOnUiThread {
          browser.handleAction(action) {
            result = it
            latch.countDown()
          }
        }
        if (!latch.await(10, TimeUnit.SECONDS)) {
          browser.currentSnapshot("timeout").apply {
            put("status", "timeout")
            put("text", "Browser action timed out while waiting for the Android WebView.")
          }.toString()
        } else {
          (result ?: browser.currentSnapshot(actionName)).toString()
        }
      }
    } catch (error: Throwable) {
      JSONObject()
        .put("status", "error")
        .put("controlMode", "ai")
        .put("url", JSONObject.NULL)
        .put("title", "MobileBrowser error")
        .put("text", error.message ?: error.toString())
        .put("links", org.json.JSONArray())
        .put("forms", org.json.JSONArray())
        .put("lastAction", "error")
        .put("waitingReason", JSONObject.NULL)
        .put("bridgeKind", "android-webview")
        .put("updatedAt", System.currentTimeMillis())
        .toString()
    }
  }

  private fun ensureBrowserActivity(
    activity: Activity,
    action: JSONObject,
    isMainThread: Boolean,
  ): BrowserActivity? {
    currentActivity.get()?.let { return it }

    val launch = {
      val intent = Intent(activity, BrowserActivity::class.java).apply {
        addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
        action.optString("url").takeIf { it.isNotBlank() }?.let { putExtra(BrowserActivity.EXTRA_URL, it) }
      }
      activity.startActivity(intent)
    }

    if (isMainThread) {
      launch()
      return null
    }

    val launchLatch = CountDownLatch(1)
    activity.runOnUiThread {
      launch()
      launchLatch.countDown()
    }
    launchLatch.await(1, TimeUnit.SECONDS)

    val startedAt = System.currentTimeMillis()
    while (System.currentTimeMillis() - startedAt < 2500) {
      currentActivity.get()?.let { return it }
      Thread.sleep(40)
    }

    throw IllegalStateException("BrowserActivity did not attach in time.")
  }

  private fun launchSnapshot(actionName: String, url: String): JSONObject {
    return JSONObject()
      .put("status", "launching")
      .put("controlMode", "ai")
      .put("url", url.takeIf { it.isNotBlank() } ?: JSONObject.NULL)
      .put("title", "Launching Mobile Browser")
      .put("text", "BrowserActivity is launching. Retry snapshot after the page opens.")
      .put("links", org.json.JSONArray())
      .put("forms", org.json.JSONArray())
      .put("lastAction", actionName.ifBlank { "launch" })
      .put("waitingReason", JSONObject.NULL)
      .put("bridgeKind", "android-webview")
      .put("updatedAt", System.currentTimeMillis())
  }
}
