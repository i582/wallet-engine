plugins {
    id("com.android.library") version "9.3.1"
}

base {
    archivesName.set("wallet-engine")
}

android {
    namespace = "org.ton.wallet.engine"
    compileSdk {
        version = release(37)
    }

    defaultConfig {
        minSdk = 28
        consumerProguardFiles("consumer-rules.pro")
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_11
        targetCompatibility = JavaVersion.VERSION_11
    }

    sourceSets {
        named("main") {
            kotlin.directories.add(rootProject.file("../../bindings/kotlin/src/main/kotlin").path)
            jniLibs.directories.add(rootProject.file("../../target/android/jniLibs").path)
        }
    }
}

dependencies {
    api("net.java.dev.jna:jna:5.12.0@aar")
    api("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.10.2")
    implementation("androidx.annotation:annotation:1.9.1")
}
