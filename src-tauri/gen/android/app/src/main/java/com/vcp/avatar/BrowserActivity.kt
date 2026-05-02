package com.vcp.avatar

import android.annotation.SuppressLint
import android.app.Activity
import android.app.DownloadManager
import android.content.Context
import android.graphics.Bitmap
import android.graphics.Canvas
import android.net.Uri
import android.os.Bundle
import android.os.Environment
import android.util.Base64
import android.view.Gravity
import android.view.inputmethod.EditorInfo
import android.webkit.CookieManager
import android.webkit.DownloadListener
import android.webkit.ValueCallback
import android.webkit.WebChromeClient
import android.webkit.WebResourceRequest
import android.webkit.WebSettings
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.Toast
import org.json.JSONArray
import org.json.JSONObject
import org.json.JSONTokener
import java.io.ByteArrayOutputStream

class BrowserActivity : Activity() {
  companion object {
    const val EXTRA_URL = "url"
  }

  private lateinit var webView: WebView
  private lateinit var addressBar: EditText
  private var controlMode = "ai"
  private var waitingReason: String? = null
  private var lastAction = "init"
  private var lastSnapshot = baseSnapshot("init")

  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    BrowserBridge.attach(this)
    buildLayout()
    configureWebView()

    val initialUrl = intent.getStringExtra(EXTRA_URL)
    if (!initialUrl.isNullOrBlank()) {
      navigate(initialUrl)
    } else {
      webView.loadDataWithBaseURL(
        "https://vcp.local/",
        "<!doctype html><html><body style='font-family:sans-serif;padding:24px'><h2>VCPMobile Browser</h2><p>Waiting for an AI navigation command.</p></body></html>",
        "text/html",
        "UTF-8",
        null,
      )
    }
  }

  override fun onNewIntent(intent: android.content.Intent?) {
    super.onNewIntent(intent)
    intent?.getStringExtra(EXTRA_URL)?.takeIf { it.isNotBlank() }?.let { navigate(it) }
  }

  override fun onDestroy() {
    BrowserBridge.detach(this)
    super.onDestroy()
  }

  @SuppressLint("SetJavaScriptEnabled")
  private fun configureWebView() {
    WebView.setWebContentsDebuggingEnabled(true)
    CookieManager.getInstance().setAcceptCookie(true)
    CookieManager.getInstance().setAcceptThirdPartyCookies(webView, true)

    webView.settings.apply {
      javaScriptEnabled = true
      domStorageEnabled = true
      databaseEnabled = true
      loadsImagesAutomatically = true
      javaScriptCanOpenWindowsAutomatically = true
      mediaPlaybackRequiresUserGesture = false
      mixedContentMode = WebSettings.MIXED_CONTENT_COMPATIBILITY_MODE
      builtInZoomControls = true
      displayZoomControls = false
      useWideViewPort = true
      loadWithOverviewMode = true
    }

    webView.webChromeClient = WebChromeClient()
    webView.webViewClient = object : WebViewClient() {
      override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean {
        return false
      }

      override fun onPageFinished(view: WebView, url: String) {
        addressBar.setText(url)
        refreshSnapshot("page_finished") { }
      }
    }

    webView.setDownloadListener(DownloadListener { url, userAgent, contentDisposition, mimeType, _ ->
      runCatching {
        val request = DownloadManager.Request(Uri.parse(url))
          .setMimeType(mimeType)
          .addRequestHeader("User-Agent", userAgent)
          .setNotificationVisibility(DownloadManager.Request.VISIBILITY_VISIBLE_NOTIFY_COMPLETED)
          .setDestinationInExternalPublicDir(Environment.DIRECTORY_DOWNLOADS, Uri.parse(url).lastPathSegment ?: "download")
        (getSystemService(Context.DOWNLOAD_SERVICE) as DownloadManager).enqueue(request)
        Toast.makeText(this, "Download started", Toast.LENGTH_SHORT).show()
      }.onFailure {
        Toast.makeText(this, it.message ?: "Download failed", Toast.LENGTH_SHORT).show()
      }
    })
  }

  private fun buildLayout() {
    val root = LinearLayout(this).apply {
      orientation = LinearLayout.VERTICAL
      setBackgroundColor(0xff0f172a.toInt())
    }

    val toolbar = LinearLayout(this).apply {
      orientation = LinearLayout.HORIZONTAL
      gravity = Gravity.CENTER_VERTICAL
      setPadding(8.dp(), 8.dp(), 8.dp(), 8.dp())
    }

    val back = toolbarButton("<") { if (webView.canGoBack()) webView.goBack() }
    val forward = toolbarButton(">") { if (webView.canGoForward()) webView.goForward() }
    val reload = toolbarButton("R") { webView.reload() }
    val close = toolbarButton("X") { finish() }

    addressBar = EditText(this).apply {
      setSingleLine(true)
      imeOptions = EditorInfo.IME_ACTION_GO
      textSize = 14f
      setTextColor(0xfff8fafc.toInt())
      setHintTextColor(0xff94a3b8.toInt())
      hint = "https://"
      setBackgroundColor(0x22334155)
      setPadding(10.dp(), 0, 10.dp(), 0)
      setOnEditorActionListener { _, actionId, _ ->
        if (actionId == EditorInfo.IME_ACTION_GO) {
          navigate(text.toString())
          true
        } else {
          false
        }
      }
    }

    toolbar.addView(back)
    toolbar.addView(forward)
    toolbar.addView(reload)
    toolbar.addView(addressBar, LinearLayout.LayoutParams(0, 42.dp(), 1f))
    toolbar.addView(close)

    webView = WebView(this)
    root.addView(toolbar, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT))
    root.addView(webView, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, 0, 1f))

    val status = TextView(this).apply {
      text = "AI Browser Runtime"
      setTextColor(0xffcbd5e1.toInt())
      textSize = 11f
      setPadding(10.dp(), 4.dp(), 10.dp(), 8.dp())
    }
    root.addView(status)
    setContentView(root)
  }

  private fun toolbarButton(label: String, onClick: () -> Unit): Button {
    return Button(this).apply {
      text = label
      minWidth = 42.dp()
      minHeight = 42.dp()
      setOnClickListener { onClick() }
    }
  }

  fun handleAction(action: JSONObject, callback: (JSONObject) -> Unit) {
    val actionName = action.optString("action", action.optString("command", "")).lowercase()
    lastAction = actionName

    when (actionName) {
      "open", "navigate" -> {
        val url = action.optString("url")
        if (url.isBlank()) {
          callback(errorSnapshot("Missing url for navigate."))
        } else {
          controlMode = "ai"
          waitingReason = null
          navigate(url)
          callback(currentSnapshot("navigate"))
        }
      }
      "back" -> {
        if (webView.canGoBack()) webView.goBack()
        callback(currentSnapshot("back"))
      }
      "forward" -> {
        if (webView.canGoForward()) webView.goForward()
        callback(currentSnapshot("forward"))
      }
      "reload" -> {
        webView.reload()
        callback(currentSnapshot("reload"))
      }
      "snapshot", "read", "read_text", "read_dom" -> refreshSnapshot(actionName, callback)
      "screenshot" -> screenshotSnapshot(callback)
      "tap" -> runDomAction(tapScript(action), "tap", callback)
      "click" -> runDomAction(clickScript(action), "click", callback)
      "type" -> runDomAction(typeScript(action), "type", callback)
      "scroll" -> runDomAction(scrollScript(action), "scroll", callback)
      "eval", "evaluate" -> runEval(action.optString("script"), callback)
      "handoff", "handoff_to_user", "wait_for_user" -> {
        controlMode = "user"
        waitingReason = action.optString("reason").takeIf { it.isNotBlank() } ?: "AI requested manual browser assistance."
        Toast.makeText(this, waitingReason, Toast.LENGTH_LONG).show()
        callback(currentSnapshot("handoff"))
      }
      "resume", "resume_ai" -> {
        controlMode = "ai"
        waitingReason = null
        callback(currentSnapshot("resume"))
      }
      else -> callback(errorSnapshot("Unsupported browser action: $actionName"))
    }
  }

  fun currentSnapshot(action: String = lastAction): JSONObject {
    return lastSnapshot
      .put("status", if (waitingReason == null) "idle" else "waiting_for_user")
      .put("controlMode", controlMode)
      .put("url", webView.url ?: JSONObject.NULL)
      .put("title", webView.title ?: JSONObject.NULL)
      .put("lastAction", action)
      .put("waitingReason", waitingReason ?: JSONObject.NULL)
      .put("bridgeKind", "android-webview")
      .put("updatedAt", System.currentTimeMillis())
  }

  private fun navigate(rawUrl: String) {
    val url = normalizeUrl(rawUrl)
    addressBar.setText(url)
    webView.loadUrl(url)
  }

  private fun refreshSnapshot(action: String, callback: (JSONObject) -> Unit) {
    webView.evaluateJavascript(snapshotScript()) { result ->
      val snapshot = parseSnapshotResult(result)
      lastSnapshot = snapshot
      callback(currentSnapshot(action))
    }
  }

  private fun runDomAction(script: String, action: String, callback: (JSONObject) -> Unit) {
    webView.evaluateJavascript(script) {
      refreshSnapshot(action, callback)
    }
  }

  private fun runEval(script: String, callback: (JSONObject) -> Unit) {
    if (script.isBlank()) {
      callback(errorSnapshot("Missing script for eval."))
      return
    }
    webView.evaluateJavascript("(function(){ return (function(){ $script })(); })();") { result ->
      refreshSnapshot("eval") { snapshot ->
        snapshot.put("text", "Eval result: ${decodeJsValue(result)}\n\n${snapshot.optString("text")}")
        callback(snapshot)
      }
    }
  }

  private fun screenshotSnapshot(callback: (JSONObject) -> Unit) {
    refreshSnapshot("screenshot") { snapshot ->
      runCatching {
        val width = webView.width.coerceAtLeast(1)
        val height = webView.height.coerceAtLeast(1)
        val bitmap = Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888)
        val canvas = Canvas(bitmap)
        webView.draw(canvas)
        val output = ByteArrayOutputStream()
        bitmap.compress(Bitmap.CompressFormat.PNG, 100, output)
        bitmap.recycle()
        val encoded = Base64.encodeToString(output.toByteArray(), Base64.NO_WRAP)
        snapshot.put("screenshot", "data:image/png;base64,$encoded")
      }.onFailure {
        snapshot.put("text", "Screenshot failed: ${it.message}\n\n${snapshot.optString("text")}")
      }
      callback(snapshot)
    }
  }

  private fun parseSnapshotResult(result: String?): JSONObject {
    return runCatching {
      val decoded = decodeJsValue(result)
      JSONObject(decoded)
    }.getOrElse {
      baseSnapshot("snapshot_error").put("text", it.message ?: it.toString())
    }
  }

  private fun decodeJsValue(raw: String?): String {
    if (raw.isNullOrBlank() || raw == "null") return ""
    val value = JSONTokener(raw).nextValue()
    return when (value) {
      is String -> value
      JSONObject.NULL -> ""
      else -> value.toString()
    }
  }

  private fun snapshotScript(): String {
    return """
      (function() {
        function textOf(node) {
          return String((node.innerText || node.textContent || node.value || node.getAttribute('aria-label') || '')).trim();
        }
        var links = Array.prototype.slice.call(document.links || [], 0, 80).map(function(link) {
          return { text: textOf(link).slice(0, 160), href: link.href || '' };
        });
        var forms = Array.prototype.slice.call(document.forms || [], 0, 20).map(function(form) {
          return {
            id: form.id || null,
            name: form.name || null,
            action: form.action || null,
            method: form.method || null,
            fields: Array.prototype.slice.call(form.elements || [], 0, 80).map(function(field) {
              return field.name || field.id || field.type || field.tagName;
            }).filter(Boolean)
          };
        });
        return JSON.stringify({
          status: 'idle',
          controlMode: 'ai',
          url: location.href,
          title: document.title,
          text: String(document.body && document.body.innerText || '').slice(0, 12000),
          links: links,
          forms: forms,
          lastAction: 'snapshot',
          waitingReason: null,
          bridgeKind: 'android-webview',
          updatedAt: Date.now()
        });
      })();
    """.trimIndent()
  }

  private fun clickScript(action: JSONObject): String {
    val selector = jsString(action.optString("selector"))
    val text = jsString(action.optString("text"))
    return """
      (function() {
        var selector = $selector;
        var text = $text;
        var target = selector ? document.querySelector(selector) : null;
        if (!target && text) {
          var needle = text.toLowerCase();
          var nodes = Array.prototype.slice.call(document.querySelectorAll('a,button,input,textarea,select,[role="button"],[onclick],label'));
          target = nodes.find(function(node) {
            var value = String(node.innerText || node.textContent || node.value || node.getAttribute('aria-label') || '').trim().toLowerCase();
            return value.indexOf(needle) >= 0;
          });
        }
        if (!target) return false;
        target.scrollIntoView({ block: 'center', inline: 'center' });
        target.click();
        return true;
      })();
    """.trimIndent()
  }

  private fun typeScript(action: JSONObject): String {
    val selector = jsString(action.optString("selector"))
    val value = jsString(action.optString("value", action.optString("text")))
    return """
      (function() {
        var target = document.querySelector($selector);
        if (!target) return false;
        target.focus();
        target.value = $value;
        target.dispatchEvent(new InputEvent('input', { bubbles: true, inputType: 'insertText', data: $value }));
        target.dispatchEvent(new Event('change', { bubbles: true }));
        return true;
      })();
    """.trimIndent()
  }

  private fun tapScript(action: JSONObject): String {
    val x = action.optDouble("x", -1.0)
    val y = action.optDouble("y", -1.0)
    return """
      (function() {
        var target = document.elementFromPoint($x, $y);
        if (!target) return false;
        target.scrollIntoView({ block: 'center', inline: 'center' });
        target.click();
        return true;
      })();
    """.trimIndent()
  }

  private fun scrollScript(action: JSONObject): String {
    val direction = action.optString("direction", "down").lowercase()
    val amount = action.optInt("amount", 720)
    val dx = when (direction) {
      "left" -> -amount
      "right" -> amount
      else -> 0
    }
    val dy = when (direction) {
      "up" -> -amount
      "down" -> amount
      else -> 0
    }
    return "window.scrollBy({ left: $dx, top: $dy, behavior: 'smooth' }); true;"
  }

  private fun errorSnapshot(message: String): JSONObject {
    return currentSnapshot("error")
      .put("status", "error")
      .put("text", message)
  }

  private fun baseSnapshot(action: String): JSONObject {
    return JSONObject()
      .put("status", "idle")
      .put("controlMode", controlMode)
      .put("url", JSONObject.NULL)
      .put("title", JSONObject.NULL)
      .put("text", "")
      .put("links", JSONArray())
      .put("forms", JSONArray())
      .put("lastAction", action)
      .put("waitingReason", JSONObject.NULL)
      .put("bridgeKind", "android-webview")
      .put("updatedAt", System.currentTimeMillis())
  }

  private fun normalizeUrl(raw: String): String {
    val trimmed = raw.trim()
    if (trimmed.startsWith("http://") || trimmed.startsWith("https://")) return trimmed
    return "https://$trimmed"
  }

  private fun jsString(value: String): String = JSONObject.quote(value)

  private fun Int.dp(): Int = (this * resources.displayMetrics.density).toInt()
}
