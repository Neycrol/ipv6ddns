package com.neycrol.ipv6ddns.service

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.util.Log
import com.neycrol.ipv6ddns.data.ConfigStore
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch

/**
 * Receives BOOT_COMPLETED broadcast to auto-start the IPv6 DDNS monitoring service.
 * Only starts if the user has explicitly enabled auto-start in settings (opt-in).
 * Also schedules the WorkManager keep-alive worker as a safety net.
 */
class BootReceiver : BroadcastReceiver() {
    companion object {
        private const val TAG = "ipv6ddns/BootReceiver"
    }

    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != Intent.ACTION_BOOT_COMPLETED) return

        Log.i(TAG, "Boot completed, checking if service should auto-start")

        val pendingResult = goAsync()
        CoroutineScope(Dispatchers.IO).launch {
            try {
                val shouldAutoStart = ConfigStore.isAutoStartOnBoot(context)

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
            } finally {
                pendingResult.finish()
            }
        }
    }
}
