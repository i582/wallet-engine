plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
}

fun readEnvironmentValue(file: File, name: String): String {
    if (!file.isFile) return ""
    return file.useLines { lines ->
        lines.firstNotNullOfOrNull { line ->
            val separator = line.indexOf('=')
            if (separator <= 0 || line.substring(0, separator).trim() != name) {
                null
            } else {
                line.substring(separator + 1).trim().removeSurrounding("\"")
            }
        }
    }.orEmpty()
}

fun String.asBuildConfigString(): String =
    "\"${replace("\\", "\\\\").replace("\"", "\\\"")}\""

val localEnvironmentFile = rootProject.file(".env")
val tonCenterTestnetApiKey = readEnvironmentValue(
    localEnvironmentFile,
    "TONCENTER_TESTNET_API_KEY",
)

android {
    namespace = "com.example.tonwallet"
    compileSdk {
        version = release(37)
    }

    defaultConfig {
        applicationId = "com.example.tonwallet"
        minSdk = 28
        targetSdk = 37
        versionCode = 1
        versionName = "1.0"

        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        buildConfigField(
            "String",
            "TONCENTER_TESTNET_API_KEY",
            tonCenterTestnetApiKey.asBuildConfigString(),
        )

        ndk { abiFilters += listOf("arm64-v8a", "x86_64") }
    }

    buildTypes {
        create("instrumentedTest") {
            initWith(getByName("debug"))
            applicationIdSuffix = ".instrumented"
            versionNameSuffix = "-instrumented"
            matchingFallbacks += listOf("debug")
        }
        release {
            optimization {
                enable = false
            }
        }
    }
    testBuildType = "instrumentedTest"
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_11
        targetCompatibility = JavaVersion.VERSION_11
    }
    buildFeatures {
        compose = true
        buildConfig = true
    }
    sourceSets {
        named("main") {
            kotlin.srcDir(rootProject.file("../../bindings/kotlin/src/main/kotlin"))
            jniLibs.srcDir(rootProject.file("../../target/android/jniLibs"))
        }
    }
}

dependencies {
    implementation(platform(libs.androidx.compose.bom))
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.compose.material3)
    implementation(libs.androidx.compose.ui)
    implementation(libs.androidx.compose.ui.graphics)
    implementation(libs.androidx.compose.ui.tooling.preview)
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.lifecycle.runtime.ktx)
    implementation(libs.androidx.lifecycle.viewmodel.compose)
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation(libs.androidx.compose.material.icons.extended)
    implementation("androidx.annotation:annotation:1.9.1")
    implementation("net.java.dev.jna:jna:5.12.0@aar")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.10.2")
    testImplementation(libs.junit)
    androidTestImplementation(platform(libs.androidx.compose.bom))
    androidTestImplementation(libs.androidx.compose.ui.test.junit4)
    androidTestImplementation(libs.androidx.espresso.core)
    androidTestImplementation(libs.androidx.junit)
    debugImplementation(libs.androidx.compose.ui.test.manifest)
    debugImplementation(libs.androidx.compose.ui.tooling)
}
