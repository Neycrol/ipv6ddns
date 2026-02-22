package com.neycrol.ipv6ddns.service

import android.util.Log
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import org.json.JSONObject
import java.io.OutputStreamWriter
import java.net.HttpURLConnection
import java.net.URL

/**
 * Cloudflare DNS API client for updating AAAA records.
 * Implements exponential backoff retry logic.
 */
class CloudflareApi(
    private val apiToken: String,
    private val zoneId: String,
    private val recordName: String,
    private val timeoutMs: Long = 30_000L
) {
    companion object {
        private const val TAG = "ipv6ddns/CloudflareApi"
        private const val BASE_URL = "https://api.cloudflare.com/client/v4"
        private const val MAX_RETRIES = 5
        private const val INITIAL_BACKOFF_MS = 1000L
        private const val MAX_BACKOFF_MS = 60_000L
    }

    data class DnsRecord(
        val id: String,
        val name: String,
        val type: String,
        val content: String
    )

    sealed class ApiResult {
        data class Success(val recordId: String, val ipv6: String) : ApiResult()
        data class Error(val message: String) : ApiResult()
    }

    /**
     * Update the AAAA record for [recordName] to point to [ipv6Address].
     * Creates the record if it doesn't exist.
     * Uses exponential backoff for retries.
     */
    suspend fun updateAAAARecord(ipv6Address: String): ApiResult = withContext(Dispatchers.IO) {
        var lastError: String? = null

        for (attempt in 0 until MAX_RETRIES) {
            if (attempt > 0) {
                val backoff = (INITIAL_BACKOFF_MS * (1L shl (attempt - 1)))
                    .coerceAtMost(MAX_BACKOFF_MS)
                Log.i(TAG, "Retry attempt $attempt/$MAX_RETRIES after ${backoff}ms")
                delay(backoff)
            }

            try {
                // Step 1: Find existing AAAA record
                val existingRecord = findRecord()

                // Step 2: Update or create
                return@withContext if (existingRecord != null) {
                    updateRecord(existingRecord.id, ipv6Address)
                } else {
                    createRecord(ipv6Address)
                }
            } catch (e: Exception) {
                lastError = e.message ?: "Unknown error"
                Log.w(TAG, "Attempt ${attempt + 1}/$MAX_RETRIES failed: $lastError", e)
            }
        }

        ApiResult.Error("All $MAX_RETRIES attempts failed. Last error: $lastError")
    }

    private fun findRecord(): DnsRecord? {
        val url = URL("$BASE_URL/zones/$zoneId/dns_records?type=AAAA&name=$recordName")
        val conn = (url.openConnection() as HttpURLConnection).apply {
            requestMethod = "GET"
            setRequestProperty("Authorization", "Bearer $apiToken")
            setRequestProperty("Content-Type", "application/json")
            connectTimeout = timeoutMs.toInt()
            readTimeout = timeoutMs.toInt()
        }

        try {
            val responseCode = conn.responseCode
            val body = if (responseCode in 200..299) {
                conn.inputStream.bufferedReader().readText()
            } else {
                val errorBody = conn.errorStream?.bufferedReader()?.readText() ?: "No error body"
                throw RuntimeException("HTTP $responseCode: $errorBody")
            }

            val json = JSONObject(body)
            if (!json.getBoolean("success")) {
                val errors = json.getJSONArray("errors")
                throw RuntimeException("API error: $errors")
            }

            val results = json.getJSONArray("result")
            if (results.length() == 0) return null

            val record = results.getJSONObject(0)
            return DnsRecord(
                id = record.getString("id"),
                name = record.getString("name"),
                type = record.getString("type"),
                content = record.getString("content")
            )
        } finally {
            conn.disconnect()
        }
    }

    private fun updateRecord(recordId: String, ipv6Address: String): ApiResult {
        val url = URL("$BASE_URL/zones/$zoneId/dns_records/$recordId")
        val conn = (url.openConnection() as HttpURLConnection).apply {
            requestMethod = "PUT"
            setRequestProperty("Authorization", "Bearer $apiToken")
            setRequestProperty("Content-Type", "application/json")
            connectTimeout = timeoutMs.toInt()
            readTimeout = timeoutMs.toInt()
            doOutput = true
        }

        try {
            val payload = JSONObject().apply {
                put("type", "AAAA")
                put("name", recordName)
                put("content", ipv6Address)
                put("ttl", 1) // Auto TTL
                put("proxied", false)
            }

            OutputStreamWriter(conn.outputStream).use { writer ->
                writer.write(payload.toString())
            }

            val responseCode = conn.responseCode
            val body = if (responseCode in 200..299) {
                conn.inputStream.bufferedReader().readText()
            } else {
                val errorBody = conn.errorStream?.bufferedReader()?.readText() ?: "No error body"
                throw RuntimeException("HTTP $responseCode: $errorBody")
            }

            val json = JSONObject(body)
            if (!json.getBoolean("success")) {
                val errors = json.getJSONArray("errors")
                throw RuntimeException("API error: $errors")
            }

            val result = json.getJSONObject("result")
            Log.i(TAG, "Updated AAAA record ${result.getString("id")} -> $ipv6Address")
            return ApiResult.Success(recordId = result.getString("id"), ipv6 = ipv6Address)
        } finally {
            conn.disconnect()
        }
    }

    private fun createRecord(ipv6Address: String): ApiResult {
        val url = URL("$BASE_URL/zones/$zoneId/dns_records")
        val conn = (url.openConnection() as HttpURLConnection).apply {
            requestMethod = "POST"
            setRequestProperty("Authorization", "Bearer $apiToken")
            setRequestProperty("Content-Type", "application/json")
            connectTimeout = timeoutMs.toInt()
            readTimeout = timeoutMs.toInt()
            doOutput = true
        }

        try {
            val payload = JSONObject().apply {
                put("type", "AAAA")
                put("name", recordName)
                put("content", ipv6Address)
                put("ttl", 1) // Auto TTL
                put("proxied", false)
            }

            OutputStreamWriter(conn.outputStream).use { writer ->
                writer.write(payload.toString())
            }

            val responseCode = conn.responseCode
            val body = if (responseCode in 200..299) {
                conn.inputStream.bufferedReader().readText()
            } else {
                val errorBody = conn.errorStream?.bufferedReader()?.readText() ?: "No error body"
                throw RuntimeException("HTTP $responseCode: $errorBody")
            }

            val json = JSONObject(body)
            if (!json.getBoolean("success")) {
                val errors = json.getJSONArray("errors")
                throw RuntimeException("API error: $errors")
            }

            val result = json.getJSONObject("result")
            Log.i(TAG, "Created AAAA record ${result.getString("id")} -> $ipv6Address")
            return ApiResult.Success(recordId = result.getString("id"), ipv6 = ipv6Address)
        } finally {
            conn.disconnect()
        }
    }
}
