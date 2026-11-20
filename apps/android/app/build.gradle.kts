plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

// `cargo ndk` cross-compiles the Rust app (client-core + Slint UI) into jniLibs.
// ABIs default to arm64 + x86_64 (emulator); override with -PatomicAbis=x86_64 for quick tests.
// Release APKs are split by ABI (one APK per ABI, see `splits` below): superchat-<v>-android-arm64.apk and ...-x86_64.apk.
val abis = (project.findProperty("atomicAbis") as String? ?: "arm64-v8a,x86_64").split(",")
// The Cargo profile of the native library: "release-android" (size-optimised, see the workspace Cargo.toml) or "release".
val cargoProfile = (project.findProperty("atomicCargoProfile") as String?) ?: "release-android"
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
    args += listOf("-P", "26", "-o", projectDir.resolve("src/main/jniLibs").absolutePath, "build", "--profile", cargoProfile, "-p", "atomic-android")
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
        // -PatomicVersionCode / -PatomicVersionName only exist for the update tests (a fake newer build of the same app).
        // One versionCode for every ABI: the APKs are installed by hand or by the in-app updater (which picks the one for the
        // device), never uploaded together to one store listing, so Android does not need them to differ.
        versionCode = (project.findProperty("atomicVersionCode") as String?)?.toInt() ?: 11
        versionName = (project.findProperty("atomicVersionName") as String?) ?: "1.4.1"
        resourceConfigurations += listOf("en")
    }

    // Release signing: set ATOMIC_KEYSTORE_FILE (+ ATOMIC_KEYSTORE_PASSWORD, ATOMIC_KEY_ALIAS, ATOMIC_KEY_PASSWORD) to sign with the
    // permanent release key. CI (release.yml) refuses to build a release without it. Without them a local build is signed with
    // the debug key, which is fine for testing but cannot update an install signed with the release key.
    val releaseKeystore = System.getenv("ATOMIC_KEYSTORE_FILE")
    if (releaseKeystore != null) {
        signingConfigs.create("atomicRelease") {
            storeFile = file(releaseKeystore)
            storePassword = System.getenv("ATOMIC_KEYSTORE_PASSWORD")
            keyAlias = System.getenv("ATOMIC_KEY_ALIAS") ?: "atomic"
            keyPassword = System.getenv("ATOMIC_KEY_PASSWORD") ?: System.getenv("ATOMIC_KEYSTORE_PASSWORD")
        }
    }

    // One APK per ABI instead of one that carries every native library (the Rust library is most of the size).
    splits {
        abi {
            isEnable = true
            reset()
            include(*abis.toTypedArray())
            isUniversalApk = (project.findProperty("atomicUniversal") == "true")
        }
    }

    buildTypes {
        release {
            // R8 shrinks and obfuscates the Kotlin side; proguard-rules.pro keeps what the Rust library calls by name.
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
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
    // QR login: CameraX preview and analysis, decoded in memory with ZXing core (no Google Play services needed). 1.4: video recording
    // was removed, so CameraX video capture is no longer a dependency.
    implementation("androidx.camera:camera-camera2:1.4.2")
    implementation("androidx.camera:camera-lifecycle:1.4.2")
    implementation("androidx.camera:camera-view:1.4.2")
    implementation("androidx.activity:activity:1.9.3")
    implementation("com.google.zxing:core:3.5.3")
    if (fcmEnabled) implementation("com.google.firebase:firebase-messaging:24.1.0")
}

// `./gradlew exportApks` builds the release APKs and copies them to dist/ as superchat-<version>-android-arm64.apk and
// superchat-<version>-android-x86_64.apk (what the update check looks for; see docs/DEPLOY.md), each with a .sha256 file.
val exportApks by tasks.registering(Copy::class) {
    dependsOn("assembleRelease")
    val ver = (project.findProperty("atomicVersionName") as String?) ?: "1.4.1"
    from(layout.buildDirectory.dir("outputs/apk/release")) { include("*.apk") }
    into(repoRoot.resolve("dist"))
    rename { n ->
        when {
            n.contains("arm64-v8a") -> "superchat-$ver-android-arm64.apk"
            n.contains("x86_64") -> "superchat-$ver-android-x86_64.apk"
            n.contains("universal") -> "superchat-$ver-android.apk"
            else -> n
        }
    }
}
