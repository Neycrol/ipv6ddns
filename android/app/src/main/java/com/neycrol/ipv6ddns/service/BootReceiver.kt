package com.neycrol.ipv6ddns.service

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.util.Log
import com.neycrol.ipv6ddns.data.ConfigStore
import com.neycrol.ipv6ddns.data.ConfigToml
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch

/**
 * Receives BOOT_COMPLETED broadcast to automatically restart the DDNS service
 * after device reboot, if the user had enabled it.
 */
class BootReceiver : BroadcastReceiver() {

    override fun onReceive(context: Context, intent: Intent?) {
        if (intent?.action != Intent.ACTION_BOOT_COMPLETED) return

        Log.i(TAG, "Boot completed, checking if service should be started")

        val pendingResult = goAsync()
        CoroutineScope(Dispatchers.IO).launch {
            try {
                val enabled = ConfigStore.isEnabled(context)
                if (!enabled) {
                    Log.i(TAG, "Service not enabled, skipping auto-start")
                    return@launch
                }

                val config = ConfigStore.getConfig(context)
                if (config.apiToken.isEmpty() || config.zoneId.isEmpty() || config.recordName.isEmpty()) {
                    Log.w(TAG, "Incomplete config, skipping auto-start")
                    return@launch
                }

                val configFile = ConfigToml.writeConfig(context, config)
                Log.i(TAG, "Starting service after boot")
                Ipv6DdnsService.startFromSavedConfig(context, configFile)
            } catch (e: Exception) {
                Log.e(TAG, "Failed to start service after boot: ${e.message}", e)
            } finally {
                pendingResult.finish()
            }
        }
    }

    companion object {
        private const val TAG = "ipv6ddns/BootReceiver"
    }
}
