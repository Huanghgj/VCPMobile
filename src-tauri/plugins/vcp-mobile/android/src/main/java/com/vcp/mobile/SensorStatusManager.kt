package com.vcp.mobile

import android.content.Context
import android.hardware.Sensor
import android.hardware.SensorEvent
import android.hardware.SensorEventListener
import android.hardware.SensorManager
import android.location.Location
import android.location.LocationListener
import android.location.LocationManager
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.util.Log
import androidx.core.content.ContextCompat
import app.tauri.plugin.JSObject
import java.util.Locale
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import kotlin.math.sqrt

class SensorStatusManager(private val context: Context) {
    companion object {
        private const val TAG = "SensorStatusManager"
        private const val BURST_ACTIVE_DURATION = 2000L // 2s sampling
        private const val BURST_SLEEP_DURATION = 28000L // 28s sleep
        private const val ON_DEMAND_AMBIENT_DURATION = 1200L
        private const val ON_DEMAND_LOCATION_DURATION = 2500L
        private const val LOCATION_CACHE_TTL = 120000L
        private const val MOTION_CACHE_TTL = 30000L
        private const val AMBIENT_CACHE_TTL = 60000L
        private const val SAMPLING_PERIOD_US = 100000 // 100ms = 10Hz
    }

    private val sensorManager = context.getSystemService(Context.SENSOR_SERVICE) as SensorManager
    private val locationManager = context.getSystemService(Context.LOCATION_SERVICE) as LocationManager

    // Cached values (thread-safe updates)
    @Volatile private var latestLocationStr = "位置信息: 等待数据采集..."
    @Volatile private var latestMotionStr = "运动状态: 静止"
    @Volatile private var latestAmbientStr = "环境传感器: 设备不支持或权限未授予"

    @Volatile private var isRunning = false
    private val mainHandler = Handler(Looper.getMainLooper())
    @Volatile private var isMotionBurstActive = false
    @Volatile private var isAmbientSnapshotActive = false
    @Volatile private var isLocationSnapshotActive = false
    private var motionStopRunnable: Runnable? = null
    private var motionScheduleRunnable: Runnable? = null
    private var ambientStopRunnable: Runnable? = null
    private var locationStopRunnable: Runnable? = null
    private var motionSnapshotLatch: CountDownLatch? = null
    private var ambientSnapshotLatch: CountDownLatch? = null
    private var locationSnapshotLatch: CountDownLatch? = null
    @Volatile private var snapshotGeneration = 0L
    @Volatile private var lastLocationRefreshAt = 0L
    @Volatile private var lastMotionRequestAt = 0L
    @Volatile private var lastAmbientRequestAt = 0L

    // Sensor instances
    private val accelerometer = sensorManager.getDefaultSensor(Sensor.TYPE_ACCELEROMETER)
    private val gyroscope = sensorManager.getDefaultSensor(Sensor.TYPE_GYROSCOPE)
    private val magneticField = sensorManager.getDefaultSensor(Sensor.TYPE_MAGNETIC_FIELD)
    private val lightSensor = sensorManager.getDefaultSensor(Sensor.TYPE_LIGHT)
    private val pressureSensor = sensorManager.getDefaultSensor(Sensor.TYPE_PRESSURE)

    // Temporary storage for burst sampling
    private val burstAccelSamples = ArrayList<Double>()
    private val burstGyroSamples = ArrayList<Double>()
    private val burstMagSamples = ArrayList<Double>()

    // Motion Sensor Listener for Burst
    private val motionListener = object : SensorEventListener {
        private var lastAccelTime = 0L
        private var lastGyroTime = 0L
        private var lastMagTime = 0L

        override fun onSensorChanged(event: SensorEvent?) {
            if (event == null) return
            val now = System.currentTimeMillis()
            when (event.sensor.type) {
                Sensor.TYPE_ACCELEROMETER -> {
                    if (now - lastAccelTime < 100) return
                    lastAccelTime = now
                    val x = event.values[0]
                    val y = event.values[1]
                    val z = event.values[2]
                    val magnitude = sqrt((x * x + y * y + z * z).toDouble())
                    synchronized(burstAccelSamples) {
                        burstAccelSamples.add(magnitude)
                    }
                }
                Sensor.TYPE_GYROSCOPE -> {
                    if (now - lastGyroTime < 100) return
                    lastGyroTime = now
                    val x = event.values[0]
                    val y = event.values[1]
                    val z = event.values[2]
                    val magnitude = sqrt((x * x + y * y + z * z).toDouble())
                    synchronized(burstGyroSamples) {
                        burstGyroSamples.add(magnitude)
                    }
                }
                Sensor.TYPE_MAGNETIC_FIELD -> {
                    if (now - lastMagTime < 100) return
                    lastMagTime = now
                    val x = event.values[0]
                    val y = event.values[1]
                    val z = event.values[2]
                    val magnitude = sqrt((x * x + y * y + z * z).toDouble())
                    synchronized(burstMagSamples) {
                        burstMagSamples.add(magnitude)
                    }
                }
            }
        }
        override fun onAccuracyChanged(sensor: Sensor?, accuracy: Int) {}
    }

