package dev.golammowla.dropit.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.golammowla.dropit.discovery.DiscoveredPeer
import dev.golammowla.dropit.transfer.TransferItem
import dev.golammowla.dropit.transfer.TransferStatus

enum class BottomTab {
    TRANSFERS, DEVICES, HISTORY, SETTINGS
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun MainAppScreen(
    currentTab: BottomTab,
    onTabSelected: (BottomTab) -> Unit,
    peers: List<DiscoveredPeer>,
    transfers: List<TransferItem>,
    onSelectPeer: (DiscoveredPeer) -> Unit,
    onSendFilesClicked: () -> Unit,
    onPauseTransfer: (String) -> Unit,
    onResumeTransfer: (String) -> Unit,
    onCancelTransfer: (String) -> Unit
) {
    val accentColor = Color(0xFF2563EB) // Blue accent

    Scaffold(
        topBar = {
            TopAppBar(
                title = {
                    Column {
                        Text(
                            text = "Drop It",
                            fontSize = 20.sp,
                            fontWeight = FontWeight.Bold
                        )
                        Text(
                            text = "Encrypted • TLS 1.3 • Same Wi-Fi",
                            fontSize = 12.sp,
                            color = Color(0xFF10B981) // Green
                        )
                    }
                },
                colors = TopAppBarDefaults.topAppBarColors(
                    containerColor = MaterialTheme.colorScheme.surface
                )
            )
        },
        bottomBar = {
            NavigationBar {
                NavigationBarItem(
                    selected = currentTab == BottomTab.TRANSFERS,
                    onClick = { onTabSelected(BottomTab.TRANSFERS) },
                    icon = { Icon(Icons.Default.Send, contentDescription = "Transfers") },
                    label = { Text("Transfers") }
                )
                NavigationBarItem(
                    selected = currentTab == BottomTab.DEVICES,
                    onClick = { onTabSelected(BottomTab.DEVICES) },
                    icon = { Icon(Icons.Default.Devices, contentDescription = "Devices") },
                    label = { Text("Devices") }
                )
                NavigationBarItem(
                    selected = currentTab == BottomTab.HISTORY,
                    onClick = { onTabSelected(BottomTab.HISTORY) },
                    icon = { Icon(Icons.Default.History, contentDescription = "History") },
                    label = { Text("History") }
                )
                NavigationBarItem(
                    selected = currentTab == BottomTab.SETTINGS,
                    onClick = { onTabSelected(BottomTab.SETTINGS) },
                    icon = { Icon(Icons.Default.Settings, contentDescription = "Settings") },
                    label = { Text("Settings") }
                )
            }
        },
        floatingActionButton = {
            if (currentTab == BottomTab.TRANSFERS || currentTab == BottomTab.DEVICES) {
                FloatingActionButton(
                    onClick = onSendFilesClicked,
                    containerColor = accentColor,
                    contentColor = Color.White
                ) {
                    Icon(Icons.Default.Add, contentDescription = "Send Files")
                }
            }
        }
    ) { innerPadding ->
        Box(
            modifier = Modifier
                .padding(innerPadding)
                .fillMaxSize()
        ) {
            when (currentTab) {
                BottomTab.TRANSFERS -> TransferQueueView(
                    transfers = transfers,
                    onPause = onPauseTransfer,
                    onResume = onResumeTransfer,
                    onCancel = onCancelTransfer
                )
                BottomTab.DEVICES -> DeviceListView(
                    peers = peers,
                    onSelectPeer = onSelectPeer
                )
                BottomTab.HISTORY -> HistoryView()
                BottomTab.SETTINGS -> SettingsView()
            }
        }
    }
}

@Composable
fun TransferQueueView(
    transfers: List<TransferItem>,
    onPause: (String) -> Unit,
    onResume: (String) -> Unit,
    onCancel: (String) -> Unit
) {
    if (transfers.isEmpty()) {
        Box(
            modifier = Modifier.fillMaxSize(),
            contentAlignment = Alignment.Center
        ) {
            Text(
                text = "No active transfers\nDrop or select files to send",
                color = Color.Gray,
                textAlign = androidx.compose.ui.text.style.TextAlign.Center
            )
        }
    } else {
        LazyColumn(
            modifier = Modifier
                .fillMaxSize()
                .padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp)
        ) {
            items(transfers, key = { it.transferId }) { item ->
                TransferCard(item, onPause, onResume, onCancel)
            }
        }
    }
}

