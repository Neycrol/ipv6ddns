package com.neycrol.ipv6ddns

import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import android.util.Log
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LargeTopAppBar
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.material3.TopAppBarScrollBehavior
import androidx.compose.material3.rememberTopAppBarState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import com.neycrol.ipv6ddns.data.AppConfig
import com.neycrol.ipv6ddns.data.ConfigStore
import com.neycrol.ipv6ddns.data.ConfigToml
import com.neycrol.ipv6ddns.service.Ipv6DdnsService
import com.neycrol.ipv6ddns.ui.AppColors
import com.neycrol.ipv6ddns.ui.Ipv6DdnsTheme
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

private const val TAG = "ipv6ddns/MainActivity"

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        setContent {
            Ipv6DdnsTheme {
                AppScreen()
            }
        }
    }
}

private fun isBatteryOptimizationEnabled(context: android.content.Context): Boolean {
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
        val pm = context.getSystemService(android.content.Context.POWER_SERVICE)
                as android.os.PowerManager
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

private const val MIN_TIMEOUT = 1L
private const val MAX_TIMEOUT = 300L
private const val MIN_POLL_INTERVAL = 10L
private const val MAX_POLL_INTERVAL = 3600L

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun AppScreen() {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val config by ConfigStore.configFlow(context).collectAsState(initial = AppConfig())
    val running by ConfigStore.runningFlow(context).collectAsState(initial = false)

    var apiToken by rememberSaveable { mutableStateOf("") }
    var zoneId by rememberSaveable { mutableStateOf("") }
    var recordName by rememberSaveable { mutableStateOf("") }
    var timeoutSec by rememberSaveable { mutableStateOf("30") }
    var pollIntervalSec by rememberSaveable { mutableStateOf("60") }
    var verbose by rememberSaveable { mutableStateOf(false) }
    var multiRecord by rememberSaveable { mutableStateOf("error") }
    var showMenu by rememberSaveable { mutableStateOf(false) }
    var errorMessage by rememberSaveable { mutableStateOf<String?>(null) }
    val clearError = { errorMessage = null }
    val multiRecordOptions = listOf(
        "error" to stringResource(R.string.multi_record_error),
        "first" to stringResource(R.string.multi_record_first),
        "all" to stringResource(R.string.multi_record_all)
    )
    val multiRecordLabel =
        multiRecordOptions.firstOrNull { it.first == multiRecord }?.second ?: multiRecord

    fun validateConfig(): String? {
        if (apiToken.trim().isEmpty())
            return context.getString(R.string.validation_api_token_required)
        if (zoneId.trim().isEmpty())
            return context.getString(R.string.validation_zone_id_required)
        if (recordName.trim().isEmpty())
            return context.getString(R.string.validation_record_name_required)
        val timeout = timeoutSec.toLongOrNull()
        if (timeout == null || timeout < MIN_TIMEOUT || timeout > MAX_TIMEOUT)
            return context.getString(R.string.validation_timeout_range, MIN_TIMEOUT, MAX_TIMEOUT)
        val poll = pollIntervalSec.toLongOrNull()
        if (poll == null || poll < MIN_POLL_INTERVAL || poll > MAX_POLL_INTERVAL)
            return context.getString(
                R.string.validation_poll_interval_range, MIN_POLL_INTERVAL, MAX_POLL_INTERVAL
            )
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

    val scrollBehavior = TopAppBarDefaults.exitUntilCollapsedScrollBehavior(
        rememberTopAppBarState()
    )

    Scaffold(
        modifier = Modifier.nestedScroll(scrollBehavior.nestedScrollConnection),
        topBar = { AppTopBar(scrollBehavior) }
    ) { padding ->
        Column(
            modifier = Modifier
                .padding(padding)
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp, vertical = 8.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp)
        ) {
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
                                val intent =
                                    Intent(context, Ipv6DdnsService::class.java).apply {
                                        action = Ipv6DdnsService.ACTION_START
                                        putExtra(
                                            Ipv6DdnsService.EXTRA_CONFIG_PATH,
                                            configFile.absolutePath
                                        )
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

            CloudflareConfigCard(
                apiToken = apiToken,
                onApiTokenChange = { apiToken = it; clearError() },
                zoneId = zoneId,
                onZoneIdChange = { zoneId = it; clearError() },
                recordName = recordName,
                onRecordNameChange = { recordName = it; clearError() },
                enabled = !running
            )

            RuntimeConfigCard(
                timeoutSec = timeoutSec,
                onTimeoutChange = { timeoutSec = it.filter { ch -> ch.isDigit() }; clearError() },
                pollIntervalSec = pollIntervalSec,
                onPollIntervalChange = {
                    pollIntervalSec = it.filter { ch -> ch.isDigit() }; clearError()
                },
                verbose = verbose,
                onVerboseChange = { verbose = it; clearError() },
                multiRecordLabel = multiRecordLabel,
                multiRecordOptions = multiRecordOptions,
                showMenu = showMenu,
                onMenuShow = { showMenu = true },
                onMenuDismiss = { showMenu = false },
                onMultiRecordSelect = { multiRecord = it; clearError(); showMenu = false },
                enabled = !running,
                context = context
            )

            Spacer(modifier = Modifier.height(8.dp))
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun AppTopBar(scrollBehavior: TopAppBarScrollBehavior) {
    LargeTopAppBar(
        title = { Text(stringResource(R.string.app_name)) },
        scrollBehavior = scrollBehavior,
        colors = TopAppBarDefaults.largeTopAppBarColors(
            containerColor = MaterialTheme.colorScheme.surface,
            scrolledContainerColor = MaterialTheme.colorScheme.surfaceContainerHighest,
        )
    )
}

// --- Status Card -------------------------------------------------------------

@Composable
fun StatusCard(
    running: Boolean,
    config: AppConfig,
    errorMessage: String?,
    onStartClick: () -> Unit,
    onStopClick: () -> Unit
) {
    val dark = isSystemInDarkTheme()
    val statusColor by animateColorAsState(
        targetValue = if (running) {
            if (dark) AppColors.StatusRunningDark else AppColors.StatusRunning
        } else {
            if (dark) AppColors.StatusStoppedDark else AppColors.StatusStopped
        },
        animationSpec = tween(400),
        label = "statusColor"
    )

    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceContainerLow
        )
    ) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp)
        ) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(10.dp)
            ) {
                Box(
                    modifier = Modifier
                        .size(10.dp)
                        .clip(CircleShape)
                        .background(statusColor)
                )
                Text(
                    text = stringResource(
                        if (running) R.string.status_running else R.string.status_stopped
                    ),
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.SemiBold,
                    color = statusColor
                )
            }

            val syncText = if (config.lastSyncTime > 0) {
                val fmt = SimpleDateFormat("yyyy-MM-dd HH:mm:ss", Locale.getDefault())
                stringResource(R.string.last_sync) + ": " + fmt.format(Date(config.lastSyncTime))
            } else {
                stringResource(R.string.never_synced)
            }
            Text(
                text = syncText,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )

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
                    onClick = onStartClick,
                    enabled = !running
                ) {
                    Text(stringResource(R.string.action_start))
                }
                OutlinedButton(
                    modifier = Modifier.weight(1f),
                    onClick = onStopClick,
                    enabled = running,
                    colors = ButtonDefaults.outlinedButtonColors(
                        contentColor = MaterialTheme.colorScheme.error
                    )
                ) {
                    Text(stringResource(R.string.action_stop))
                }
            }
        }
    }
}