    // Ambient sensors (Light and Pressure) listener
    private var lastLux = -1.0
    private var lastPressure = -1.0

    private val ambientListener = object : SensorEventListener {
        override fun onSensorChanged(event: SensorEvent?) {
            if (event == null) return
            if (event.sensor.type == Sensor.TYPE_LIGHT) {
                lastLux = event.values[0].toDouble()
                updateAmbientString()
            } else if (event.sensor.type == Sensor.TYPE_PRESSURE) {
                lastPressure = event.values[0].toDouble()
                updateAmbientString()
            }
        }
        override fun onAccuracyChanged(sensor: Sensor?, accuracy: Int) {}
    }

    // Location Listener
    private val locationListener = object : LocationListener {
        override fun onLocationChanged(location: Location) {
            updateLocationString(location)
        }
        @Deprecated("Deprecated in Java")
        override fun onStatusChanged(provider: String?, status: Int, extras: Bundle?) {}
        override fun onProviderEnabled(provider: String) {}
        override fun onProviderDisabled(provider: String) {}
    }

    @Synchronized
    fun start() {
        if (isRunning) return
        val generation = ++snapshotGeneration
        Log.i(TAG, "Priming SensorStatusManager one-shot snapshots")

        // 1. Warm cached location only; do not keep GNSS/network listeners active.
        refreshLocationSnapshot(force = true)

        // 2. Keep this compatibility entry point battery-friendly. Distributed tools
        // request fresh motion/ambient snapshots on demand instead of keeping timers alive.
        requestMotionSnapshot(generation)
        requestAmbientSnapshot(generation)
    }

    @Synchronized
    fun stop() {
        snapshotGeneration++
        val hasPendingSnapshot =
            motionSnapshotLatch != null || ambientSnapshotLatch != null || locationSnapshotLatch != null
        val hasScheduledStop =
            motionStopRunnable != null || motionScheduleRunnable != null ||
                ambientStopRunnable != null || locationStopRunnable != null
        if (
            !isRunning &&
            !isMotionBurstActive &&
            !isAmbientSnapshotActive &&
            !isLocationSnapshotActive &&
            !hasPendingSnapshot &&
            !hasScheduledStop
        ) return
        isRunning = false
        Log.i(TAG, "Stopping SensorStatusManager collection services")

        // Unregister location
        try {
            locationManager.removeUpdates(locationListener)
        } catch (e: SecurityException) {
            Log.e(TAG, "Failed to remove location updates", e)
        }

        // Unregister all sensors
        sensorManager.unregisterListener(ambientListener)
        sensorManager.unregisterListener(motionListener)
        isMotionBurstActive = false
        isAmbientSnapshotActive = false
        isLocationSnapshotActive = false

        motionStopRunnable?.let { mainHandler.removeCallbacks(it) }
        motionScheduleRunnable?.let { mainHandler.removeCallbacks(it) }
        ambientStopRunnable?.let { mainHandler.removeCallbacks(it) }
        locationStopRunnable?.let { mainHandler.removeCallbacks(it) }
        motionStopRunnable = null
        motionScheduleRunnable = null
        ambientStopRunnable = null
        locationStopRunnable = null
        motionSnapshotLatch?.countDown()
        ambientSnapshotLatch?.countDown()
        locationSnapshotLatch?.countDown()
        motionSnapshotLatch = null
        ambientSnapshotLatch = null
        locationSnapshotLatch = null
    }

