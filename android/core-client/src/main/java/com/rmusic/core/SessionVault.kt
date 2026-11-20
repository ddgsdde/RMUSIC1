package com.rmusic.core

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import org.json.JSONObject

/** Only a Core-scoped session token is persisted. Passwords never reach disk. */
class SessionVault(context: Context) {
    private val preferences = context.getSharedPreferences("core-session", Context.MODE_PRIVATE)
    private val alias = "rmusic-native-core-session-v1"

    private fun key(): SecretKey {
        val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (store.getKey(alias, null) as? SecretKey)?.let {
            return it
        }
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
            .apply {
                init(
                    KeyGenParameterSpec.Builder(
                            alias,
                            KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
                        )
                        .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                        .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                        .build()
                )
            }
            .generateKey()
    }

    fun save(session: CoreSession) {
        val raw =
            json(
                    "address" to session.address,
                    "token" to session.token,
                    "core" to session.coreId,
                    "user" to session.userId,
                    "username" to session.username,
                    "admin" to session.admin,
                )
                .toString()
                .toByteArray()
        val cipher =
            Cipher.getInstance("AES/GCM/NoPadding").apply { init(Cipher.ENCRYPT_MODE, key()) }
        val bytes = cipher.iv + cipher.doFinal(raw)
        check(
            preferences
                .edit()
                .putString("session", Base64.encodeToString(bytes, Base64.NO_WRAP))
                .commit()
        ) {
            "无法保存Core会话"
        }
    }

    fun load(): CoreSession? = runCatching {
        val bytes =
            Base64.decode(preferences.getString("session", null) ?: return null, Base64.NO_WRAP)
        val cipher =
            Cipher.getInstance("AES/GCM/NoPadding").apply {
                init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, bytes.copyOfRange(0, 12)))
            }
        val v =
            JSONObject(cipher.doFinal(bytes.copyOfRange(12, bytes.size)).toString(Charsets.UTF_8))
        CoreSession(
            v.getString("address"),
            v.getString("token"),
            v.getString("core"),
            v.getString("user"),
            v.getString("username"),
            v.optBoolean("admin"),
        )
    }
        .getOrNull()

    fun clear() {
        preferences.edit().remove("session").apply()
    }
}
