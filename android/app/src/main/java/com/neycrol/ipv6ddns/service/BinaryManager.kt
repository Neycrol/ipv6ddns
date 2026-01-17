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
            "Unsupported ABI: ${abis.joinToString()} (supported: arm64-v8a, x86_64). " +
            "This app is not compatible with your device architecture. " +
            "Please use a device with ARM64 or x86_64 architecture."
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
        val assetName = assetNameForAbi()
        val expectedChecksum = getExpectedChecksum(context, assetName)
            ?: run {
                Log.e(TAG, "Checksum file not found for $assetName; refusing to run")
                dest.delete()
                marker.delete()
                checksumMarker.delete()
                throw SecurityException(
                    "Security check failed: Checksum file missing for $assetName. " +
                    "Expected file: $assetName.sha256 in app assets. " +
                    "Please reinstall the app or contact support."
                )
            }
        val needsCopy = !dest.exists() ||
            !marker.exists() ||
            marker.readText().trim() != versionMarker
        if (needsCopy) {
            try {
                context.assets.open(assetName).use { input ->
                    FileOutputStream(dest, false).use { output ->
                        input.copyTo(output)
                    }
                }
                Log.i(TAG, "Copied binary from assets: $assetName (version: $versionMarker)")

                // Verify checksum immediately after copy, before writing marker
                val actualChecksum = computeSha256(dest)
                if (actualChecksum != expectedChecksum) {
                    Log.e(TAG, "Checksum mismatch for $assetName")
                    Log.e(TAG, "Expected: $expectedChecksum")
                    Log.e(TAG, "Actual: $actualChecksum")
                    Log.e(TAG, "Binary file: ${dest.absolutePath} (size: ${dest.length()} bytes)")
                    dest.delete()
                    marker.delete()
                    checksumMarker.delete()
                    throw SecurityException(
                        "Security check failed: Binary checksum mismatch. " +
                        "This may indicate a corrupted installation. " +
                        "Please clear app data and reinstall: Settings > Apps > ipv6ddns > Clear data."
                    )
                }
                Log.i(TAG, "Checksum verified for $assetName: $actualChecksum")
                checksumMarker.writeText(actualChecksum)

                // Only write marker after successful checksum verification
                marker.writeText(versionMarker)
            } catch (e: Exception) {
                Log.e(TAG, "Failed to copy binary from assets: $assetName", e)
                dest.delete()
                marker.delete()
                checksumMarker.delete()
                throw SecurityException(
                    "Failed to extract binary: ${e.localizedMessage}. " +
                    "Please ensure the app has sufficient storage space and try reinstalling."
                )
            }
        } else {
            // Verify checksum even if no copy was needed (e.g., app update scenario)
            val actualChecksum = computeSha256(dest)
            if (actualChecksum != expectedChecksum) {
                Log.e(TAG, "Checksum mismatch for $assetName (re-verification)")
                Log.e(TAG, "Expected: $expectedChecksum")
                Log.e(TAG, "Actual: $actualChecksum")
                Log.e(TAG, "Binary file: ${dest.absolutePath} (size: ${dest.length()} bytes)")
                dest.delete()
                marker.delete()
                checksumMarker.delete()
                throw SecurityException(
                    "Security check failed: Binary checksum mismatch. " +
                    "This may indicate a corrupted installation. " +
                    "Please clear app data and reinstall: Settings > Apps > ipv6ddns > Clear data."
                )
            }
            Log.i(TAG, "Checksum verified for $assetName: $actualChecksum")
            checksumMarker.writeText(actualChecksum)
        }
        try {
            Os.chmod(dest.absolutePath, 0x1C0)
        } catch (e: Exception) {
            Log.w(TAG, "chmod via Os failed, falling back: ${e.message}")
            try {
                val process = Runtime.getRuntime().exec(arrayOf("chmod", "700", dest.absolutePath))
                val exitCode = process.waitFor()
                if (exitCode != 0) {
                    Log.w(TAG, "chmod fallback failed with exit code: $exitCode")
                }
            } catch (ignored: Exception) {
                Log.w(TAG, "chmod fallback failed: ${ignored.message}")
            }
        }
        return dest
    }
}
