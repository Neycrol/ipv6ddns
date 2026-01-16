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

    private fun computeSha256(file: File): String {
        val digest = java.security.MessageDigest.getInstance("SHA-256")
        file.inputStream().use { input ->
            val buffer = ByteArray(8192)
            var bytesRead: Int
            while (input.read(buffer).also { bytesRead = it } != -1) {
                digest.update(buffer, 0, bytesRead)
            }
        }
        return digest.digest().joinToString("") { "%02x".format(it) }
    }

    private fun getExpectedChecksum(context: Context, assetName: String): String? {
        return try {
            context.assets.open("$assetName.sha256").use { input ->
                input.bufferedReader().use { reader ->
                    reader.readLine()?.trim()?.split(" ")?.first()
                }
            }
        } catch (e: Exception) {
            Log.w(TAG, "Checksum file not found for $assetName: ${e.message}")
            null
        }
    }

    fun ensureBinary(context: Context): File {
        val destDir = File(context.filesDir, "bin")
        if (!destDir.exists()) {
            destDir.mkdirs()
        }
        val dest = File(destDir, "ipv6ddns")
        val marker = File(destDir, "ipv6ddns.version")
        val checksumMarker = File(destDir, "ipv6ddns.checksum")
        val versionMarker = BuildConfig.VERSION_CODE.toString()
        val needsCopy = !dest.exists() ||
            !marker.exists() ||
            marker.readText().trim() != versionMarker ||
            !checksumMarker.exists()
        if (needsCopy) {
            val assetName = assetNameForAbi()
            context.assets.open(assetName).use { input ->
                FileOutputStream(dest, false).use { output ->
                    input.copyTo(output)
                }
            }
            marker.writeText(versionMarker)

            val actualChecksum = computeSha256(dest)
            val expectedChecksum = getExpectedChecksum(context, assetName)

            if (expectedChecksum != null) {
                if (actualChecksum != expectedChecksum) {
                    Log.e(TAG, "Checksum mismatch for $assetName")
                    Log.e(TAG, "Expected: $expectedChecksum")
                    Log.e(TAG, "Actual: $actualChecksum")
                    dest.delete()
                    marker.delete()
                    throw SecurityException("Binary checksum verification failed")
                }
                Log.i(TAG, "Checksum verified for $assetName: $actualChecksum")
            } else {
                Log.w(TAG, "No checksum available for $assetName, skipping verification")
            }

            checksumMarker.writeText(actualChecksum)
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
