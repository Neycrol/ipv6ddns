package com.neycrol.ipv6ddns

import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import android.util.Log
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import com.neycrol.ipv6ddns.data.AppConfig
import com.neycrol.ipv6ddns.data.ConfigStore
import com.neycrol.ipv6ddns.data.ConfigToml
import com.neycrol.ipv6ddns.service.Ipv6DdnsService
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

private const val TAG = "ipv6ddns/MainActivity"

class MainActivity : ComponentActivity() {

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent { AppScreen() }
    }

}

private fun isBatteryOptimizationEnabled(context: android.content.Context): Boolean {
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
        val pm = context.getSystemService(android.content.Context.POWER_SERVICE) as android.os.PowerManager
        return !pm.isIgnoringBatteryOptimizations(context.packageName)
    }
    return false
}

private fun requestBatteryOptimizationExemption(context: android.content.Context) {
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
        try {
            val intent = Intent().apply {
                action = Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS
                data = Uri.parse("package:${context.packageName}")
                if (context !is android.app.Activity) {
                    addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                }
            }
            context.startActivity(intent)
            Log.i(TAG, "Requested battery optimization exemption")
        } catch (e: Exception) {
            Log.e(TAG, "Failed to request battery optimization exemption", e)
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun AppScreen() {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val config by ConfigStore.configFlow(context).collectAsState(
        initial = AppConfig()
    )
    val running by ConfigStore.runningFlow(context).collectAsState(initial = false)

    var apiToken by rememberSaveable { mutableStateOf("") }
    var zoneId by rememberSaveable { mutableStateOf("") }
    var recordName by rememberSaveable { mutableStateOf("") }
    var timeoutSec by rememberSaveable { mutableStateOf("30") }
    var pollIntervalSec by rememberSaveable { mutableStateOf("60") }
    var verbose by rememberSaveable { mutableStateOf(false) }
    var multiRecord by rememberSaveable { mutableStateOf("error") }
    var showMenu by remember { mutableStateOf(false) }
    var errorMessage by rememberSaveable { mutableStateOf<String?>(null) }
    val clearError = { errorMessage = null }
    val multiRecordOptions = listOf(
        "error" to stringResource(R.string.multi_record_error),
        "first" to stringResource(R.string.multi_record_first),
        "all" to stringResource(R.string.multi_record_all)
    )
    val multiRecordLabel = multiRecordOptions.firstOrNull { it.first == multiRecord }?.second ?: multiRecord

    // Validation constants
    val minTimeout = 1L
    val maxTimeout = 300L
    val minPollInterval = 10L
    val maxPollInterval = 3600L

    // Validation function
    fun validateConfig(): String? {
        if (apiToken.trim().isEmpty()) {
            return context.getString(R.string.validation_api_token_required)
        }
        if (zoneId.trim().isEmpty()) {
            return context.getString(R.string.validation_zone_id_required)
        }
        if (recordName.trim().isEmpty()) {
            return context.getString(R.string.validation_record_name_required)
        }
        val timeout = timeoutSec.toLongOrNull()
        if (timeout == null || timeout < minTimeout || timeout > maxTimeout) {
            return context.getString(R.string.validation_timeout_range, minTimeout, maxTimeout)
        }
        val pollInterval = pollIntervalSec.toLongOrNull()
        if (pollInterval == null || pollInterval < minPollInterval || pollInterval > maxPollInterval) {
            return context.getString(R.string.validation_poll_interval_range, minPollInterval, maxPollInterval)
        }
        return null
    }

    LaunchedEffect(config) {
        apiToken = config.apiToken
        zoneId = config.zoneId
        recordName = config.recordName
        timeoutSec = config.timeoutSec.toString()
        pollIntervalSec = config.pollIntervalSec.toString()
        verbose = config.verbose
        multiRecord = config.multiRecord
    }

    Scaffold(
        topBar = { TopAppBar(title = { Text(stringResource(R.string.app_name)) }) }
    ) { padding ->
        Column(
            modifier = Modifier
                .padding(padding)
                .padding(16.dp)
                .fillMaxSize(),
            verticalArrangement = Arrangement.spacedBy(16.dp)
        ) {
            CloudflareConfigCard(
                apiToken = apiToken,
                onApiTokenChange = {
                    apiToken = it
                    clearError()
                },
                zoneId = zoneId,
                onZoneIdChange = {
                    zoneId = it
                    clearError()
                },
                recordName = recordName,
                onRecordNameChange = {
                    recordName = it
                    clearError()
                }
            )

            RuntimeConfigCard(
                timeoutSec = timeoutSec,
                onTimeoutChange = {
                    timeoutSec = it.filter { ch -> ch.isDigit() }
                    clearError()
                },
                pollIntervalSec = pollIntervalSec,
                onPollIntervalChange = {
                    pollIntervalSec = it.filter { ch -> ch.isDigit() }
                    clearError()
                },
                verbose = verbose,
                onVerboseChange = {
                    verbose = it
                    clearError()
                },
                multiRecord = multiRecord,
                multiRecordLabel = multiRecordLabel,
                multiRecordOptions = multiRecordOptions,
                showMenu = showMenu,
                onMenuShow = { showMenu = true },
                onMenuDismiss = { showMenu = false },
                onMultiRecordSelect = { option ->
                    multiRecord = option
                    clearError()
                    showMenu = false
                },
                context = context
            )

            StatusCard(
                running = running,
                config = config,
                errorMessage = errorMessage,
                onStartClick = {
                    errorMessage = validateConfig()
                    if (errorMessage == null) {
                        val cfg = AppConfig(
                            apiToken = apiToken.trim(),
                            zoneId = zoneId.trim(),
                            recordName = recordName.trim(),
                            timeoutSec = timeoutSec.toLong(),
                            pollIntervalSec = pollIntervalSec.toLong(),
                            verbose = verbose,
                            multiRecord = multiRecord
                        )
                        scope.launch(Dispatchers.IO) {
                            ConfigStore.saveConfig(context, cfg)
                            val configFile = ConfigToml.writeConfig(context, cfg)
                            withContext(Dispatchers.Main) {
                                val intent = Intent(context, Ipv6DdnsService::class.java).apply {
                                    action = Ipv6DdnsService.ACTION_START
                                    putExtra(Ipv6DdnsService.EXTRA_CONFIG_PATH, configFile.absolutePath)
                                }
                                context.startForegroundService(intent)
                            }
                        }
                    }
                },
                onStopClick = {
                    val intent = Intent(context, Ipv6DdnsService::class.java).apply {
                        action = Ipv6DdnsService.ACTION_STOP
                    }
                    context.startService(intent)
                }
            )
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun CloudflareConfigCard(
    apiToken: String,
    onApiTokenChange: (String) -> Unit,
    zoneId: String,
    onZoneIdChange: (String) -> Unit,
    recordName: String,
    onRecordNameChange: (String) -> Unit
) {
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp)
        ) {
            Text(stringResource(R.string.section_cloudflare))
            OutlinedTextField(
                value = apiToken,
                onValueChange = onApiTokenChange,
                label = { Text(stringResource(R.string.label_api_token)) },
                modifier = Modifier.fillMaxWidth(),
                visualTransformation = PasswordVisualTransformation()
            )
            OutlinedTextField(
                value = zoneId,
                onValueChange = onZoneIdChange,
                label = { Text(stringResource(R.string.label_zone_id)) },
                modifier = Modifier.fillMaxWidth()
            )
            OutlinedTextField(
                value = recordName,
                onValueChange = onRecordNameChange,
                label = { Text(stringResource(R.string.label_record_name)) },
                modifier = Modifier.fillMaxWidth()
            )
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun RuntimeConfigCard(
    timeoutSec: String,
    onTimeoutChange: (String) -> Unit,
    pollIntervalSec: String,
    onPollIntervalChange: (String) -> Unit,
    verbose: Boolean,
    onVerboseChange: (Boolean) -> Unit,
    multiRecord: String,
    multiRecordLabel: String,
    multiRecordOptions: List<Pair<String, String>>,
    showMenu: Boolean,
    onMenuShow: () -> Unit,
    onMenuDismiss: () -> Unit,
    onMultiRecordSelect: (String) -> Unit,
    context: android.content.Context
) {
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp)
        ) {
            Text(stringResource(R.string.section_runtime))
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically
            ) {
                OutlinedTextField(
                    value = timeoutSec,
                    onValueChange = onTimeoutChange,
                    label = { Text(stringResource(R.string.label_timeout)) },
                    modifier = Modifier.weight(1f)
                )
                Spacer(modifier = Modifier.width(12.dp))
                OutlinedTextField(
                    value = pollIntervalSec,
                    onValueChange = onPollIntervalChange,
                    label = { Text(stringResource(R.string.label_poll)) },
                    modifier = Modifier.weight(1f)
                )
            }

            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.SpaceBetween
            ) {
                Text(stringResource(R.string.label_verbose))
                Switch(
                    checked = verbose,
                    onCheckedChange = onVerboseChange
                )
            }

            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.SpaceBetween
            ) {
                Text(stringResource(R.string.label_multi_record))
                Button(onClick = onMenuShow) {
                    Text(multiRecordLabel)
                }
                DropdownMenu(
                    expanded = showMenu,
                    onDismissRequest = onMenuDismiss
                ) {
                    multiRecordOptions.forEach { (option, label) ->
                        DropdownMenuItem(
                            text = { Text(label) },
                            onClick = { onMultiRecordSelect(option) }
                        )
                    }
                }
            }

            // Battery optimization exemption card
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M && isBatteryOptimizationEnabled(context)) {
                BatteryOptimizationCard(context = context)
            }
        }
    }
}

