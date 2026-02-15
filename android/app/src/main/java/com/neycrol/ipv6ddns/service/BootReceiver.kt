package com.neycrol.ipv6ddns.service

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.util.Log

/**
 * Receives BOOT_COMPLETED broadcast to auto-start the IPv6 DDNS monitoring service.
 * Only starts if the user has enabled auto-start in settings.
 * Also schedules the WorkManager keep-alive worker as a safety net.
 */
class BootReceiver : BroadcastReceiver() {
    companion object {
        private const val TAG = "ipv6ddns/BootReceiver"
    }

    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != Intent.ACTION_BOOT_COMPLETED) return

        Log.i(TAG, "Boot completed, checking if service should auto-start")

        // Check if service was previously running
        val prefs = context.getSharedPreferences("ipv6ddns_boot", Context.MODE_PRIVATE)
        val shouldAutoStart = prefs.getBoolean("auto_start", false)

        if (shouldAutoStart) {
            Log.i(TAG, "Auto-starting IPv6 DDNS service")
            val serviceIntent = Intent(context, Ipv6DdnsService::class.java).apply {
                action = Ipv6DdnsService.ACTION_START
            }
            context.startForegroundService(serviceIntent)
            // Schedule keep-alive worker as safety net
            ServiceKeepAliveWorker.schedule(context)
        } else {
            Log.i(TAG, "Auto-start not enabled, skipping")
        }
    }
}