    @Synchronized
    fun getSensorData(type: String): JSObject {
        prepareOnDemandSnapshot(type)
        val motionLatch = motionSnapshotLatch
        val ambientLatch = ambientSnapshotLatch
        val locationLatch = locationSnapshotLatch
        waitForRequestedSnapshots(type, motionLatch, ambientLatch, locationLatch)
        val obj = JSObject()
        when (type) {
            "location" -> obj.put("value", latestLocationStr)
            "motion" -> obj.put("value", latestMotionStr)
            "ambient" -> obj.put("value", latestAmbientStr)
            "all" -> {
                obj.put("location", latestLocationStr)
                obj.put("motion", latestMotionStr)
                obj.put("ambient", latestAmbientStr)
            }
            else -> obj.put("value", "未知传感器类型: $type")
        }
        return obj
    }

    private fun prepareOnDemandSnapshot(type: String) {
        val generation = snapshotGeneration
        when (type) {
            "location" -> requestLocationSnapshot(generation)
            "motion" -> requestMotionSnapshot(generation)
            "ambient" -> requestAmbientSnapshot(generation)
            "all" -> {
                requestLocationSnapshot(generation)
                requestMotionSnapshot(generation)
                requestAmbientSnapshot(generation)
            }
        }
    }

    // ==================================================================
    // Location Helpers
    // ==================================================================
    private fun refreshLocationSnapshot(force: Boolean = false) {
        val now = System.currentTimeMillis()
        if (!force && now - lastLocationRefreshAt < LOCATION_CACHE_TTL) return
        lastLocationRefreshAt = now

        val hasFine = ContextCompat.checkSelfPermission(context, android.Manifest.permission.ACCESS_FINE_LOCATION) == android.content.pm.PackageManager.PERMISSION_GRANTED
        val hasCoarse = ContextCompat.checkSelfPermission(context, android.Manifest.permission.ACCESS_COARSE_LOCATION) == android.content.pm.PackageManager.PERMISSION_GRANTED

        if (!hasFine && !hasCoarse) {
            latestLocationStr = "位置信息: 未获得定位权限"
            return
        }

        try {
            val providers = mutableListOf<String>()
            if (hasFine || hasCoarse) {
                providers.add(LocationManager.NETWORK_PROVIDER)
            }
            if (hasFine) {
                providers.add(LocationManager.GPS_PROVIDER)
            }
            val bestLocation = providers
                .filter { locationManager.isProviderEnabled(it) }
                .mapNotNull { locationManager.getLastKnownLocation(it) }
                .maxByOrNull { it.time }

            if (bestLocation != null) {
                updateLocationString(bestLocation)
            } else if (latestLocationStr.startsWith("位置信息: 等待")) {
                latestLocationStr = "位置信息: 暂无系统缓存定位"
            }
        } catch (e: SecurityException) {
            latestLocationStr = "位置信息: 获取异常 (${e.message})"
            Log.e(TAG, "SecurityException reading last known location", e)
        } catch (e: Exception) {
            latestLocationStr = "位置信息: 未开启定位服务"
            Log.e(TAG, "Exception reading last known location", e)
        }
    }

    private fun requestLocationSnapshot(generation: Long = snapshotGeneration) {
        val now = System.currentTimeMillis()
        if (isLocationSnapshotActive || now - lastLocationRefreshAt < LOCATION_CACHE_TTL) {
            refreshLocationSnapshot()
            return
        }
        lastLocationRefreshAt = now
        val latch = CountDownLatch(1)
        locationSnapshotLatch = latch
        refreshLocationSnapshot(force = true)

        mainHandler.post {
            if (generation != snapshotGeneration) {
                latch.countDown()
                if (locationSnapshotLatch === latch) {
                    locationSnapshotLatch = null
                }
                return@post
            }
            if (isLocationSnapshotActive) {
                latch.countDown()
                return@post
            }
            if (!registerLocationListeners()) {
                latch.countDown()
                return@post
            }
            isLocationSnapshotActive = true
            val stopRunnable = Runnable {
                if (generation != snapshotGeneration && !isLocationSnapshotActive) {
                    latch.countDown()
                    return@Runnable
                }
                try {
                    locationManager.removeUpdates(locationListener)
                } catch (e: SecurityException) {
                    Log.e(TAG, "Failed to remove location snapshot updates", e)
                }
                isLocationSnapshotActive = false
                latch.countDown()
                if (locationSnapshotLatch === latch) {
                    locationSnapshotLatch = null
                }
            }
            locationStopRunnable = stopRunnable
            mainHandler.postDelayed(stopRunnable, ON_DEMAND_LOCATION_DURATION)
        }
    }

