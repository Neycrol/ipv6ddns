package com.neycrol.ipv6ddns.service

import android.app.ActivityManager
import android.content.Context
import android.content.Intent
import android.util.Log
import androidx.work.CoroutineWorker
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import com.neycrol.ipv6ddns.data.ConfigStore
import kotlinx.coroutines.flow.first
import java.util.concurrent.TimeUnit

/**
 * WorkManager periodic worker that checks if the foreground service is alive
 * and restarts it if needed. This acts as a safety net against aggressive
 * battery optimization on Chinese ROM devices (MIUI, ColorOS, etc.).
 *
 * Runs every 15 minutes (the minimum interval for WorkManager).
 */
class ServiceKeepAliveWorker(
    context: Context,
    params: WorkerParameters
) : CoroutineWorker(context, params) {

    companion object {
        private const val TAG = "ipv6ddns/KeepAlive"
        private const val WORK_NAME = "ipv6ddns_keepalive"

        /**
         * Schedule the periodic keep-alive check.
         */
        fun schedule(context: Context) {
            val request = PeriodicWorkRequestBuilder<ServiceKeepAliveWorker>(
                15, TimeUnit.MINUTES
            ).build()

            WorkManager.getInstance(context).enqueueUniquePeriodicWork(
                WORK_NAME,
                ExistingPeriodicWorkPolicy.KEEP,
                request
            )
            Log.i(TAG, "Keep-alive worker scheduled")
        }

        /**
         * Cancel the periodic keep-alive check.
         */
        fun cancel(context: Context) {
            WorkManager.getInstance(context).cancelUniqueWork(WORK_NAME)
            Log.i(TAG, "Keep-alive worker cancelled")
        }
    }

    override suspend fun doWork(): Result {
        val running = ConfigStore.runningFlow(applicationContext).first()
        if (!running) {
            Log.d(TAG, "Service not expected to be running, skipping")
            return Result.success()
        }

        if (!isServiceRunning()) {
            Log.w(TAG, "Service not running but should be, restarting")
            val intent = Intent(applicationContext, Ipv6DdnsService::class.java).apply {
                action = Ipv6DdnsService.ACTION_START
            }
            applicationContext.startForegroundService(intent)
        } else {
            Log.d(TAG, "Service is running, all good")
        }

        return Result.success()
    }

    @Suppress("DEPRECATION")
    private fun isServiceRunning(): Boolean {
        val manager = applicationContext.getSystemService(
            Context.ACTIVITY_SERVICE
        ) as ActivityManager
        for (service in manager.getRunningServices(Int.MAX_VALUE)) {
            if (Ipv6DdnsService::class.java.name == service.service.className) {
                return true
            }
        }
        return false
    }
}
