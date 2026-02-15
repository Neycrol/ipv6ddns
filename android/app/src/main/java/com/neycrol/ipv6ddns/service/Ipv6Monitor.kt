package com.neycrol.ipv6ddns.service

import android.content.Context
import android.net.ConnectivityManager
import android.net.LinkAddress
import android.net.LinkProperties
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.os.Build
import android.util.Log
import java.net.Inet6Address

/**
 * Event-driven IPv6 address monitor using Android's ConnectivityManager.NetworkCallback.
 *
 * Filters addresses using IFA flags to exclude:
 * - Link-local addresses (fe80::/10)
 * - Temporary/privacy addresses (IFA_F_TEMPORARY, flag 0x01)
 * - Deprecated addresses (IFA_F_DEPRECATED, flag 0x20)
 *
 * Only reports stable, global-scope IPv6 addresses.
 */
class Ipv6Monitor(
    private val context: Context,
    private val onIpv6Changed: (String) -> Unit
) {
    companion object {
        private const val TAG = "ipv6ddns/Ipv6Monitor"

        // IFA flags from linux/if_addr.h
        private const val IFA_F_TEMPORARY = 0x01
        private const val IFA_F_DEPRECATED = 0x20
    }

    private val connectivityManager =
        context.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager

    private var lastReportedAddress: String? = null
    private var registered = false

    private val networkCallback = object : ConnectivityManager.NetworkCallback() {
        override fun onLinkPropertiesChanged(network: Network, linkProperties: LinkProperties) {
            Log.d(TAG, "onLinkPropertiesChanged: ${linkProperties.interfaceName}")
            processLinkProperties(linkProperties)
        }

        override fun onLost(network: Network) {
            Log.i(TAG, "Network lost")
            lastReportedAddress = null
        }
    }

    /**
     * Start monitoring IPv6 address changes.
     * Uses registerDefaultNetworkCallback to track the device's default network.
     */
    fun start() {
        if (registered) {
            Log.w(TAG, "Already registered")
            return
        }

        try {
            connectivityManager.registerDefaultNetworkCallback(networkCallback)
            registered = true
            Log.i(TAG, "Registered default network callback")

            // Check current state immediately
            val activeNetwork = connectivityManager.activeNetwork
            if (activeNetwork != null) {
                val linkProps = connectivityManager.getLinkProperties(activeNetwork)
                if (linkProps != null) {
                    processLinkProperties(linkProps)
                }
            }
        } catch (e: Exception) {
            Log.e(TAG, "Failed to register network callback", e)
        }
    }

    /**
     * Stop monitoring IPv6 address changes.
     */
    fun stop() {
        if (!registered) return
        try {
            connectivityManager.unregisterNetworkCallback(networkCallback)
            registered = false
            lastReportedAddress = null
            Log.i(TAG, "Unregistered network callback")
        } catch (e: Exception) {
            Log.w(TAG, "Failed to unregister network callback", e)
        }
    }

    private fun processLinkProperties(linkProperties: LinkProperties) {
        val validAddresses = linkProperties.linkAddresses
            .filter { isValidGlobalIpv6(it) }
            .map { it.address.hostAddress ?: "" }
            .filter { it.isNotEmpty() }

        if (validAddresses.isEmpty()) {
            Log.d(TAG, "No valid global IPv6 addresses found")
            return
        }

        // Pick the first valid stable global IPv6 address
        val bestAddress = validAddresses.first()

        if (bestAddress != lastReportedAddress) {
            Log.i(TAG, "IPv6 address changed: $lastReportedAddress -> $bestAddress")
            lastReportedAddress = bestAddress
            onIpv6Changed(bestAddress)
        } else {
            Log.d(TAG, "IPv6 address unchanged: $bestAddress")
        }
    }

    /**
     * Check if a LinkAddress is a valid, stable, global-scope IPv6 address.
     *
     * Filters out:
     * - Non-IPv6 addresses
     * - Link-local addresses (fe80::/10)
     * - Temporary/privacy addresses (IFA_F_TEMPORARY)
     * - Deprecated addresses (IFA_F_DEPRECATED)
     */
    private fun isValidGlobalIpv6(linkAddress: LinkAddress): Boolean {
        val addr = linkAddress.address

        // Must be IPv6
        if (addr !is Inet6Address) return false

        // Exclude link-local
        if (addr.isLinkLocalAddress) return false

        // Exclude loopback
        if (addr.isLoopbackAddress) return false

        // Exclude site-local (deprecated fec0::/10)
        @Suppress("DEPRECATION")
        if (addr.isSiteLocalAddress) return false

        // Exclude multicast
        if (addr.isMulticastAddress) return false

        // Check IFA flags via LinkAddress.getFlags()
        val flags = linkAddress.flags

        // Exclude temporary/privacy addresses (IFA_F_TEMPORARY = 0x01)
        if (flags and IFA_F_TEMPORARY != 0) {
            Log.d(TAG, "Skipping temporary address: ${addr.hostAddress}")
            return false
        }

        // Exclude deprecated addresses (IFA_F_DEPRECATED = 0x20)
        if (flags and IFA_F_DEPRECATED != 0) {
            Log.d(TAG, "Skipping deprecated address: ${addr.hostAddress}")
            return false
        }

        return true
    }
}
