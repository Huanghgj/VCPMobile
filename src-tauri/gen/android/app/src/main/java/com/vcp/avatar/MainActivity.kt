package com.vcp.avatar

import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts

class MainActivity : TauriActivity() {
  private val runtimePermissionLauncher =
    registerForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { results ->
      val denied = results.filterValues { granted -> !granted }.keys
      if (denied.isNotEmpty()) {
        android.util.Log.w("VCPMobile", "Runtime permissions denied: $denied")
      }
    }

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    requestInitialRuntimePermissions()
  }

  override fun onResume() {
    super.onResume()
    OverlayBridge.showPendingSurfaceIfPermitted(this)
  }

  private fun requestInitialRuntimePermissions() {
    val requiredPermissions = mutableListOf(
      Manifest.permission.ACCESS_COARSE_LOCATION,
      Manifest.permission.ACCESS_FINE_LOCATION,
    )

    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
      requiredPermissions.add(Manifest.permission.POST_NOTIFICATIONS)
    }

    val missingPermissions = requiredPermissions
      .filter { permission -> checkSelfPermission(permission) != PackageManager.PERMISSION_GRANTED }
      .toTypedArray()

    if (missingPermissions.isNotEmpty()) {
      runtimePermissionLauncher.launch(missingPermissions)
    }
  }
}
