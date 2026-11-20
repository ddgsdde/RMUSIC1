package com.rmusic.android.development
import androidx.test.platform.app.InstrumentationRegistry
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
class SessionVaultTest {
 @Test fun encryptedSessionIsBoundToCoreAndProfileAndCanBeForgotten(){
  val context=InstrumentationRegistry.getInstrumentation().targetContext
  val profile="qa-vault-${System.nanoTime()}"
  fun command(action:String,core:String="qa-core",token:String?=null):JSONObject=JSONObject(SessionVault.command(context,JSONObject().put("action",action).put("profile_id",profile).put("core_id",core).put("token",token)))
  try {
   command("save",token="synthetic-session-vault-test")
   val persisted=context.getSharedPreferences("rmusic-core-sessions",0).getString(profile,null)!!
   assertFalse(persisted.contains("synthetic-session-vault-test"))
   assertEquals("synthetic-session-vault-test",command("load").getString("token"))
   assertFalse(command("load",core="different-core").has("token"))
   command("forget");assertFalse(command("load").has("token"))
  } finally {command("forget")}
 }
}
