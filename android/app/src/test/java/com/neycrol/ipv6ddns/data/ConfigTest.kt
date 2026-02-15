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
        assertEquals(0L, config.lastSyncTime)
        assertEquals("", config.currentIpv6)
        assertEquals("", config.lastError)
    }

    @Test
    fun testCustomValues() {
        val config = AppConfig(
            apiToken = "test_token",
            zoneId = "test_zone",
            recordName = "example.com",
            timeoutSec = 45,
            lastSyncTime = 1234567890L,
            currentIpv6 = "2001:db8::1",
            lastError = "some error"
        )
        assertEquals("test_token", config.apiToken)
        assertEquals("test_zone", config.zoneId)
        assertEquals("example.com", config.recordName)
        assertEquals(45L, config.timeoutSec)
        assertEquals(1234567890L, config.lastSyncTime)
        assertEquals("2001:db8::1", config.currentIpv6)
        assertEquals("some error", config.lastError)
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

    @Test
    fun testCurrentIpv6() {
        val config = AppConfig(currentIpv6 = "2001:db8::1")
        assertEquals("2001:db8::1", config.currentIpv6)
    }

    @Test
    fun testLastError() {
        val config = AppConfig(lastError = "Connection timeout")
        assertEquals("Connection timeout", config.lastError)
    }
}