// --- Cloudflare Config Card --------------------------------------------------

@Composable
fun CloudflareConfigCard(
    apiToken: String,
    onApiTokenChange: (String) -> Unit,
    zoneId: String,
    onZoneIdChange: (String) -> Unit,
    recordName: String,
    onRecordNameChange: (String) -> Unit,
    enabled: Boolean
) {
    var tokenVisible by rememberSaveable { mutableStateOf(false) }

    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceContainerLow
        )
    ) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp)
        ) {
            SectionTitle(stringResource(R.string.section_cloudflare))

            OutlinedTextField(
                value = apiToken,
                onValueChange = onApiTokenChange,
                label = { Text(stringResource(R.string.label_api_token)) },
                modifier = Modifier.fillMaxWidth(),
                enabled = enabled,
                singleLine = true,
                visualTransformation = if (tokenVisible) {
                    VisualTransformation.None
                } else {
                    PasswordVisualTransformation()
                },
                trailingIcon = {
                    IconButton(onClick = { tokenVisible = !tokenVisible }) {
                        Icon(
                            painter = painterResource(
                                if (tokenVisible) android.R.drawable.ic_menu_view
                                else android.R.drawable.ic_secure
                            ),
                            contentDescription = stringResource(
                                if (tokenVisible) R.string.label_hide_token
                                else R.string.label_show_token
                            )
                        )
                    }
                },
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password)
            )
            OutlinedTextField(
                value = zoneId,
                onValueChange = onZoneIdChange,
                label = { Text(stringResource(R.string.label_zone_id)) },
                modifier = Modifier.fillMaxWidth(),
                enabled = enabled,
                singleLine = true,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Ascii)
            )
            OutlinedTextField(
                value = recordName,
                onValueChange = onRecordNameChange,
                label = { Text(stringResource(R.string.label_record_name)) },
                modifier = Modifier.fillMaxWidth(),
                enabled = enabled,
                singleLine = true,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri)
            )
        }
    }
}

