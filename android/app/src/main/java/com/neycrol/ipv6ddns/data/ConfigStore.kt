package com.neycrol.ipv6ddns.data

import android.content.Context
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.longPreferencesKey
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.datastore.preferences.preferencesDataStore
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.map

private val Context.dataStore by preferencesDataStore(name = "ipv6ddns_config")

object ConfigStore {
    private val KEY_TOKEN = stringPreferencesKey("api_token")
    private val KEY_ZONE = stringPreferencesKey("zone_id")
    private val KEY_RECORD = stringPreferencesKey("record_name")
    private val KEY_TIMEOUT = longPreferencesKey("timeout_sec")
    private val KEY_RUNNING = booleanPreferencesKey("running")
    private val KEY_LAST_SYNC = longPreferencesKey("last_sync_time")
    private val KEY_CURRENT_IPV6 = stringPreferencesKey("current_ipv6")
    private val KEY_LAST_ERROR = stringPreferencesKey("last_error")

    fun configFlow(context: Context): Flow<AppConfig> {
        return context.dataStore.data.map { prefs: Preferences ->
            AppConfig(
                apiToken = prefs[KEY_TOKEN] ?: "",
                zoneId = prefs[KEY_ZONE] ?: "",
                recordName = prefs[KEY_RECORD] ?: "",
                timeoutSec = prefs[KEY_TIMEOUT] ?: 30,
                lastSyncTime = prefs[KEY_LAST_SYNC] ?: 0L,
                currentIpv6 = prefs[KEY_CURRENT_IPV6] ?: "",
                lastError = prefs[KEY_LAST_ERROR] ?: ""
            )
        }
    }

    fun runningFlow(context: Context): Flow<Boolean> {
        return context.dataStore.data.map { prefs -> prefs[KEY_RUNNING] ?: false }
    }

    suspend fun saveConfig(context: Context, cfg: AppConfig) {
        context.dataStore.edit { prefs ->
            prefs[KEY_TOKEN] = cfg.apiToken
            prefs[KEY_ZONE] = cfg.zoneId
            prefs[KEY_RECORD] = cfg.recordName
            prefs[KEY_TIMEOUT] = cfg.timeoutSec
        }
    }

    suspend fun updateLastSyncTime(context: Context, timestamp: Long) {
        context.dataStore.edit { prefs ->
            prefs[KEY_LAST_SYNC] = timestamp
        }
    }

    suspend fun updateCurrentIpv6(context: Context, ipv6: String) {
        context.dataStore.edit { prefs ->
            prefs[KEY_CURRENT_IPV6] = ipv6
        }
    }

    suspend fun updateLastError(context: Context, error: String) {
        context.dataStore.edit { prefs ->
            prefs[KEY_LAST_ERROR] = error
        }
    }

    suspend fun clearError(context: Context) {
        context.dataStore.edit { prefs ->
            prefs[KEY_LAST_ERROR] = ""
        }
    }

    suspend fun setRunning(context: Context, running: Boolean) {
        context.dataStore.edit { prefs ->
            prefs[KEY_RUNNING] = running
        }
    }
}
