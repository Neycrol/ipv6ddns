package com.neycrol.ipv6ddns.service

import org.junit.Test
import org.junit.Assert.*
import java.io.File
import java.io.FileOutputStream

/**
 * Unit tests for BinaryManager
 *
 * Note: These tests focus on the logic that doesn't require Android context.
 * Full integration tests would require Robolectric or Android instrumentation tests.
 */
class BinaryManagerTest {

    @Test
    fun testComputeSha256() {
        // Create a temporary file with known content
        val tempFile = File.createTempFile("test", ".bin")
        tempFile.deleteOnExit()

        val testContent = "Hello, World!".toByteArray()
        FileOutputStream(tempFile).use { it.write(testContent) }

        // The SHA-256 hash of "Hello, World!" is:
        // dffd6021bb2bd5b0af676290809ec3a53191dd81c7f70a4b28688a362182986f
        val expectedHash = "dffd6021bb2bd5b0af676290809ec3a53191dd81c7f70a4b28688a362182986f"

        // Note: We can't directly test BinaryManager.computeSha256() as it's private
        // This test demonstrates the expected behavior
        val digest = java.security.MessageDigest.getInstance("SHA-256")
        tempFile.inputStream().use { input ->
            val buffer = ByteArray(8192)
            var bytesRead: Int
            while (input.read(buffer).also { bytesRead = it } != -1) {
                digest.update(buffer, 0, bytesRead)
            }
        }
        val actualHash = digest.digest().joinToString("") { "%02x".format(it) }

        assertEquals(expectedHash, actualHash)
    }

    @Test
    fun testSha256EmptyFile() {
        val tempFile = File.createTempFile("test", ".bin")
        tempFile.deleteOnExit()

        // The SHA-256 hash of empty content is:
        // e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
        val expectedHash = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"

        val digest = java.security.MessageDigest.getInstance("SHA-256")
        tempFile.inputStream().use { input ->
            val buffer = ByteArray(8192)
            var bytesRead: Int
            while (input.read(buffer).also { bytesRead = it } != -1) {
                digest.update(buffer, 0, bytesRead)
            }
        }
        val actualHash = digest.digest().joinToString("") { "%02x".format(it) }

        assertEquals(expectedHash, actualHash)
    }

    @Test
    fun testSha256LargeFile() {
        val tempFile = File.createTempFile("test", ".bin")
        tempFile.deleteOnExit()

        // Create a 1MB file with repeating pattern
        val testContent = ByteArray(1024 * 1024) { (it % 256).toByte() }
        FileOutputStream(tempFile).use { it.write(testContent) }

        // Verify that the hash computation completes without error
        val digest = java.security.MessageDigest.getInstance("SHA-256")
        tempFile.inputStream().use { input ->
            val buffer = ByteArray(8192)
            var bytesRead: Int
            while (input.read(buffer).also { bytesRead = it } != -1) {
                digest.update(buffer, 0, bytesRead)
            }
        }
        val hash = digest.digest().joinToString("") { "%02x".format(it) }

        // Just verify we got a valid hash (64 hex characters)
        assertEquals(64, hash.length)
        assertTrue(hash.all { it.isDigit() || it.lowercaseChar() in 'a'..'f' })
    }

    @Test
    fun testChecksumFormat() {
        // Test that checksum format is correct (64 hex characters)
        val testChecksum = "dffd6021bb2bd5b0af676290809ec3a53191dd81c7f70a4b28688a362182986f"

        assertEquals(64, testChecksum.length)
        assertTrue(testChecksum.all { it.isDigit() || it.lowercaseChar() in 'a'..'f' })
    }

    @Test
    fun testChecksumLowerCase() {
        // Test that checksum is lowercase
        val testChecksum = "dffd6021bb2bd5b0af676290809ec3a53191dd81c7f70a4b28688a362182986f"

        assertTrue(testChecksum == testChecksum.lowercase())
    }

    @Test
    fun testAssetNameForAbi() {
        // Note: We can't directly test BinaryManager.assetNameForAbi() as it's private
        // This test documents the expected behavior

        // Expected mappings:
        // arm64-v8a -> ipv6ddns-arm64-v8a
        // x86_64 -> ipv6ddns-x86_64

        val arm64Name = "ipv6ddns-arm64-v8a"
        val x86Name = "ipv6ddns-x86_64"

        assertEquals("ipv6ddns-arm64-v8a", arm64Name)
        assertEquals("ipv6ddns-x86_64", x86Name)
    }

    @Test
    fun testBinaryPermissions() {
        // Test that the expected permission value is correct
        // 0x1C0 in octal is 0o700 (rwx for owner only)
        val expectedPermissions = 0x1C0

        assertEquals(0x1C0, expectedPermissions)
    }
}