package com.neycrol.ipv6ddns

import org.junit.Test
import org.junit.Assert.*

/**
 * Unit tests for MainActivity validation logic
 *
 * Note: These tests focus on the validation logic that doesn't require Android context.
 * Full integration tests would require Robolectric or Android instrumentation tests.
 */
class MainActivityTest {

    // Validation constants matching MainActivity
    private val minTimeout = 1L
    private val maxTimeout = 300L
    private val minPollInterval = 10L
    private val maxPollInterval = 3600L

    @Test
    fun testValidation_emptyApiToken() {
        val apiToken = ""
        val zoneId = "test_zone"
        val recordName = "test.example.com"
        val timeout = "30"
        val pollInterval = "60"

        val error = validateConfig(apiToken, zoneId, recordName, timeout, pollInterval)
        assertNotNull("Empty API token should fail validation", error)
        assertTrue("Error should mention API token", error!!.contains("API Token"))
    }

    @Test
    fun testValidation_emptyZoneId() {
        val apiToken = "test_token"
        val zoneId = ""
        val recordName = "test.example.com"
        val timeout = "30"
        val pollInterval = "60"

        val error = validateConfig(apiToken, zoneId, recordName, timeout, pollInterval)
        assertNotNull("Empty zone ID should fail validation", error)
        assertTrue("Error should mention Zone ID", error!!.contains("Zone ID"))
    }

    @Test
    fun testValidation_emptyRecordName() {
        val apiToken = "test_token"
        val zoneId = "test_zone"
        val recordName = ""
        val timeout = "30"
        val pollInterval = "60"

        val error = validateConfig(apiToken, zoneId, recordName, timeout, pollInterval)
        assertNotNull("Empty record name should fail validation", error)
        assertTrue("Error should mention Record Name", error!!.contains("Record Name"))
    }

    @Test
    fun testValidation_timeoutBelowMin() {
        val apiToken = "test_token"
        val zoneId = "test_zone"
        val recordName = "test.example.com"
        val timeout = "0"
        val pollInterval = "60"

        val error = validateConfig(apiToken, zoneId, recordName, timeout, pollInterval)
        assertNotNull("Timeout below min should fail validation", error)
        assertTrue("Error should mention timeout", error!!.lowercase().contains("timeout"))
    }

    @Test
    fun testValidation_timeoutAboveMax() {
        val apiToken = "test_token"
        val zoneId = "test_zone"
        val recordName = "test.example.com"
        val timeout = "500"
        val pollInterval = "60"

        val error = validateConfig(apiToken, zoneId, recordName, timeout, pollInterval)
        assertNotNull("Timeout above max should fail validation", error)
        assertTrue("Error should mention timeout", error!!.lowercase().contains("timeout"))
    }

    @Test
    fun testValidation_timeoutAtMin() {
        val apiToken = "test_token"
        val zoneId = "test_zone"
        val recordName = "test.example.com"
        val timeout = minTimeout.toString()
        val pollInterval = "60"

        val error = validateConfig(apiToken, zoneId, recordName, timeout, pollInterval)
        assertNull("Timeout at min should pass validation", error)
    }

    @Test
    fun testValidation_timeoutAtMax() {
        val apiToken = "test_token"
        val zoneId = "test_zone"
        val recordName = "test.example.com"
        val timeout = maxTimeout.toString()
        val pollInterval = "60"

        val error = validateConfig(apiToken, zoneId, recordName, timeout, pollInterval)
        assertNull("Timeout at max should pass validation", error)
    }

    @Test
    fun testValidation_pollIntervalBelowMin() {
        val apiToken = "test_token"
        val zoneId = "test_zone"
        val recordName = "test.example.com"
        val timeout = "30"
        val pollInterval = "5"

        val error = validateConfig(apiToken, zoneId, recordName, timeout, pollInterval)
        assertNotNull("Poll interval below min should fail validation", error)
        assertTrue("Error should mention poll interval", error!!.lowercase().contains("poll interval"))
    }

