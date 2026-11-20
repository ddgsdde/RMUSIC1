plugins { id("com.android.library") }
android { namespace="com.rmusic.media"; compileSdk=36; defaultConfig { minSdk=26 }; compileOptions { sourceCompatibility=JavaVersion.VERSION_17; targetCompatibility=JavaVersion.VERSION_17 } }
dependencies {
    api(project(":core-client"))
    api("androidx.media3:media3-exoplayer:1.9.4")
    api("androidx.media3:media3-session:1.9.4")
    api("androidx.media3:media3-datasource-okhttp:1.9.4")
    implementation("androidx.media3:media3-database:1.9.4")
    testImplementation("junit:junit:4.13.2")
    testImplementation("org.json:json:20250517")
}
