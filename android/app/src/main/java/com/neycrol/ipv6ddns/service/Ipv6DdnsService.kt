package com.neycrol.ipv6ddns.service

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.IBinder
import android.util.Log
import androidx.core.app.NotificationCompat
import com.neycrol.ipv6ddns.MainActivity
import com.neycrol.ipv6ddns.data.AppConfig
import com.neycrol.ipv6ddns.data.ConfigStore
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/**
 * Foreground service that monitors IPv6 address changes using
 * ConnectivityManager.NetworkCallback and updates Cloudflare AAAA records.
 *
 * Uses connectedDevice foreground service type for Android 14+ compliance.
 */
class Ipv6DdnsService : Service() {
    private val scope = CoroutineScope(Dispatchers.IO + SupervisorJob())
    private var ipv6Monitor: Ipv6Monitor? = null
    private var currentIpv6: String? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_START -> {
                // Android 14+ requires explicit foreground service type
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
                    startForeground(
                        NOTIFICATION_ID,
                        buildNotification(null, null),
                        android.content.pm.ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE
                    )
                } else {
                    startForeground(NOTIFICATION_ID, buildNotification(null, null))
                }
                startMonitoring()
            }
            ACTION_STOP -> {
                stopMonitoring()
                stopForeground(STOP_FOREGROUND_REMOVE)
                stopSelf()
            }
            else -> {
                // Service restarted by system without action
                Log.w(TAG, "Service restarted without action; attempting to resume monitoring")
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
                    startForeground(
                        NOTIFICATION_ID,
                        buildNotification(null, null),
                        android.content.pm.ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE
                    )
                } else {
                    startForeground(NOTIFICATION_ID, buildNotification(null, null))
                }
                startMonitoring()
            }
        }
        return START_STICKY
    }

    private fun startMonitoring() {
        if (ipv6Monitor != null) {
            Log.w(TAG, "Monitor already running")
            return
        }

        runBlocking { ConfigStore.setRunning(this@Ipv6DdnsService, true) }

        ipv6Monitor = Ipv6Monitor(this) { newIpv6 ->
            onIpv6Changed(newIpv6)
        }
        ipv6Monitor?.start()
        Log.i(TAG, "IPv6 monitoring started")
    }

    private fun stopMonitoring() {
        ipv6Monitor?.stop()
        ipv6Monitor = null
        currentIpv6 = null
        runBlocking { ConfigStore.setRunning(this@Ipv6DdnsService, false) }
        Log.i(TAG, "IPv6 monitoring stopped")
    }

    private fun onIpv6Changed(newIpv6: String) {
        Log.i(TAG, "IPv6 changed to: $newIpv6")
        currentIpv6 = newIpv6

        // Update stored current IPv6
        scope.launch {
            ConfigStore.updateCurrentIpv6(this@Ipv6DdnsService, newIpv6)
            ConfigStore.clearError(this@Ipv6DdnsService)
        }

        // Update notification with new address
        updateNotification(newIpv6, null)

        // Trigger Cloudflare DNS update
        scope.launch {
            val config = ConfigStore.configFlow(this@Ipv6DdnsService).first()
            if (config.apiToken.isBlank() || config.zoneId.isBlank() || config.recordName.isBlank()) {
                Log.w(TAG, "Cloudflare config incomplete, skipping DNS update")
                val error = "Configuration incomplete"
                ConfigStore.updateLastError(this@Ipv6DdnsService, error)
                updateNotification(newIpv6, error)
                return@launch
            }

            val api = CloudflareApi(
                apiToken = config.apiToken,
                zoneId = config.zoneId,
                recordName = config.recordName,
                timeoutMs = config.timeoutSec * 1000L
            )

            when (val result = api.updateAAAARecord(newIpv6)) {
                is CloudflareApi.ApiResult.Success -> {
                    Log.i(TAG, "DNS updated: ${result.recordId} -> ${result.ipv6}")
                    ConfigStore.updateLastSyncTime(
                        this@Ipv6DdnsService,
                        System.currentTimeMillis()
                    )
                    ConfigStore.clearError(this@Ipv6DdnsService)
                    updateNotification(newIpv6, null)
                }
                is CloudflareApi.ApiResult.Error -> {
                    Log.e(TAG, "DNS update failed: ${result.message}")
                    ConfigStore.updateLastError(this@Ipv6DdnsService, result.message)
                    updateNotification(newIpv6, result.message)
                }
            }
        }
    }

    private fun updateNotification(ipv6: String?, error: String?) {
        val manager = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        manager.notify(NOTIFICATION_ID, buildNotification(ipv6, error))
    }

    private fun buildNotification(ipv6: String?, error: String?): Notification {
        val channelId = ensureChannel()

        val contentIntent = PendingIntent.getActivity(
            this, 0,
            Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )

        val contentText = buildString {
            if (ipv6 != null) {
                append("IPv6: $ipv6")
            } else {
                append("Waiting for IPv6 address...")
            }
            if (error != null) {
                append("\n⚠ $error")
            }
        }

        return NotificationCompat.Builder(this, channelId)
            .setContentTitle("ipv6ddns running")
            .setContentText(contentText)
            .setStyle(NotificationCompat.BigTextStyle().bigText(contentText))
            .setSmallIcon(android.R.drawable.stat_notify_sync)
            .setOngoing(true)
            .setContentIntent(contentIntent)
            .build()
    }

    private fun ensureChannel(): String {
        val channelId = "ipv6ddns"
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val manager = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
            val channel = NotificationChannel(
                channelId,
                "ipv6ddns",
                NotificationManager.IMPORTANCE_LOW
            ).apply {
                description = "IPv6 DDNS monitoring service"
            }
            manager.createNotificationChannel(channel)
        }
        return channelId
    }

    override fun onDestroy() {
        stopMonitoring()
        scope.cancel()
        super.onDestroy()
    }

    companion object {
        const val ACTION_START = "com.neycrol.ipv6ddns.START"
        const val ACTION_STOP = "com.neycrol.ipv6ddns.STOP"
        private const val NOTIFICATION_ID = 1001
        private const val TAG = "ipv6ddns/Service"
    }
}
