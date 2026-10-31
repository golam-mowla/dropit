package dev.golammowla.dropit

import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.provider.OpenableColumns
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.*
import androidx.lifecycle.lifecycleScope
import dev.golammowla.dropit.discovery.DiscoveredPeer
import dev.golammowla.dropit.discovery.DiscoveryManager
import dev.golammowla.dropit.service.DropItTransferService
import dev.golammowla.dropit.transfer.TransferClient
import dev.golammowla.dropit.ui.BottomTab
import dev.golammowla.dropit.ui.MainAppScreen
import kotlinx.coroutines.launch

class MainActivity : ComponentActivity() {

    private lateinit var discoveryManager: DiscoveryManager
    private lateinit var transferClient: TransferClient
    private var selectedPeer: DiscoveredPeer? = null

    private val filePickerLauncher = registerForActivityResult(
        ActivityResultContracts.OpenMultipleDocuments()
    ) { uris: List<Uri> ->
        if (uris.isNotEmpty()) {
            val peer = selectedPeer
            if (peer != null) {
                sendSelectedFiles(peer, uris)
            } else {
                Toast.makeText(this, "Please select a target device first", Toast.LENGTH_SHORT).show()
            }
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        discoveryManager = DiscoveryManager(this)
        transferClient = TransferClient(this)

        handleIncomingShareIntent(intent)

        setContent {
            var currentTab by remember { mutableStateOf(BottomTab.TRANSFERS) }
            val peersMap by discoveryManager.peers.collectAsState()
            val transfers by transferClient.transfers.collectAsState()

            MainAppScreen(
                currentTab = currentTab,
                onTabSelected = { currentTab = it },
                peers = peersMap.values.toList(),
                transfers = transfers,
                onSelectPeer = { peer ->
                    selectedPeer = peer
                    Toast.makeText(this, "Target: ${peer.name}", Toast.LENGTH_SHORT).show()
                },
                onSendFilesClicked = {
                    filePickerLauncher.launch(arrayOf("*/*"))
                },
                onPauseTransfer = { id -> transferClient.pauseTransfer(id) },
                onResumeTransfer = { id -> transferClient.resumeTransfer(id) },
                onCancelTransfer = { id -> transferClient.cancelTransfer(id) }
            )
        }
    }

    override fun onResume() {
        super.onResume()
        discoveryManager.startDiscovery()
    }

    override fun onPause() {
        super.onPause()
        discoveryManager.stopDiscovery()
    }

    private fun handleIncomingShareIntent(intent: Intent?) {
        if (intent == null) return
        when (intent.action) {
            Intent.ACTION_SEND -> {
                (intent.getParcelableExtra<Uri>(Intent.EXTRA_STREAM))?.let { uri ->
                    Toast.makeText(this, "Received file via Share Sheet. Choose target device.", Toast.LENGTH_LONG).show()
                }
            }
            Intent.ACTION_SEND_MULTIPLE -> {
                intent.getParcelableArrayListExtra<Uri>(Intent.EXTRA_STREAM)?.let { uris ->
                    Toast.makeText(this, "Received ${uris.size} files via Share Sheet.", Toast.LENGTH_LONG).show()
                }
            }
        }
    }

    private fun sendSelectedFiles(peer: DiscoveredPeer, uris: List<Uri>) {
        lifecycleScope.launch {
            DropItTransferService.startService(this@MainActivity, "Sending ${uris.size} file(s) to ${peer.name}...")

            for (uri in uris) {
                val (name, size) = queryFileInfo(uri)
                transferClient.sendFile(peer.host, peer.port, uri, name, size) { _, _, _ -> }
            }

            DropItTransferService.stopService(this@MainActivity)
        }
    }

    private fun queryFileInfo(uri: Uri): Pair<String, Long> {
        var name = "unknown_file"
        var size = 0L
        contentResolver.query(uri, null, null, null, null)?.use { cursor ->
            val nameIndex = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
            val sizeIndex = cursor.getColumnIndex(OpenableColumns.SIZE)
            if (cursor.moveToFirst()) {
                if (nameIndex != -1) name = cursor.getString(nameIndex) ?: name
                if (sizeIndex != -1) size = cursor.getLong(sizeIndex)
            }
        }
        return Pair(name, size)
    }
}
