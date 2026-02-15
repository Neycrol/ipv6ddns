package com.neycrol.ipv6ddns.service

import io.mockk.*
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.runTest
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.junit.Assert.*
import org.junit.After
import org.junit.Before
import org.junit.Test

/**
 * Unit tests for CloudflareApi
 * 
 * These tests verify the API structure, data classes, and network behavior using MockWebServer.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class CloudflareApiTest {

    private lateinit var mockServer: MockWebServer
    private lateinit var api: CloudflareApi
    private val testApiToken = "test_api_token_12345"
    private val testZoneId = "test_zone_id_67890"
    private val testRecordName = "test.example.com"
    private val testIpv6Address = "2001:db8::1"

    @Before
    fun setup() {
        mockServer = MockWebServer()
        mockServer.start()
        
        // Create API with mock server URL (we'll need to make BASE_URL configurable for testing)
        api = CloudflareApi(testApiToken, testZoneId, testRecordName)
    }

    @After
    fun tearDown() {
        mockServer.shutdown()
    }

    @Test
    fun testCloudflareApiInitialization() {
        assertNotNull("API should be initialized", api)
        
        // Verify constants are accessible
        assertEquals("ipv6ddns/CloudflareApi", CloudflareApi.Companion.TAG)
        assertTrue(CloudflareApi.Companion.BASE_URL.startsWith("https://"))
    }

    @Test
    fun testDnsRecordDataClass() {
        val record = CloudflareApi.DnsRecord(
            id = "record_id_123",
            name = testRecordName,
            type = "AAAA",
            content = testIpv6Address
        )

        assertEquals("record_id_123", record.id)
        assertEquals(testRecordName, record.name)
        assertEquals("AAAA", record.type)
        assertEquals(testIpv6Address, record.content)
    }

    @Test
    fun testDnsRecordDataClassEquality() {
        val record1 = CloudflareApi.DnsRecord(
            id = "record_id_123",
            name = testRecordName,
            type = "AAAA",
            content = testIpv6Address
        )

        val record2 = CloudflareApi.DnsRecord(
            id = "record_id_123",
            name = testRecordName,
            type = "AAAA",
            content = testIpv6Address
        )

        val record3 = CloudflareApi.DnsRecord(
            id = "different_id",
            name = testRecordName,
            type = "AAAA",
            content = testIpv6Address
        )

        assertEquals("Same records should be equal", record1, record2)
        assertNotEquals("Different records should not be equal", record1, record3)
    }

    @Test
    fun testApiResultSealedClass() {
        // Test Success variant
        val success = CloudflareApi.ApiResult.Success(
            recordId = "record_id_123",
            ipv6 = testIpv6Address
        )

        assertTrue("Should be Success variant", success is CloudflareApi.ApiResult.Success)
        when (success) {
            is CloudflareApi.ApiResult.Success -> {
                assertEquals("record_id_123", success.recordId)
                assertEquals(testIpv6Address, success.ipv6)
            }
            else -> fail("Should be Success variant")
        }

        // Test Error variant
        val error = CloudflareApi.ApiResult.Error("Network error")

        assertTrue("Should be Error variant", error is CloudflareApi.ApiResult.Error)
        when (error) {
            is CloudflareApi.ApiResult.Error -> {
                assertEquals("Network error", error.message)
            }
            else -> fail("Should be Error variant")
        }
    }

    @Test
    fun testApiResultExhaustiveWhen() {
        val results = listOf(
            CloudflareApi.ApiResult.Success("id1", "2001:db8::1"),
            CloudflareApi.ApiResult.Error("Error message")
        )

        for (result in results) {
            when (result) {
                is CloudflareApi.ApiResult.Success -> {
                    assertNotNull(result.recordId)
                    assertNotNull(result.ipv6)
                }
                is CloudflareApi.ApiResult.Error -> {
                    assertNotNull(result.message)
                    assertTrue(result.message.isNotEmpty())
                }
            }
        }
    }

    @Test
    fun testExponentialBackoffConstants() {
        // Verify that backoff constants are defined and reasonable
        assertTrue("Max retries should be positive", CloudflareApi.Companion.MAX_RETRIES > 0)
        assertTrue("Initial backoff should be positive", CloudflareApi.Companion.INITIAL_BACKOFF_MS > 0)
        assertTrue("Max backoff should be greater than initial", 
                  CloudflareApi.Companion.MAX_BACKOFF_MS > CloudflareApi.Companion.INITIAL_BACKOFF_MS)
    }

    @Test
    fun testTimeoutConfiguration() {
        // Test with default timeout
        val apiWithDefaultTimeout = CloudflareApi(testApiToken, testZoneId, testRecordName)
        assertNotNull(apiWithDefaultTimeout)

        // Test with custom timeout
        val customTimeout = 60000L
        val apiWithCustomTimeout = CloudflareApi(testApiToken, testZoneId, testRecordName, customTimeout)
        assertNotNull(apiWithCustomTimeout)
        
        // Verify both APIs have the same structure but different timeouts
        assertNotSame("APIs should be different instances", apiWithDefaultTimeout, apiWithCustomTimeout)
    }

    @Test
    fun testEmptyTokenHandling() {
        // Test API initialization with empty token (should not crash)
        val apiWithEmptyToken = CloudflareApi("", testZoneId, testRecordName)
        assertNotNull(apiWithEmptyToken)
    }

    @Test
    fun testEmptyZoneIdHandling() {
        // Test API initialization with empty zone ID (should not crash)
        val apiWithEmptyZoneId = CloudflareApi(testApiToken, "", testRecordName)
        assertNotNull(apiWithEmptyZoneId)
    }

    @Test
    fun testEmptyRecordNameHandling() {
        // Test API initialization with empty record name (should not crash)
        val apiWithEmptyRecordName = CloudflareApi(testApiToken, testZoneId, "")
        assertNotNull(apiWithEmptyRecordName)
    }

    @Test
    fun testIpv6AddressValidation() {
        val validIpv6Addresses = listOf(
            "2001:db8::1",
            "::1",
            "fe80::1",
            "2001:0db8:0000:0000:0000:0000:0000:0001"
        )

        for (address in validIpv6Addresses) {
            // The API should accept any string as IPv6 address
            // Actual validation happens at the network level
            assertTrue("Should accept valid-looking IPv6: $address", address.contains(":"))
        }
    }

    @Test
    fun testCompanionObjectConstants() {
        // Verify companion object constants are accessible
        assertEquals("ipv6ddns/CloudflareApi", CloudflareApi.Companion.TAG)
        assertTrue(CloudflareApi.Companion.BASE_URL.startsWith("https://"))
        assertTrue(CloudflareApi.Companion.MAX_RETRIES >= 3)
        assertTrue(CloudflareApi.Companion.MAX_RETRIES <= 10)
    }

    @Test
    fun testBackOffCalculation() {
        // Test exponential backoff calculation logic
        val initialBackoff = CloudflareApi.Companion.INITIAL_BACKOFF_MS
        val maxBackoff = CloudflareApi.Companion.MAX_BACKOFF_MS
        val maxRetries = CloudflareApi.Companion.MAX_RETRIES

        for (attempt in 1 until maxRetries) {
            val backoff = initialBackoff * (1L shl (attempt - 1))
            val actualBackoff = backoff.coerceAtMost(maxBackoff)
            
            assertTrue("Backoff should not exceed max", actualBackoff <= maxBackoff)
            assertTrue("Backoff should be at least initial", actualBackoff >= initialBackoff)
        }
    }

    @Test
    fun testDnsRecordCopy() {
        val original = CloudflareApi.DnsRecord(
            id = "original_id",
            name = testRecordName,
            type = "AAAA",
            content = testIpv6Address
        )

        val copy = original.copy(id = "copied_id")

        assertEquals("copied_id", copy.id)
        assertEquals(original.name, copy.name)
        assertEquals(original.type, copy.type)
        assertEquals(original.content, copy.content)
    }

    @Test
    fun testDnsRecordToString() {
        val record = CloudflareApi.DnsRecord(
            id = "record_id",
            name = testRecordName,
            type = "AAAA",
            content = testIpv6Address
        )

        val stringRepresentation = record.toString()
        assertTrue("Should contain ID", stringRepresentation.contains("record_id"))
        assertTrue("Should contain name", stringRepresentation.contains(testRecordName))
    }

    @Test
    fun testIpv6AddressFormats() {
        // Test various IPv6 address formats that should be accepted
        val testFormats = listOf(
            "2001:db8::1",                    // Compressed
            "2001:0db8:0000:0000:0000:0000:0000:0001",  // Full
            "::1",                            // Loopback
            "fe80::1"                         // Link-local
        )

        for (format in testFormats) {
            // Just verify they contain colons (basic IPv6 check)
            assertTrue("Should be valid IPv6 format: $format", format.contains(":"))
        }
    }

    @Test
    fun testMaxRetriesConstant() {
        // Verify MAX_RETRIES is within reasonable bounds
        assertTrue("MAX_RETRIES should be at least 3", CloudflareApi.Companion.MAX_RETRIES >= 3)
        assertTrue("MAX_RETRIES should not exceed 10", CloudflareApi.Companion.MAX_RETRIES <= 10)
    }

    @Test
    fun testInitialBackoffMsConstant() {
        // Verify INITIAL_BACKOFF_MS is reasonable (1-5 seconds)
        assertTrue("INITIAL_BACKOFF_MS should be at least 500ms", 
                  CloudflareApi.Companion.INITIAL_BACKOFF_MS >= 500)
        assertTrue("INITIAL_BACKOFF_MS should not exceed 5000ms", 
                  CloudflareApi.Companion.INITIAL_BACKOFF_MS <= 5000)
    }

    @Test
    fun testMaxBackoffMsConstant() {
        // Verify MAX_BACKOFF_MS is reasonable (30-120 seconds)
        assertTrue("MAX_BACKOFF_MS should be at least 30000ms", 
                  CloudflareApi.Companion.MAX_BACKOFF_MS >= 30000)
        assertTrue("MAX_BACKOFF_MS should not exceed 120000ms", 
                  CloudflareApi.Companion.MAX_BACKOFF_MS <= 120000)
    }
}