// --- Runtime Config Card -----------------------------------------------------

@Composable
fun RuntimeConfigCard(
    timeoutSec: String,
    onTimeoutChange: (String) -> Unit,
    pollIntervalSec: String,
    onPollIntervalChange: (String) -> Unit,
    verbose: Boolean,
    onVerboseChange: (Boolean) -> Unit,
    multiRecordLabel: String,
    multiRecordOptions: List<Pair<String, String>>,
    showMenu: Boolean,
    onMenuShow: () -> Unit,
    onMenuDismiss: () -> Unit,
    onMultiRecordSelect: (String) -> Unit,
    enabled: Boolean,
    context: android.content.Context
) {
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceContainerLow
        )
    ) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp)
        ) {
            SectionTitle(stringResource(R.string.section_runtime))

            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically
            ) {
                OutlinedTextField(
                    value = timeoutSec,
                    onValueChange = onTimeoutChange,
                    label = { Text(stringResource(R.string.label_timeout)) },
                    modifier = Modifier.weight(1f),
                    enabled = enabled,
                    singleLine = true,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number)
                )
                Spacer(modifier = Modifier.width(12.dp))
                OutlinedTextField(
                    value = pollIntervalSec,
                    onValueChange = onPollIntervalChange,
                    label = { Text(stringResource(R.string.label_poll)) },
                    modifier = Modifier.weight(1f),
                    enabled = enabled,
                    singleLine = true,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number)
                )
            }

            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.SpaceBetween
            ) {
                Text(
                    stringResource(R.string.label_verbose),
                    style = MaterialTheme.typography.bodyLarge
                )
                Switch(
                    checked = verbose,
                    onCheckedChange = onVerboseChange,
                    enabled = enabled
                )
            }

            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.SpaceBetween
            ) {
                Text(
                    stringResource(R.string.label_multi_record),
                    style = MaterialTheme.typography.bodyLarge
                )
                Box {
                    OutlinedButton(onClick = onMenuShow, enabled = enabled) {
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
            }

            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M
                && isBatteryOptimizationEnabled(context)
            ) {
                BatteryOptimizationCard(context = context)
            }
        }
    }
}

// --- Battery Optimization Card -----------------------------------------------

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
                stringResource(R.string.battery_optimization_title),
                color = MaterialTheme.colorScheme.onTertiaryContainer,
                style = MaterialTheme.typography.bodyMedium,
                fontWeight = FontWeight.Medium
            )
            Text(
                stringResource(R.string.battery_optimization_description),
                color = MaterialTheme.colorScheme.onTertiaryContainer,
                style = MaterialTheme.typography.bodySmall
            )
            OutlinedButton(
                onClick = { requestBatteryOptimizationExemption(context) },
                modifier = Modifier.fillMaxWidth()
            ) {
                Text(stringResource(R.string.battery_optimization_action))
            }
        }
    }
}

// --- Shared Components -------------------------------------------------------

@Composable
private fun SectionTitle(text: String) {
    Text(
        text = text,
        style = MaterialTheme.typography.titleSmall,
        fontWeight = FontWeight.SemiBold,
        color = MaterialTheme.colorScheme.primary
    )
}
