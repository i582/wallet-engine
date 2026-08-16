package com.example.tonwallet.data

import com.example.tonwallet.BuildConfig
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.ton.wallet.engine.HttpHeader
import org.ton.wallet.engine.HttpHostErrorKind
import org.ton.wallet.engine.HttpHostException
import org.ton.wallet.engine.HttpMethod
import org.ton.wallet.engine.HttpRequest
import org.ton.wallet.engine.HttpRequestId
import org.ton.wallet.engine.HttpResponse
import org.ton.wallet.engine.WalletHttpHost
import java.io.ByteArrayOutputStream
import java.io.IOException
import java.net.HttpURLConnection
import java.net.SocketTimeoutException
import java.net.URL
import java.net.UnknownHostException
import java.util.LinkedHashSet
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean

class AndroidWalletHttpHost : WalletHttpHost {
    private val lock = Any()
    private val active = mutableMapOf<ULong, HttpURLConnection>()
    private val cancelledBeforeStart = LinkedHashSet<ULong>()

    override suspend fun executeHttp(request: HttpRequest): HttpResponse = withContext(Dispatchers.IO) {
        val url = prepare(request)
        val connection = (url.openConnection() as HttpURLConnection).apply {
            instanceFollowRedirects = false
            requestMethod = when (request.method) {
                HttpMethod.GET -> "GET"
                HttpMethod.POST -> "POST"
            }
            val timeoutMillis = request.timeoutMs.toInt()
            connectTimeout = timeoutMillis
            readTimeout = timeoutMillis
            useCaches = false
            request.headers.forEach { header -> setRequestProperty(header.name, header.value) }
            if (BuildConfig.TONCENTER_TESTNET_API_KEY.isNotBlank() && url.host == TESTNET_HOST) {
                setRequestProperty("X-API-Key", BuildConfig.TONCENTER_TESTNET_API_KEY)
            }
            if (request.body.isNotEmpty()) {
                doOutput = true
                setFixedLengthStreamingMode(request.body.size)
            }
        }

        synchronized(lock) {
            if (cancelledBeforeStart.remove(request.id.value)) {
                connection.disconnect()
                throw hostError(HttpHostErrorKind.CANCELLED, "Request was cancelled")
            }
            active[request.id.value] = connection
        }

        val deadlineExpired = AtomicBoolean(false)
        val deadline = deadlineScheduler.schedule(
            {
                deadlineExpired.set(true)
                connection.disconnect()
            },
            request.timeoutMs.toLong(),
            TimeUnit.MILLISECONDS,
        )

        try {
            if (request.body.isNotEmpty()) connection.outputStream.use { it.write(request.body) }
            val status = connection.responseCode
            if (status in 300..399) {
                throw hostError(HttpHostErrorKind.POLICY_VIOLATION, "HTTP redirects are not allowed")
            }
            val headers = boundedHeaders(connection)
            val input = if (status in 200..299) connection.inputStream else connection.errorStream
            val body = input?.use { stream ->
                val limit = MAX_RESPONSE_BODY_BYTES
                val output = ByteArrayOutputStream(minOf(limit, DEFAULT_BUFFER_SIZE))
                val buffer = ByteArray(DEFAULT_BUFFER_SIZE)
                var total = 0
                while (true) {
                    val count = stream.read(buffer)
                    if (count < 0) break
                    total += count
                    if (total > limit) {
                        throw hostError(HttpHostErrorKind.RESPONSE_TOO_LARGE, "Response body exceeded its limit")
                    }
                    output.write(buffer, 0, count)
                }
                output.toByteArray()
            } ?: byteArrayOf()
            val response = HttpResponse(
                status = status.toUShort(),
                headers = headers,
                body = body,
                // Redirect following is disabled above, so a non-redirect response
                // belongs to the exact request URL. HttpURLConnection can expose a
                // differently normalized URL string even when no redirect occurred.
                finalUrl = request.url,
            )
            if (deadlineExpired.get()) {
                throw hostError(HttpHostErrorKind.TIMEOUT, "Provider request timed out")
            }
            response
        } catch (error: CancellationException) {
            connection.disconnect()
            throw error
        } catch (error: HttpHostException) {
            throw error
        } catch (error: UnknownHostException) {
            throw hostError(HttpHostErrorKind.DNS, "Could not resolve the provider host")
        } catch (error: SocketTimeoutException) {
            throw hostError(HttpHostErrorKind.TIMEOUT, "Provider request timed out")
        } catch (error: IOException) {
            if (deadlineExpired.get()) {
                throw hostError(HttpHostErrorKind.TIMEOUT, "Provider request timed out")
            }
            if (synchronized(lock) { request.id.value !in active }) {
                throw hostError(HttpHostErrorKind.CANCELLED, "Request was cancelled")
            }
            throw hostError(
                HttpHostErrorKind.CONNECTION_LOST,
                error.message ?: "Network request failed",
            )
        } catch (error: Throwable) {
            throw hostError(HttpHostErrorKind.OTHER, error.message ?: "HTTP host failed")
        } finally {
            deadline.cancel(false)
            synchronized(lock) { active.remove(request.id.value) }
            connection.disconnect()
        }
    }