@Composable
fun TransferCard(
    item: TransferItem,
    onPause: (String) -> Unit,
    onResume: (String) -> Unit,
    onCancel: (String) -> Unit
) {
    Card(
        shape = RoundedCornerShape(12.dp),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceVariant
        ),
        modifier = Modifier.fillMaxWidth()
    ) {
        Column(modifier = Modifier.padding(16.dp)) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically
            ) {
                Text(
                    text = item.fileName,
                    fontWeight = FontWeight.SemiBold,
                    fontSize = 16.sp,
                    maxLines = 1
                )
                Text(
                    text = item.status.name,
                    fontSize = 12.sp,
                    fontWeight = FontWeight.Bold,
                    color = when (item.status) {
                        TransferStatus.COMPLETED -> Color(0xFF10B981)
                        TransferStatus.SENDING -> Color(0xFF2563EB)
                        TransferStatus.PAUSED -> Color(0xFFF59E0B)
                        TransferStatus.FAILED -> Color(0xFFEF4444)
                        TransferStatus.CANCELLED -> Color.Gray
                        TransferStatus.QUEUED -> Color.Gray
                    }
                )
            }

            Spacer(modifier = Modifier.height(8.dp))

            val progress = if (item.totalBytes > 0) {
                (item.bytesTransferred.toFloat() / item.totalBytes.toFloat()).coerceIn(0f, 1f)
            } else 0f

            LinearProgressIndicator(
                progress = { progress },
                modifier = Modifier
                    .fillMaxWidth()
                    .height(6.dp),
                color = Color(0xFF2563EB)
            )

            Spacer(modifier = Modifier.height(8.dp))

            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween
            ) {
                val speedMb = item.speedBytesPerSec.toDouble() / (1024.0 * 1024.0)
                Text(
                    text = "${formatBytes(item.bytesTransferred)} / ${formatBytes(item.totalBytes)}",
                    fontSize = 12.sp,
                    color = Color.Gray
                )
                if (item.status == TransferStatus.SENDING) {
                    Text(
                        text = String.format("%.2f MB/s", speedMb),
                        fontSize = 12.sp,
                        fontWeight = FontWeight.Medium
                    )
                }
            }

            Spacer(modifier = Modifier.height(12.dp))

            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.End
            ) {
                if (item.status == TransferStatus.SENDING) {
                    Button(
                        onClick = { onPause(item.transferId) },
                        shape = RoundedCornerShape(8.dp),
                        contentPadding = PaddingValues(horizontal = 12.dp, vertical = 6.dp)
                    ) {
                        Text("Pause", fontSize = 12.sp)
                    }
                } else if (item.status == TransferStatus.PAUSED) {
                    Button(
                        onClick = { onResume(item.transferId) },
                        shape = RoundedCornerShape(8.dp),
                        contentPadding = PaddingValues(horizontal = 12.dp, vertical = 6.dp)
                    ) {
                        Text("Resume", fontSize = 12.sp)
                    }
                }

                Spacer(modifier = Modifier.width(8.dp))

                if (item.status == TransferStatus.SENDING || item.status == TransferStatus.PAUSED || item.status == TransferStatus.QUEUED) {
                    OutlinedButton(
                        onClick = { onCancel(item.transferId) },
                        shape = RoundedCornerShape(8.dp),
                        contentPadding = PaddingValues(horizontal = 12.dp, vertical = 6.dp)
                    ) {
                        Text("Cancel", fontSize = 12.sp, color = Color(0xFFEF4444))
                    }
                }
            }
        }
    }
}

@Composable
fun DeviceListView(
    peers: List<DiscoveredPeer>,
    onSelectPeer: (DiscoveredPeer) -> Unit
) {
    if (peers.isEmpty()) {
        Box(
            modifier = Modifier.fillMaxSize(),
            contentAlignment = Alignment.Center
        ) {
            Text("Searching for nearby devices on Wi-Fi...", color = Color.Gray)
        }
    } else {
        LazyColumn(
            modifier = Modifier
                .fillMaxSize()
                .padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp)
        ) {
            items(peers, key = { it.id }) { peer ->
                Card(
                    shape = RoundedCornerShape(12.dp),
                    modifier = Modifier
                        .fillMaxWidth()
                        .clickable { onSelectPeer(peer) }
                ) {
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(16.dp),
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        Box(
                            modifier = Modifier
                                .size(10.dp)
                                .background(Color(0xFF10B981), shape = RoundedCornerShape(5.dp))
                        )
                        Spacer(modifier = Modifier.width(12.dp))
                        Column(modifier = Modifier.weight(1f)) {
                            Text(peer.name, fontWeight = FontWeight.Bold)
                            Text("${peer.host}:${peer.port}", fontSize = 12.sp, color = Color.Gray)
                        }
                        if (peer.isFavorite) {
                            Icon(Icons.Default.Star, contentDescription = "Favorite", tint = Color(0xFFF59E0B))
                        }
                    }
                }
            }
        }
    }
}

@Composable
fun HistoryView() {
    Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        Text("No completed transfers in history", color = Color.Gray)
    }
}

@Composable
fun SettingsView() {
    Column(modifier = Modifier.padding(16.dp)) {
        Text("Drop It Settings", fontSize = 18.sp, fontWeight = FontWeight.Bold)
        Spacer(modifier = Modifier.height(16.dp))
        Text("Security: Strict TLS 1.3 + Android Keystore")
        Text("Discovery: Local Network mDNS (_dropit._tcp)")
        Text("Save Location: Downloads/DropIt")
    }
}

fun formatBytes(bytes: Long): String {
    if (bytes <= 0) return "0 B"
    val units = arrayOf("B", "KB", "MB", "GB", "TB")
    val digitGroups = (Math.log10(bytes.toDouble()) / Math.log10(1024.0)).toInt()
    return String.format("%.1f %s", bytes / Math.pow(1024.0, digitGroups.toDouble()), units[digitGroups])
}
