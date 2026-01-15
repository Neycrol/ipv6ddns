package com.neycrol.ipv6ddns.data

import android.content.Context
import java.io.File

object ConfigToml {
    fun writeConfig(context: Context, cfg: AppConfig): File {
        val dir = File(context.filesDir, "ipv6ddns")
        if (!dir.exists()) {
            dir.mkdirs()
        }
        val file = File(dir, "config.toml")
        val content = buildString {
            appendLine("api_token = \"${cfg.apiToken}\"")
            appendLine("zone_id = \"${cfg.zoneId}\"")
            appendLine("record_name = \"${cfg.recordName}\"")
            appendLine("timeout = ${cfg.timeoutSec}")
            appendLine("poll_interval = ${cfg.pollIntervalSec}")
            appendLine("verbose = ${cfg.verbose}")
            appendLine("multi_record = \"${cfg.multiRecord}\"")
        }
        file.writeText(content)
        return file
    }
}