    private fun registerLocationListeners(): Boolean {
        val hasFine = ContextCompat.checkSelfPermission(context, android.Manifest.permission.ACCESS_FINE_LOCATION) == android.content.pm.PackageManager.PERMISSION_GRANTED
        val hasCoarse = ContextCompat.checkSelfPermission(context, android.Manifest.permission.ACCESS_COARSE_LOCATION) == android.content.pm.PackageManager.PERMISSION_GRANTED

        if (!hasFine && !hasCoarse) {
            latestLocationStr = "位置信息: 未获得定位权限"
            Log.w(TAG, "Location permissions not granted.")
            return false
        }

        var registered = false
        try {
            // Register for network provider
            if (locationManager.isProviderEnabled(LocationManager.NETWORK_PROVIDER)) {
                locationManager.requestLocationUpdates(
                    LocationManager.NETWORK_PROVIDER,
                    120000L, // 120s
                    25f,
                    locationListener,
                    Looper.getMainLooper()
                )
                registered = true
                val lastKnown = locationManager.getLastKnownLocation(LocationManager.NETWORK_PROVIDER)
                if (lastKnown != null) {
                    updateLocationString(lastKnown)
                }
            }

            // Register for GPS provider only when fine location is granted.
            if (hasFine && locationManager.isProviderEnabled(LocationManager.GPS_PROVIDER)) {
                locationManager.requestLocationUpdates(
                    LocationManager.GPS_PROVIDER,
                    120000L, // 120s
                    25f,
                    locationListener,
                    Looper.getMainLooper()
                )
                registered = true
                val lastKnown = locationManager.getLastKnownLocation(LocationManager.GPS_PROVIDER)
                if (lastKnown != null) {
                    updateLocationString(lastKnown)
                }
            }
        } catch (e: SecurityException) {
            latestLocationStr = "位置信息: 获取异常 (${e.message})"
            Log.e(TAG, "SecurityException registering location updates", e)
            return false
        } catch (e: Exception) {
            latestLocationStr = "位置信息: 未开启定位服务"
            Log.e(TAG, "Exception registering location updates", e)
            return false
        }
        return registered
    }

    private fun updateLocationString(loc: Location) {
        val latitude = loc.latitude
        val longitude = loc.longitude
        val accuracy = loc.accuracy
        val altitude = loc.altitude

        val latDir = if (latitude >= 0) "N" else "S"
        val lonDir = if (longitude >= 0) "E" else "W"
        val lat = Math.abs(latitude)
        val lon = Math.abs(longitude)

        val accStr = if (accuracy > 0) "${Math.round(accuracy)}m" else "N/A"
        val altStr = if (loc.hasAltitude()) "${Math.round(altitude)}m" else "N/A"

        latestLocationStr = String.format(
            Locale.US,
            "坐标: %.4f°%s, %.4f°%s | 精度: %s | 海拔: %s",
            lat, latDir, lon, lonDir, accStr, altStr
        )
    }

    // ==================================================================
    // Motion Burst Sampling Helpers
    // ==================================================================
    private fun requestMotionSnapshot(generation: Long = snapshotGeneration) {
        val now = System.currentTimeMillis()
        if (isRunning || isMotionBurstActive || now - lastMotionRequestAt < MOTION_CACHE_TTL) return
        lastMotionRequestAt = now
        val latch = CountDownLatch(1)
        motionSnapshotLatch = latch

        mainHandler.post {
            if (generation != snapshotGeneration) {
                latch.countDown()
                if (motionSnapshotLatch === latch) {
                    motionSnapshotLatch = null
                }
                return@post
            }
            if (!isRunning && !isMotionBurstActive) {
                startMotionBurst(scheduleContinuous = false, generation = generation, latch = latch)
            } else {
                latch.countDown()
            }
        }
    }

    private fun scheduleNextMotionBurst() {
        if (!isRunning) return

        val runnable = Runnable {
            if (!isRunning) return@Runnable
            startMotionBurst(scheduleContinuous = true, generation = snapshotGeneration)
        }
        motionScheduleRunnable = runnable
        mainHandler.post(runnable)
    }