@Composable
fun BatteryOptimizationCard(context: android.content.Context) {
    Card(
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.tertiaryContainer
        ),
        modifier = Modifier.fillMaxWidth()
    ) {
        Column(
            modifier = Modifier.padding(12.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp)
        ) {
            Text(
                "Battery optimization is enabled",
                color = MaterialTheme.colorScheme.onTertiaryContainer,
                style = MaterialTheme.typography.bodyMedium
            )
            Text(
                "This may cause the service to stop unexpectedly. We recommend disabling battery optimization for this app.",
                color = MaterialTheme.colorScheme.onTertiaryContainer,
                style = MaterialTheme.typography.bodySmall
            )
            Button(
                onClick = { requestBatteryOptimizationExemption(context) },
                modifier = Modifier.fillMaxWidth()
            ) {
                Text("Disable Battery Optimization")
            }
        }
    }
}

@Composable
fun StatusCard(
    running: Boolean,
    config: AppConfig,
    errorMessage: String?,
    onStartClick: () -> Unit,
    onStopClick: () -> Unit
) {
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp)
        ) {
            Text(
                stringResource(
                    if (running) R.string.status_running else R.string.status_stopped
                ),
                fontWeight = FontWeight.Bold
            )
            if (config.lastSyncTime > 0) {
                val dateFormat = SimpleDateFormat("yyyy-MM-dd HH:mm:ss", Locale.getDefault())
                val syncTime = dateFormat.format(Date(config.lastSyncTime))
                Text(
                    stringResource(R.string.last_sync) + ": $syncTime",
                    style = MaterialTheme.typography.bodySmall
                )
            }
            errorMessage?.let { error ->
                Card(
                    colors = CardDefaults.cardColors(
                        containerColor = MaterialTheme.colorScheme.errorContainer
                    ),
                    modifier = Modifier.fillMaxWidth()
                ) {
                    Text(
                        text = error,
                        color = MaterialTheme.colorScheme.onErrorContainer,
                        modifier = Modifier.padding(12.dp),
                        style = MaterialTheme.typography.bodySmall
                    )
                }
            }
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(12.dp)
            ) {
                Button(
                    modifier = Modifier.weight(1f),
                    onClick = onStartClick
                ) {
                    Text(stringResource(R.string.action_start))
                }
                Button(
                    modifier = Modifier.weight(1f),
                    onClick = onStopClick
                ) {
                    Text(stringResource(R.string.action_stop))
                }
            }
        }
    }
}