    override suspend fun cancelHttp(requestId: HttpRequestId) {
        val connection = synchronized(lock) {
            active.remove(requestId.value) ?: run {
                cancelledBeforeStart += requestId.value
                while (cancelledBeforeStart.size > MAX_EARLY_CANCELLATIONS) {
                    cancelledBeforeStart.remove(cancelledBeforeStart.first())
                }
                null
            }
        }
        connection?.disconnect()
    }

    private fun prepare(request: HttpRequest): URL {
        val url = runCatching { URL(request.url) }.getOrElse {
            throw hostError(HttpHostErrorKind.POLICY_VIOLATION, "Request URL is invalid")
        }
        if (url.protocol != "https" || url.userInfo != null) {
            throw hostError(HttpHostErrorKind.POLICY_VIOLATION, "Only credential-free HTTPS URLs are allowed")
        }
        if (request.headers.any { it.name.equals("x-api-key", ignoreCase = true) }) {
            throw hostError(HttpHostErrorKind.POLICY_VIOLATION, "Credential headers are host-owned")
        }
        if (request.timeoutMs == 0UL || request.timeoutMs > MAX_TIMEOUT_MILLIS.toULong()) {
            throw hostError(HttpHostErrorKind.POLICY_VIOLATION, "Request timeout is invalid")
        }
        return url
    }

    private fun boundedHeaders(connection: HttpURLConnection): List<HttpHeader> {
        var size = 0
        return buildList {
            connection.headerFields.forEach { (name, values) ->
                if (name == null) return@forEach
                values.orEmpty().forEach { value ->
                    size += name.length + value.length + 4
                    if (size > MAX_RESPONSE_HEADER_BYTES) {
                        throw hostError(
                            HttpHostErrorKind.RESPONSE_TOO_LARGE,
                            "Response headers exceeded their limit",
                        )
                    }
                    add(HttpHeader(name, value))
                }
            }
        }
    }

    private fun hostError(kind: HttpHostErrorKind, diagnostic: String) =
        HttpHostException.Failed(kind, diagnostic.take(256))

    private companion object {
        const val TESTNET_HOST = "testnet.toncenter.com"
        const val MAX_TIMEOUT_MILLIS = 5 * 60 * 1000
        const val MAX_RESPONSE_HEADER_BYTES = 64 * 1024
        const val MAX_RESPONSE_BODY_BYTES = 4 * 1024 * 1024
        const val MAX_EARLY_CANCELLATIONS = 256
        val deadlineScheduler = Executors.newSingleThreadScheduledExecutor { task ->
            Thread(task, "wallet-engine-http-deadline").apply { isDaemon = true }
        }
    }
}
