package com.neycrol.ipv6ddns.service

import android.content.Context
import android.os.Build
import android.system.Os
import android.util.Log
import com.neycrol.ipv6ddns.BuildConfig
import java.io.File
import java.io.FileOutputStream

object BinaryManager {
    private const val TAG = "ipv6ddns/Binary"

    private fun assetNameForAbi(): String {
        val abis = Build.SUPPORTED_ABIS ?: arrayOf("arm64-v8a")
        for (abi in abis) {
            when (abi) {
                "arm64-v8a" -> return "ipv6ddns-arm64-v8a"
                "x86_64" -> return "ipv6ddns-x86_64"
            }
        }
        throw IllegalStateException(
            "Unsupported ABI: ${abis.joinToString()} (supported: arm64-v8a, x86_64)"
        )
    }

    fun ensureBinary(context: Context): File {
        val destDir = File(context.filesDir, "bin")
        if (!destDir.exists()) {
            destDir.mkdirs()
        }
        val dest = File(destDir, "ipv6ddns")
        val marker = File(destDir, "ipv6ddns.version")
        val versionMarker = BuildConfig.VERSION_CODE.toString()
        val needsCopy = !dest.exists() ||
            !marker.exists() ||
            marker.readText().trim() != versionMarker
        if (needsCopy) {
            val assetName = assetNameForAbi()
            context.assets.open(assetName).use { input ->
                FileOutputStream(dest, false).use { output ->
                    input.copyTo(output)
                }
            }
            marker.writeText(versionMarker)
        }
        try {
            Os.chmod(dest.absolutePath, 0x1C0)
        } catch (e: Exception) {
            Log.w(TAG, "chmod via Os failed, falling back: ${e.message}")
            try {
                Runtime.getRuntime().exec(arrayOf("chmod", "700", dest.absolutePath)).waitFor()
            } catch (ignored: Exception) {
                Log.w(TAG, "chmod fallback failed: ${ignored.message}")
            }
        }
        return dest
    }
}
