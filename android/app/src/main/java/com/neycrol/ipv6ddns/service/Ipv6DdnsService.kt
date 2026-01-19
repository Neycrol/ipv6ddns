package com.neycrol.ipv6ddns.service

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.IBinder
import android.util.Log
import androidx.core.app.NotificationCompat
import com.neycrol.ipv6ddns.data.ConfigStore
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import java.io.BufferedReader
import java.io.File
import java.io.InputStreamReader

class Ipv6DdnsService : Service() {
    private val scope = CoroutineScope(Dispatchers.IO + SupervisorJob())
    private var process: Process? = null
    private var restartAttempts = 0
    private val maxRestartAttempts = 5
    private var currentConfigFile: File? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_START -> {
                val configPath = intent.getStringExtra(EXTRA_CONFIG_PATH)
                if (configPath != null) {
                    // Android 14+ requires explicit foreground service type
                    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
                        startForeground(
                            NOTIFICATION_ID,
                            buildNotification(),
                            android.content.pm.ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC
                        )
                    } else {
                        startForeground(NOTIFICATION_ID, buildNotification())
                    }
                    scope.launch { startProcess(File(configPath)) }
                } else {
                    Log.e(TAG, "Missing config path")
                }
            }
            ACTION_STOP -> {
                stopProcess()
                stopForeground(STOP_FOREGROUND_REMOVE)
                stopSelf()
            }
            else -> {
                Log.w(TAG, "Service restarted without action; stopping.")
                runBlocking { ConfigStore.setRunning(this@Ipv6DdnsService, false) }
                stopForeground(STOP_FOREGROUND_REMOVE)
                stopSelf()
            }
        }
        return START_STICKY
    }

    @Synchronized
    private fun startProcess(configFile: File) {
        if (process != null) return
        currentConfigFile = configFile
        try {
            val bin = BinaryManager.ensureBinary(this)
            val builder = ProcessBuilder(
                bin.absolutePath,
                "--config",
                configFile.absolutePath
            )
            builder.redirectErrorStream(true)
            process = builder.start()
            restartAttempts = 0 // Reset restart counter on successful start
            runBlocking { ConfigStore.setRunning(this@Ipv6DdnsService, true) }
            streamLogs(process!!)
        } catch (e: SecurityException) {
            // Handle binary extraction/security failures
            Log.e(TAG, "Binary security check failed: ${e.message}", e)
            runBlocking { ConfigStore.setRunning(this@Ipv6DdnsService, false) }
            stopForeground(STOP_FOREGROUND_REMOVE)
            stopSelf()
        } catch (e: java.io.IOException) {
            // Handle I/O errors (e.g., binary not found, permission denied)
            Log.e(TAG, "Failed to start ipv6ddns (I/O error): ${e.message}", e)
            runBlocking { ConfigStore.setRunning(this@Ipv6DdnsService, false) }
            stopForeground(STOP_FOREGROUND_REMOVE)
            stopSelf()
        } catch (e: Exception) {
            // Handle other errors
            Log.e(TAG, "Failed to start ipv6ddns: ${e.message}", e)
            runBlocking { ConfigStore.setRunning(this@Ipv6DdnsService, false) }
            stopForeground(STOP_FOREGROUND_REMOVE)
            stopSelf()
        }
    }

    private fun streamLogs(proc: Process) {
        val reader = BufferedReader(InputStreamReader(proc.inputStream))
        var line: String?
        while (reader.readLine().also { line = it } != null) {
            val text = line ?: ""
            Log.i(TAG, text)
            if (text.contains("Synced (ID:")) {
                runBlocking {
                    ConfigStore.updateLastSyncTime(
                        this@Ipv6DdnsService,
                        System.currentTimeMillis()
                    )
                }
            }
        }
        val exitCode = proc.waitFor()
        Log.w(TAG, "ipv6ddns exited with code $exitCode")
        runBlocking { ConfigStore.setRunning(this@Ipv6DdnsService, false) }
        process = null

        // Implement exponential backoff for restarts
        if (restartAttempts < maxRestartAttempts) {
            restartAttempts++
            val backoffDelayMs = (1000L * (1 shl (restartAttempts - 1))).coerceAtMost(60000L)
            Log.w(TAG, "Attempting restart $restartAttempts/$maxRestartAttempts after ${backoffDelayMs}ms delay")
            // Schedule restart without blocking this thread
            scope.launch {
                delay(backoffDelayMs)
                val configFile = currentConfigFile
                if (configFile != null) {
                    startProcess(configFile)
                }
            }
        } else {
            Log.e(TAG, "Max restart attempts ($maxRestartAttempts) reached, stopping service")
            restartAttempts = 0
            currentConfigFile = null
            stopForeground(STOP_FOREGROUND_REMOVE)
            stopSelf()
        }
    }

    @Synchronized
    private fun stopProcess() {
        try {
            process?.destroy()
            process = null
            restartAttempts = 0 // Reset restart counter on manual stop
            currentConfigFile = null
            runBlocking { ConfigStore.setRunning(this@Ipv6DdnsService, false) }
        } catch (e: Exception) {
            Log.w(TAG, "Stop failed: ${e.message}")
        }
    }

    private fun buildNotification(): Notification {
        val channelId = ensureChannel()
        return NotificationCompat.Builder(this, channelId)
            .setContentTitle("ipv6ddns running")
            .setContentText("Monitoring IPv6 changes")
            .setSmallIcon(android.R.drawable.stat_notify_sync)
            .setOngoing(true)
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
            )
            manager.createNotificationChannel(channel)
        }
        return channelId
    }

    companion object {
        const val ACTION_START = "com.neycrol.ipv6ddns.START"
        const val ACTION_STOP = "com.neycrol.ipv6ddns.STOP"
        const val EXTRA_CONFIG_PATH = "config_path"
        private const val NOTIFICATION_ID = 1001
        private const val TAG = "ipv6ddns/Service"
    }
}
