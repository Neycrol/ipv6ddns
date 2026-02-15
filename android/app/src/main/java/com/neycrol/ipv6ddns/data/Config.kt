package com.neycrol.ipv6ddns.data

data class AppConfig(
    val apiToken: String = "",
    val zoneId: String = "",
    val recordName: String = "",
    val timeoutSec: Long = 30,
    val lastSyncTime: Long = 0L,
    val currentIpv6: String = "",
    val lastError: String = ""
)