    private fun startMotionBurst(scheduleContinuous: Boolean, generation: Long = snapshotGeneration, latch: CountDownLatch? = motionSnapshotLatch) {
        if (generation != snapshotGeneration) {
            if (!scheduleContinuous) {
                latch?.countDown()
            }
            return
        }
        if (isMotionBurstActive) return
        if (accelerometer == null) {
            latestMotionStr = "运动状态: 设备无重力传感器"
            if (!scheduleContinuous) {
                latch?.countDown()
            }
            return
        }
        isMotionBurstActive = true

        synchronized(burstAccelSamples) { burstAccelSamples.clear() }
        synchronized(burstGyroSamples) { burstGyroSamples.clear() }
        synchronized(burstMagSamples) { burstMagSamples.clear() }

        sensorManager.registerListener(motionListener, accelerometer, SAMPLING_PERIOD_US)
        if (gyroscope != null) {
            sensorManager.registerListener(motionListener, gyroscope, SAMPLING_PERIOD_US)
        }
        if (magneticField != null) {
            sensorManager.registerListener(motionListener, magneticField, SAMPLING_PERIOD_US)
        }

        val stopRunnable = Runnable {
            if (generation != snapshotGeneration && !isMotionBurstActive) {
                latch?.countDown()
                return@Runnable
            }
            sensorManager.unregisterListener(motionListener)
            isMotionBurstActive = false
            processMotionBurstData()

            if (scheduleContinuous && isRunning) {
                val nextRunnable = Runnable { scheduleNextMotionBurst() }
                motionScheduleRunnable = nextRunnable
                mainHandler.postDelayed(nextRunnable, BURST_SLEEP_DURATION)
            } else {
                latch?.countDown()
                if (motionSnapshotLatch === latch) {
                    motionSnapshotLatch = null
                }
            }
        }
        motionStopRunnable = stopRunnable
        mainHandler.postDelayed(stopRunnable, BURST_ACTIVE_DURATION)
    }

    private fun processMotionBurstData() {
        val accelList = synchronized(burstAccelSamples) { ArrayList(burstAccelSamples) }
        val gyroList = synchronized(burstGyroSamples) { ArrayList(burstGyroSamples) }
        val magList = synchronized(burstMagSamples) { ArrayList(burstMagSamples) }

        if (accelList.isEmpty()) return

        val accelAvg = accelList.average()
        val accelMax = accelList.maxOrNull() ?: 0.0

        val gyroAvg = if (gyroList.isNotEmpty()) gyroList.average() else 0.0
        val gyroMax = if (gyroList.isNotEmpty()) gyroList.maxOrNull() ?: 0.0 else 0.0

        val magAvg = if (magList.isNotEmpty()) magList.average() else 0.0

        var state = "静止"
        if (accelAvg > 12.0 || gyroAvg > 1.5) {
            state = "运动中"
        } else if (accelAvg > 10.5 || gyroAvg > 0.5) {
            state = "步行中"
        } else if (accelAvg > 9.5 || gyroAvg > 0.1) {
            state = "轻微移动"
        }

        val gyroStr = if (gyroscope != null) {
            String.format(Locale.US, " | 旋转角速度: %.2frad/s (峰值: %.2f)", gyroAvg, gyroMax)
        } else {
            " | 旋转角速度: 设备不支持"
        }

        val magStr = if (magneticField != null) {
            String.format(Locale.US, " | 磁场强度: %.1fμT", magAvg)
        } else {
            " | 磁场强度: 设备不支持"
        }

        val briefStr = String.format(Locale.US, "状态: %s", state)
        val detailStr = String.format(
            Locale.US,
            "状态: %s | 平均加速度: %.2fm/s² (峰值: %.2fm/s²)%s%s",
            state, accelAvg, accelMax, gyroStr, magStr
        )
        latestMotionStr = String.format(
            Locale.US,
            "[===vcp_fold: 0.0 ::desc: 物理运动姿态粗略状态(静止、步行、步行中或剧烈移动)===]\n%s\n\n[===vcp_fold: 0.50 ::desc: 九轴高频遥测指标、旋转角速度、加速度峰值、三轴磁敏度物理强度===]\n%s",
            briefStr, detailStr
        )
    }

