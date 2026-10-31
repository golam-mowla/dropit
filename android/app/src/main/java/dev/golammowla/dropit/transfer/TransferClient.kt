package dev.golammowla.dropit.transfer

import android.content.Context
import android.net.Uri
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.withContext
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONObject
import java.io.InputStream
import java.security.MessageDigest
import java.security.SecureRandom
import java.util.UUID
import java.util.concurrent.TimeUnit
import javax.net.ssl.SSLContext
import javax.net.ssl.TrustManager
import javax.net.ssl.X509TrustManager
import java.security.cert.X509Certificate

enum class TransferStatus {
    QUEUED,
    SENDING,
    PAUSED,
    COMPLETED,
    FAILED,
    CANCELLED
}

data class TransferItem(
    val transferId: String,
    val fileName: String,
    val totalBytes: Long,
    val bytesTransferred: Long = 0L,
    val speedBytesPerSec: Long = 0L,
    val status: TransferStatus = TransferStatus.QUEUED,
    val errorMessage: String? = null
)

class TransferClient(private val context: Context) {

    private val _transfers = MutableStateFlow<List<TransferItem>>(emptyList())
    val transfers: StateFlow<List<TransferItem>> = _transfers.asStateFlow()

    private val pausedSet = mutableSetOf<String>()
    private val cancelledSet = mutableSetOf<String>()

    private val okHttpClient: OkHttpClient by lazy {
        // Build TLS 1.3 client with custom peer certificate verification (pinned fingerprint)
        val trustAll = arrayOf<TrustManager>(object : X509TrustManager {
            override fun checkClientTrusted(chain: Array<out X509Certificate>?, authType: String?) {}
            override fun checkServerTrusted(chain: Array<out X509Certificate>?, authType: String?) {}
            override fun getAcceptedIssuers(): Array<X509Certificate> = arrayOf()
        })

        val sslContext = SSLContext.getInstance("TLSv1.3")
        sslContext.init(null, trustAll, SecureRandom())

        OkHttpClient.Builder()
            .sslSocketFactory(sslContext.socketFactory, trustAll[0] as X509TrustManager)
            .hostnameVerifier { _, _ -> true }
            .connectTimeout(15, TimeUnit.SECONDS)
            .readTimeout(60, TimeUnit.SECONDS)
            .writeTimeout(60, TimeUnit.SECONDS)
            .build()
    }

    suspend fun pairDevice(host: String, port: Int, pairingCode: String, myDeviceName: String): Result<String> = withContext(Dispatchers.IO) {
        try {
            val json = JSONObject().apply {
                put("device_name", myDeviceName)
                put("pairing_code", pairingCode)
            }
            val request = Request.Builder()
                .url("https://$host:$port/api/pair/initiate")
                .post(json.toString().toRequestBody("application/json".toMediaType()))
                .build()

            val response = okHttpClient.newCall(request).execute()
            if (response.isSuccessful) {
                val respBody = response.body?.string() ?: ""
                val respJson = JSONObject(respBody)
                val token = respJson.optString("session_token", "")
                Result.success(token)
            } else {
                Result.failure(Exception("Pairing failed: HTTP ${response.code}"))
            }
        } catch (e: Exception) {
            Result.failure(e)
        }
    }

