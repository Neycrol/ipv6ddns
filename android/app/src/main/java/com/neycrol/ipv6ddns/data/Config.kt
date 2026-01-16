package com.neycrol.ipv6ddns.data

data class AppConfig(
    val apiToken: String = "",
    val zoneId: String = "",
    val recordName: String = "",
    val timeoutSec: Long = 30,
    val pollIntervalSec: Long = 60,
    val verbose: Boolean = false,
    val multiRecord: String = "error",
    val lastSyncTime: Long = 0L  // Unix timestamp of last successful sync
)
