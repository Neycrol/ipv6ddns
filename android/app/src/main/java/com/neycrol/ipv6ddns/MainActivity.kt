package com.neycrol.ipv6ddns

import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
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
import androidx.compose.runtime.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import com.neycrol.ipv6ddns.data.AppConfig
import com.neycrol.ipv6ddns.data.ConfigStore
import com.neycrol.ipv6ddns.data.ConfigToml
import com.neycrol.ipv6ddns.service.Ipv6DdnsService
import kotlinx.coroutines.runBlocking

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent { AppScreen() }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun AppScreen() {
    val context = LocalContext.current
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
        topBar = { TopAppBar(title = { Text("ipv6ddns") }) }
    ) { padding ->
        Column(
            modifier = Modifier
                .padding(padding)
                .padding(16.dp)
                .fillMaxSize(),
            verticalArrangement = Arrangement.spacedBy(16.dp)
        ) {
            Card(modifier = Modifier.fillMaxWidth()) {
                Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text("Cloudflare")
                    OutlinedTextField(
                        value = apiToken,
                        onValueChange = { apiToken = it },
                        label = { Text("API Token") },
                        modifier = Modifier.fillMaxWidth(),
                        visualTransformation = PasswordVisualTransformation()
                    )
                    OutlinedTextField(
                        value = zoneId,
                        onValueChange = { zoneId = it },
                        label = { Text("Zone ID") },
                        modifier = Modifier.fillMaxWidth()
                    )
                    OutlinedTextField(
                        value = recordName,
                        onValueChange = { recordName = it },
                        label = { Text("Record Name") },
                        modifier = Modifier.fillMaxWidth()
                    )
                }
            }

            Card(modifier = Modifier.fillMaxWidth()) {
                Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text("Runtime")
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        OutlinedTextField(
                            value = timeoutSec,
                            onValueChange = { timeoutSec = it.filter { ch -> ch.isDigit() } },
                            label = { Text("Timeout (s)") },
                            modifier = Modifier.weight(1f)
                        )
                        Spacer(modifier = Modifier.width(12.dp))
                        OutlinedTextField(
                            value = pollIntervalSec,
                            onValueChange = { pollIntervalSec = it.filter { ch -> ch.isDigit() } },
                            label = { Text("Poll (s)") },
                            modifier = Modifier.weight(1f)
                        )
                    }

                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.SpaceBetween
                    ) {
                        Text("Verbose")
                        Switch(checked = verbose, onCheckedChange = { verbose = it })
                    }

                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.SpaceBetween
                    ) {
                        Text("Multi-record")
                        Button(onClick = { showMenu = true }) {
                            Text(multiRecord)
                        }
                        DropdownMenu(expanded = showMenu, onDismissRequest = { showMenu = false }) {
                            listOf("error", "first", "all").forEach { option ->
                                DropdownMenuItem(
                                    text = { Text(option) },
                                    onClick = {
                                        multiRecord = option
                                        showMenu = false
                                    }
                                )
                            }
                        }
                    }
                }
            }

            Card(modifier = Modifier.fillMaxWidth()) {
                Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text(if (running) "Status: Running" else "Status: Stopped")
                    Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                        Button(
                            onClick = {
                                val cfg = AppConfig(
                                    apiToken = apiToken.trim(),
                                    zoneId = zoneId.trim(),
                                    recordName = recordName.trim(),
                                    timeoutSec = timeoutSec.toLongOrNull() ?: 30,
                                    pollIntervalSec = pollIntervalSec.toLongOrNull() ?: 60,
                                    verbose = verbose,
                                    multiRecord = multiRecord
                                )
                                runBlocking { ConfigStore.saveConfig(context, cfg) }
                                val configFile = ConfigToml.writeConfig(context, cfg)
                                val intent = Intent(context, Ipv6DdnsService::class.java).apply {
                                    action = Ipv6DdnsService.ACTION_START
                                    putExtra(Ipv6DdnsService.EXTRA_CONFIG_PATH, configFile.absolutePath)
                                }
                                context.startForegroundService(intent)
                            }
                        ) {
                            Text("Start")
                        }
                        Button(
                            onClick = {
                                val intent = Intent(context, Ipv6DdnsService::class.java).apply {
                                    action = Ipv6DdnsService.ACTION_STOP
                                }
                                context.startService(intent)
                            }
                        ) {
                            Text("Stop")
                        }
                    }
                }
            }
        }
    }
}