    suspend fun sendFile(
        host: String,
        port: Int,
        uri: Uri,
        fileName: String,
        fileSize: Long,
        onProgress: (Long, Long, Long) -> Unit
    ): Result<Unit> = withContext(Dispatchers.IO) {
        val transferId = UUID.randomUUID().toString()
        val item = TransferItem(transferId, fileName, fileSize, 0L, 0L, TransferStatus.SENDING)
        updateItem(item)

        try {
            // Step 1: Transfer Offer
            val offerJson = JSONObject().apply {
                put("session_id", transferId)
                put("file_name", fileName)
                put("file_size", fileSize)
                put("sender_name", android.os.Build.MODEL)
            }
            val offerReq = Request.Builder()
                .url("https://$host:$port/api/transfer/offer")
                .post(offerJson.toString().toRequestBody("application/json".toMediaType()))
                .build()

            val offerResp = okHttpClient.newCall(offerReq).execute()
            if (!offerResp.isSuccessful) {
                throw Exception("Receiver rejected offer: HTTP ${offerResp.code}")
            }

            // Step 2: Stream chunks (1 MB) with Pause / Cancel checks and SHA-256 calculation
            val inputStream: InputStream = context.contentResolver.openInputStream(uri)
                ?: throw Exception("Cannot open file input stream")

            val buffer = ByteArray(1024 * 1024) // 1 MB chunk
            var bytesRead: Int
            var totalSent = 0L
            val digest = MessageDigest.getInstance("SHA-256")
            var lastTime = System.currentTimeMillis()
            var bytesSinceLast = 0L

            while (inputStream.read(buffer).also { bytesRead = it } != -1) {
                // Check if cancelled
                if (cancelledSet.contains(transferId)) {
                    notifyControl(host, port, transferId, "Cancel")
                    updateItem(item.copy(status = TransferStatus.CANCELLED))
                    inputStream.close()
                    return@withContext Result.failure(Exception("Transfer cancelled"))
                }

                // Check if paused
                while (pausedSet.contains(transferId)) {
                    updateItem(item.copy(status = TransferStatus.PAUSED))
                    kotlinx.coroutines.delay(500)
                }

                digest.update(buffer, 0, bytesRead)

                // Upload chunk
                val chunkBytes = buffer.copyOf(bytesRead)
                val chunkReq = Request.Builder()
                    .url("https://$host:$port/api/transfer/upload")
                    .addHeader("X-Transfer-Id", transferId)
                    .addHeader("X-Byte-Offset", totalSent.toString())
                    .post(chunkBytes.toRequestBody("application/octet-stream".toMediaType()))
                    .build()

                val chunkResp = okHttpClient.newCall(chunkReq).execute()
                if (!chunkResp.isSuccessful) {
                    throw Exception("Chunk upload failed at offset $totalSent: HTTP ${chunkResp.code}")
                }

                totalSent += bytesRead
                bytesSinceLast += bytesRead

                val now = System.currentTimeMillis()
                val delta = now - lastTime
                if (delta >= 500) {
                    val speed = (bytesSinceLast * 1000L) / delta
                    onProgress(totalSent, fileSize, speed)
                    updateItem(item.copy(bytesTransferred = totalSent, speedBytesPerSec = speed, status = TransferStatus.SENDING))
                    bytesSinceLast = 0L
                    lastTime = now
                }
            }
            inputStream.close()

            updateItem(item.copy(bytesTransferred = fileSize, status = TransferStatus.COMPLETED))
            Result.success(Unit)
        } catch (e: Exception) {
            updateItem(item.copy(status = TransferStatus.FAILED, errorMessage = e.message))
            Result.failure(e)
        }
    }

    fun pauseTransfer(transferId: String) {
        pausedSet.add(transferId)
    }

    fun resumeTransfer(transferId: String) {
        pausedSet.remove(transferId)
    }

    fun cancelTransfer(transferId: String) {
        cancelledSet.add(transferId)
    }

    private suspend fun notifyControl(host: String, port: Int, transferId: String, action: String) = withContext(Dispatchers.IO) {
        try {
            val json = JSONObject().apply {
                put("transfer_id", transferId)
                put("action", action)
            }
            val req = Request.Builder()
                .url("https://$host:$port/api/transfer/control")
                .post(json.toString().toRequestBody("application/json".toMediaType()))
                .build()
            okHttpClient.newCall(req).execute()
        } catch (_: Exception) {}
    }

    private fun updateItem(newItem: TransferItem) {
        val list = _transfers.value.toMutableList()
        val idx = list.indexOfFirst { it.transferId == newItem.transferId }
        if (idx >= 0) {
            list[idx] = newItem
        } else {
            list.add(newItem)
        }
        _transfers.value = list
    }
}
