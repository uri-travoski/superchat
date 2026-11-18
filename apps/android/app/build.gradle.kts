plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

// `cargo ndk` cross-compiles the Rust app (client-core + Slint UI) into jniLibs.
// ABIs default to arm64 + x86_64 (emulator); override with -PatomicAbis=x86_64 for quick tests.
val abis = (project.findProperty("atomicAbis") as String? ?: "arm64-v8a,x86_64").split(",")
val repoRoot = rootProject.projectDir.resolve("../..")
val ndkHome = System.getenv("ANDROID_NDK_HOME") ?: "/opt/android-sdk/ndk/27.1.12297006"
val sdkHome = System.getenv("ANDROID_HOME") ?: System.getenv("ANDROID_SDK_ROOT") ?: "/opt/android-sdk"

val cargoNdk by tasks.registering(Exec::class) {
    workingDir = repoRoot
    environment("ANDROID_NDK_HOME", ndkHome)
    environment("ANDROID_HOME", sdkHome)
    environment("ANDROID_JAR", "$sdkHome/platforms/android-36/android.jar")
    val args = mutableListOf("cargo", "ndk")
    abis.forEach { args += listOf("-t", it) }
    args += listOf("-P", "26", "-o", projectDir.resolve("src/main/jniLibs").absolutePath, "build", "--release", "-p", "atomic-android")
    commandLine(args)
}

// Google push (FCM) is compiled in only when a config exists; the default APK has no Google dependency.
val fcmEnabled = file("src/fcm/assets/fcm-config.json").exists()

android {
    namespace = "app.superchat"
    compileSdk = 36

    defaultConfig {
        applicationId = "app.superchat"
        minSdk = 26
        targetSdk = 35
        versionCode = 6
        versionName = "1.2.0"
        resourceConfigurations += listOf("en")
    }

    // Release signing: set ATOMIC_KEYSTORE_FILE (+ _PASSWORD, ATOMIC_KEY_ALIAS, ATOMIC_KEY_PASSWORD) to sign with your own key
    // (CI generates one when no secret is configured). Without them the build is signed with the debug key.
    val releaseKeystore = System.getenv("ATOMIC_KEYSTORE_FILE")
    if (releaseKeystore != null) {
        signingConfigs.create("atomicRelease") {
            storeFile = file(releaseKeystore)
            storePassword = System.getenv("ATOMIC_KEYSTORE_PASSWORD")
            keyAlias = System.getenv("ATOMIC_KEY_ALIAS") ?: "atomic"
            keyPassword = System.getenv("ATOMIC_KEY_PASSWORD") ?: System.getenv("ATOMIC_KEYSTORE_PASSWORD")
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            isDebuggable = (project.findProperty("atomicDebuggable") == "true")
            signingConfig = signingConfigs.getByName(if (releaseKeystore != null) "atomicRelease" else "debug")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    sourceSets.getByName("main") {
        if (fcmEnabled) {
            manifest.srcFile("src/fcm/AndroidManifest.xml")
            java.srcDir("src/fcm/java")
            assets.srcDir("src/fcm/assets")
        }
    }
    packaging {
        jniLibs { useLegacyPackaging = false }
    }
}

kotlin {
    compilerOptions {
        jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17)
    }
}

tasks.matching { it.name == "preBuild" }.configureEach { dependsOn(cargoNdk) }

dependencies {
    // QR login and video clips: CameraX preview, analysis and video capture, decoded in memory with ZXing core (no Google Play services needed).
    implementation("androidx.camera:camera-camera2:1.4.2")
    implementation("androidx.camera:camera-lifecycle:1.4.2")
    implementation("androidx.camera:camera-video:1.4.2")
    implementation("androidx.camera:camera-view:1.4.2")
    implementation("androidx.activity:activity:1.9.3")
    implementation("com.google.zxing:core:3.5.3")
    if (fcmEnabled) implementation("com.google.firebase:firebase-messaging:24.1.0")
}