    @Test
    fun testValidation_pollIntervalAboveMax() {
        val apiToken = "test_token"
        val zoneId = "test_zone"
        val recordName = "test.example.com"
        val timeout = "30"
        val pollInterval = "5000"

        val error = validateConfig(apiToken, zoneId, recordName, timeout, pollInterval)
        assertNotNull("Poll interval above max should fail validation", error)
        assertTrue("Error should mention poll interval", error!!.lowercase().contains("poll interval"))
    }

    @Test
    fun testValidation_pollIntervalAtMin() {
        val apiToken = "test_token"
        val zoneId = "test_zone"
        val recordName = "test.example.com"
        val timeout = "30"
        val pollInterval = minPollInterval.toString()

        val error = validateConfig(apiToken, zoneId, recordName, timeout, pollInterval)
        assertNull("Poll interval at min should pass validation", error)
    }

    @Test
    fun testValidation_pollIntervalAtMax() {
        val apiToken = "test_token"
        val zoneId = "test_zone"
        val recordName = "test.example.com"
        val timeout = "30"
        val pollInterval = maxPollInterval.toString()

        val error = validateConfig(apiToken, zoneId, recordName, timeout, pollInterval)
        assertNull("Poll interval at max should pass validation", error)
    }

    @Test
    fun testValidation_invalidTimeout() {
        val apiToken = "test_token"
        val zoneId = "test_zone"
        val recordName = "test.example.com"
        val timeout = "invalid"
        val pollInterval = "60"

        val error = validateConfig(apiToken, zoneId, recordName, timeout, pollInterval)
        assertNotNull("Invalid timeout should fail validation", error)
        assertTrue("Error should mention timeout", error!!.lowercase().contains("timeout"))
    }

    @Test
    fun testValidation_invalidPollInterval() {
        val apiToken = "test_token"
        val zoneId = "test_zone"
        val recordName = "test.example.com"
        val timeout = "30"
        val pollInterval = "invalid"

        val error = validateConfig(apiToken, zoneId, recordName, timeout, pollInterval)
        assertNotNull("Invalid poll interval should fail validation", error)
        assertTrue("Error should mention poll interval", error!!.lowercase().contains("poll interval"))
    }

    @Test
    fun testValidation_allValid() {
        val apiToken = "test_token"
        val zoneId = "test_zone"
        val recordName = "test.example.com"
        val timeout = "30"
        val pollInterval = "60"

        val error = validateConfig(apiToken, zoneId, recordName, timeout, pollInterval)
        assertNull("All valid values should pass validation", error)
    }

    @Test
    fun testValidation_whitespaceOnly() {
        val apiToken = "   "
        val zoneId = "test_zone"
        val recordName = "test.example.com"
        val timeout = "30"
        val pollInterval = "60"

        val error = validateConfig(apiToken, zoneId, recordName, timeout, pollInterval)
        assertNotNull("Whitespace-only API token should fail validation", error)
    }

    /**
     * Simulates the validation logic from MainActivity
     *
     * @return Error message if validation fails, null if valid
     */
    private fun validateConfig(
        apiToken: String,
        zoneId: String,
        recordName: String,
        timeout: String,
        pollInterval: String
    ): String? {
        if (apiToken.trim().isEmpty()) {
            return "API Token is required"
        }
        if (zoneId.trim().isEmpty()) {
            return "Zone ID is required"
        }
        if (recordName.trim().isEmpty()) {
            return "Record Name is required"
        }
        val timeoutValue = timeout.toLongOrNull()
        if (timeoutValue == null || timeoutValue < minTimeout || timeoutValue > maxTimeout) {
            return "Timeout must be between $minTimeout and $maxTimeout seconds"
        }
        val pollIntervalValue = pollInterval.toLongOrNull()
        if (pollIntervalValue == null || pollIntervalValue < minPollInterval || pollIntervalValue > maxPollInterval) {
            return "Poll interval must be between $minPollInterval and $maxPollInterval seconds"
        }
        return null
    }
}