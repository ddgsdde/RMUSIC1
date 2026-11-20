package com.rmusic.android.development
import android.content.Intent
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.json.JSONObject
import java.io.File

@RunWith(AndroidJUnit4::class)
class GroupEndpointTest {
    @Test fun realNativeEndpointParticipatesInCoordinatedGroupLifecycle() {
        val runner=InstrumentationRegistry.getInstrumentation()
        val args=InstrumentationRegistry.getArguments()
        val config=JSONObject(args.getString("endpoint_json") ?: error("isolated Core configuration required"))
        val done=File(runner.targetContext.cacheDir,"group-endpoint-done")
        if(done.exists()) done.delete()
        config.put("journal",File(runner.targetContext.filesDir,"group-endpoint-audit").absolutePath)
        runner.startActivitySync(Intent(runner.targetContext,MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        val ready=System.nanoTime()+15000000000L
        while(!DecoderDiagnostics.ready()&&System.nanoTime()<ready)Thread.sleep(20)
        assertTrue(DecoderDiagnostics.ready())
        EndpointDiagnostics.start(config.toString())
        try {
            val deadline=System.nanoTime()+180000000000L
            while(!done.exists()&&System.nanoTime()<deadline)Thread.sleep(50)
            assertTrue("Host did not finish the coordinated group checks",done.exists())
            assertEquals("ok",done.readText().trim())
        } finally {EndpointDiagnostics.stop()}
    }
}
