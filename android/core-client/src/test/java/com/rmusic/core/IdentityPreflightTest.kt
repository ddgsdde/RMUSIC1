package com.rmusic.core

import java.net.InetAddress
import java.net.ServerSocket
import java.util.concurrent.CopyOnWriteArrayList
import kotlin.concurrent.thread
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Test

class IdentityPreflightTest {
    @Test
    fun identityPreflightDoesNotSendSavedTokenButAuthenticatedRequestsStillDo() = runBlocking {
        val server =
            ServerSocket(0, 2, InetAddress.getByName("127.0.0.1")).apply { soTimeout = 5000 }
        val requests = CopyOnWriteArrayList<String>()
        val responder =
            thread(isDaemon = true) {
                repeat(2) {
                    server.accept().use { socket ->
                        socket.soTimeout = 5000
                        val reader = socket.getInputStream().bufferedReader()
                        val lines = mutableListOf<String>()
                        while (true) {
                            val line = reader.readLine() ?: break
                            if (line.isEmpty()) break
                            lines += line
                        }
                        requests += lines.joinToString("\n")
                        socket
                            .getOutputStream()
                            .write(
                                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}"
                                    .toByteArray()
                            )
                    }
                }
            }
        val api =
            CoreApi(
                CoreSession(
                    "http://127.0.0.1:${server.localPort}",
                    "saved-test-token",
                    "expected-core",
                    "qa",
                    "qa",
                    false,
                )
            )
        try {
            api.request("status")
            api.request("library/home")
            responder.join(5000)
            assertEquals(2, requests.size)
            assertFalse(requests[0].contains("Authorization:", true))
            assertTrue(requests[1].contains("Authorization: Bearer saved-test-token", true))
        } finally {
            api.close()
            server.close()
        }
    }
}