    // ==================================================================
    // Ambient Helpers
    // ==================================================================
    private fun registerAmbientListeners(): Boolean {
        var registered = false
        if (lightSensor != null) {
            registered = sensorManager.registerListener(
                ambientListener,
                lightSensor,
                SensorManager.SENSOR_DELAY_NORMAL,
            ) || registered
        }
        if (pressureSensor != null) {
            registered = sensorManager.registerListener(
                ambientListener,
                pressureSensor,
                SensorManager.SENSOR_DELAY_NORMAL,
            ) || registered
        }
        return registered
    }

    private fun requestAmbientSnapshot(generation: Long = snapshotGeneration) {
        val now = System.currentTimeMillis()
        if (isAmbientSnapshotActive || now - lastAmbientRequestAt < AMBIENT_CACHE_TTL) return
        lastAmbientRequestAt = now
        val latch = CountDownLatch(1)
        ambientSnapshotLatch = latch

        mainHandler.post {
            if (generation != snapshotGeneration) {
                latch.countDown()
                if (ambientSnapshotLatch === latch) {
                    ambientSnapshotLatch = null
                }
                return@post
            }
            if (isAmbientSnapshotActive) {
                latch.countDown()
                return@post
            }
            val registered = registerAmbientListeners()
            if (!registered) {
                latestAmbientStr = "环境传感器: 设备不支持或权限未授予"
                latch.countDown()
                return@post
            }
            isAmbientSnapshotActive = true
            val stopRunnable = Runnable {
                if (generation != snapshotGeneration && !isAmbientSnapshotActive) {
                    latch.countDown()
                    return@Runnable
                }
                sensorManager.unregisterListener(ambientListener)
                isAmbientSnapshotActive = false
                updateAmbientString()
                latch.countDown()
                if (ambientSnapshotLatch === latch) {
                    ambientSnapshotLatch = null
                }
            }
            ambientStopRunnable = stopRunnable
            mainHandler.postDelayed(stopRunnable, ON_DEMAND_AMBIENT_DURATION)
        }
    }

    private fun waitForRequestedSnapshots(
        type: String,
        motionLatch: CountDownLatch?,
        ambientLatch: CountDownLatch?,
        locationLatch: CountDownLatch?,
    ) {
        if (Looper.myLooper() == Looper.getMainLooper()) return
        if (type == "motion" || type == "all") {
            awaitSnapshot(motionLatch, BURST_ACTIVE_DURATION + 300L)
        }
        if (type == "ambient" || type == "all") {
            awaitSnapshot(ambientLatch, ON_DEMAND_AMBIENT_DURATION + 300L)
        }
        if (type == "location" || type == "all") {
            awaitSnapshot(locationLatch, ON_DEMAND_LOCATION_DURATION + 300L)
        }
    }

    private fun awaitSnapshot(latch: CountDownLatch?, timeoutMs: Long) {
        if (latch == null) return
        try {
            latch.await(timeoutMs, TimeUnit.MILLISECONDS)
        } catch (_: InterruptedException) {
            Thread.currentThread().interrupt()
        }
    }

    private fun updateAmbientString() {
        val lightStr = if (lightSensor != null) {
            if (lastLux >= 0.0) {
                var desc = "未知"
                if (lastLux < 50.0) desc = "暗"
                else if (lastLux < 200.0) desc = "室内"
                else if (lastLux < 1000.0) desc = "明亮"
                else desc = "户外"
                String.format(Locale.US, "环境光: %.0f lux (%s)", lastLux, desc)
            } else {
                "环境光: 采集中..."
            }
        } else {
            "环境光: 设备不支持"
        }

        val pressureStr = if (pressureSensor != null) {
            if (lastPressure >= 0.0) {
                String.format(Locale.US, "气压: %.0f hPa", lastPressure)
            } else {
                "气压: 采集中..."
            }
        } else {
            "气压: 设备不支持"
        }

        val briefStr = lightStr
        val detailStr = "$lightStr | $pressureStr"

        latestAmbientStr = String.format(
            Locale.US,
            "[===vcp_fold: 0.0 ::desc: 当前所处的物理环境光照度大体描述(如暗、室内、户外)===]\n%s\n\n[===vcp_fold: 0.45 ::desc: 物理环境大气压强、精确光照度数值与场景气压监测===]\n%s",
            briefStr, detailStr
        )
    }
}
