package com.neycrol.ipv6ddns.service

import android.app.ActivityManager
import android.content.Context
import android.util.Log
import androidx.work.CoroutineWorker
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import com.neycrol.ipv6ddns.data.ConfigStore
import com.neycrol.ipv6ddns.data.ConfigToml
import java.util.concurrent.TimeUnit

/**
 * WorkManager periodic worker that checks if the foreground service is alive.
 * If the service should be running (enabled by user) but isn't, it restarts it.
 * This acts as a safety net against aggressive ROM task killers.
 */
class ServiceGuardWorker(
    context: Context,
    params: WorkerParameters
) : CoroutineWorker(context, params) {

    override suspend fun doWork(): Result {
        Log.d(TAG, "Service guard check running")

        val enabled = ConfigStore.isEnabled(applicationContext)
        if (!enabled) {
            Log.d(TAG, "Service not enabled, skipping guard check")
            return Result.success()
        }

        if (isServiceRunning()) {
            Log.d(TAG, "Service is already running")
            return Result.success()
        }

        Log.i(TAG, "Service not running but should be, restarting")
        try {
            val config = ConfigStore.getConfig(applicationContext)
            if (config.apiToken.isEmpty() || config.zoneId.isEmpty() || config.recordName.isEmpty()) {
                Log.w(TAG, "Incomplete config, cannot restart service")
                return Result.success()
            }

            val configFile = ConfigToml.writeConfig(applicationContext, config)
            Ipv6DdnsService.startFromSavedConfig(applicationContext, configFile)
        } catch (e: Exception) {
            Log.e(TAG, "Failed to restart service: ${e.message}", e)
            return Result.retry()
        }

        return Result.success()
    }

    @Suppress("DEPRECATION")
    private fun isServiceRunning(): Boolean {
        val manager = applicationContext.getSystemService(Context.ACTIVITY_SERVICE)
                as ActivityManager
        for (service in manager.getRunningServices(Integer.MAX_VALUE)) {
            if (Ipv6DdnsService::class.java.name == service.service.className) {
                return true
            }
        }
        return false
    }

    companion object {
        private const val TAG = "ipv6ddns/GuardWorker"
        private const val WORK_NAME = "ipv6ddns_service_guard"

        /**
         * Schedule the periodic guard worker (every 15 minutes).
         */
        fun schedule(context: Context) {
            val request = PeriodicWorkRequestBuilder<ServiceGuardWorker>(
                15, TimeUnit.MINUTES
            ).build()

            WorkManager.getInstance(context).enqueueUniquePeriodicWork(
                WORK_NAME,
                ExistingPeriodicWorkPolicy.KEEP,
                request
            )
            Log.i(TAG, "Service guard worker scheduled")
        }

        /**
         * Cancel the periodic guard worker.
         */
        fun cancel(context: Context) {
            WorkManager.getInstance(context).cancelUniqueWork(WORK_NAME)
            Log.i(TAG, "Service guard worker cancelled")
        }
    }
}
