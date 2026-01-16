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
    private val KEY_POLL = longPreferencesKey("poll_interval_sec")
    private val KEY_VERBOSE = booleanPreferencesKey("verbose")
    private val KEY_MULTI = stringPreferencesKey("multi_record")
    private val KEY_RUNNING = booleanPreferencesKey("running")
    private val KEY_LAST_SYNC = longPreferencesKey("last_sync_time")

    fun configFlow(context: Context): Flow<AppConfig> {
        return context.dataStore.data.map { prefs: Preferences ->
            AppConfig(
                apiToken = prefs[KEY_TOKEN] ?: "",
                zoneId = prefs[KEY_ZONE] ?: "",
                recordName = prefs[KEY_RECORD] ?: "",
                timeoutSec = prefs[KEY_TIMEOUT] ?: 30,
                pollIntervalSec = prefs[KEY_POLL] ?: 60,
                verbose = prefs[KEY_VERBOSE] ?: false,
                multiRecord = prefs[KEY_MULTI] ?: "error",
                lastSyncTime = prefs[KEY_LAST_SYNC] ?: 0L
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
            prefs[KEY_POLL] = cfg.pollIntervalSec
            prefs[KEY_VERBOSE] = cfg.verbose
            prefs[KEY_MULTI] = cfg.multiRecord
            prefs[KEY_LAST_SYNC] = cfg.lastSyncTime
        }
    }

    suspend fun updateLastSyncTime(context: Context, timestamp: Long) {
        context.dataStore.edit { prefs ->
            prefs[KEY_LAST_SYNC] = timestamp
        }
    }

    suspend fun setRunning(context: Context, running: Boolean) {
        context.dataStore.edit { prefs ->
            prefs[KEY_RUNNING] = running
        }
    }
}
