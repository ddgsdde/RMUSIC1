package com.rmusic.android.development

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

/** Persist only Core sessions, encrypted with an application-owned Android Keystore key. */
internal object SessionVault {
    private const val alias = "rmusic.core.sessions.v1"
    private fun key(): SecretKey {
        val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (store.getKey(alias, null) as? SecretKey)?.let { return it }
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore").apply {
            init(KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM).setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE).build())
        }.generateKey()
    }
    @Synchronized fun command(context: Context, args: JSONObject): String {
        val profile = args.getString("profile_id")
        require(profile.length in 1..128)
        val prefs = context.getSharedPreferences("rmusic-core-sessions", Context.MODE_PRIVATE)
        val core = args.optString("core_id")
        when (args.getString("action")) {
            "forget" -> check(prefs.edit().remove(profile).commit())
            "save" -> {
                require(core.isNotBlank())
                val cipher = Cipher.getInstance("AES/GCM/NoPadding").apply { init(Cipher.ENCRYPT_MODE, key()) }
                cipher.updateAAD("$profile:$core".toByteArray())
                val bytes = cipher.doFinal(args.getString("token").toByteArray())
                val value = JSONObject().put("core", core).put("iv", Base64.encodeToString(cipher.iv, Base64.NO_WRAP))
                    .put("ciphertext", Base64.encodeToString(bytes, Base64.NO_WRAP))
                check(prefs.edit().putString(profile, value.toString()).commit())
            }
            "load" -> {
                val saved = prefs.getString(profile, null) ?: return "{}"
                try {
                    val value = JSONObject(saved)
                    if (core.isBlank() || value.getString("core") != core) return "{}"
                    val cipher = Cipher.getInstance("AES/GCM/NoPadding").apply {
                        init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, Base64.decode(value.getString("iv"), Base64.NO_WRAP)))
                    }
                    cipher.updateAAD("$profile:$core".toByteArray())
                    return JSONObject().put("token", String(cipher.doFinal(Base64.decode(value.getString("ciphertext"), Base64.NO_WRAP)))).toString()
                } catch (_: Exception) { return "{}" }
            }
            else -> error("Unknown session action")
        }
        return "{}"
    }
}
