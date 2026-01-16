package com.neycrol.ipv6ddns.data

import org.junit.Test
import org.junit.Assert.*

/**
 * Unit tests for AppConfig data class
 */
class ConfigTest {

    @Test
    fun testDefaultValues() {
        val config = AppConfig()
        assertEquals("", config.apiToken)
        assertEquals("", config.zoneId)
        assertEquals("", config.recordName)
        assertEquals(30L, config.timeoutSec)
        assertEquals(60L, config.pollIntervalSec)
        assertFalse(config.verbose)
        assertEquals("error", config.multiRecord)
        assertEquals(0L, config.lastSyncTime)
    }

    @Test
    fun testCustomValues() {
        val config = AppConfig(
            apiToken = "test_token",
            zoneId = "test_zone",
            recordName = "test.example.com",
            timeoutSec = 45,
            pollIntervalSec = 90,
            verbose = true,
            multiRecord = "first",
            lastSyncTime = 1234567890L
        )
        assertEquals("test_token", config.apiToken)
        assertEquals("test_zone", config.zoneId)
        assertEquals("test.example.com", config.recordName)
        assertEquals(45L, config.timeoutSec)
        assertEquals(90L, config.pollIntervalSec)
        assertTrue(config.verbose)
        assertEquals("first", config.multiRecord)
        assertEquals(1234567890L, config.lastSyncTime)
    }

    @Test
    fun testMultiRecordPolicyError() {
        val config = AppConfig(multiRecord = "error")
        assertEquals("error", config.multiRecord)
    }

    @Test
    fun testMultiRecordPolicyFirst() {
        val config = AppConfig(multiRecord = "first")
        assertEquals("first", config.multiRecord)
    }

    @Test
    fun testMultiRecordPolicyAll() {
        val config = AppConfig(multiRecord = "all")
        assertEquals("all", config.multiRecord)
    }

    @Test
    fun testLastSyncTimeDefault() {
        val config = AppConfig()
        assertEquals(0L, config.lastSyncTime)
    }

    @Test
    fun testLastSyncTimeCustom() {
        val config = AppConfig(lastSyncTime = System.currentTimeMillis())
        assertTrue(config.lastSyncTime > 0)
    }

    @Test
    fun testTimeoutSecDefault() {
        val config = AppConfig()
        assertEquals(30L, config.timeoutSec)
    }

    @Test
    fun testTimeoutSecCustom() {
        val config = AppConfig(timeoutSec = 120)
        assertEquals(120L, config.timeoutSec)
    }

    @Test
    fun testPollIntervalSecDefault() {
        val config = AppConfig()
        assertEquals(60L, config.pollIntervalSec)
    }

    @Test
    fun testPollIntervalSecCustom() {
        val config = AppConfig(pollIntervalSec = 300)
        assertEquals(300L, config.pollIntervalSec)
    }

    @Test
    fun testVerboseDefault() {
        val config = AppConfig()
        assertFalse(config.verbose)
    }

    @Test
    fun testVerboseEnabled() {
        val config = AppConfig(verbose = true)
        assertTrue(config.verbose)
    }

    @Test
    fun testDataClassEquality() {
        val config1 = AppConfig(
            apiToken = "token",
            zoneId = "zone",
            recordName = "record"
        )
        val config2 = AppConfig(
            apiToken = "token",
            zoneId = "zone",
            recordName = "record"
        )
        assertEquals(config1, config2)
    }

    @Test
    fun testDataClassCopy() {
        val original = AppConfig(
            apiToken = "token",
            zoneId = "zone",
            recordName = "record"
        )
        val copy = original.copy(apiToken = "new_token")
        assertEquals("new_token", copy.apiToken)
        assertEquals("zone", copy.zoneId)
        assertEquals("record", copy.recordName)
    }
}