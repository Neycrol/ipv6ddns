# Keep empty for now.

# Keep DataStore related classes
-keep class androidx.datastore.** { *; }
-keep class com.neycrol.ipv6ddns.data.** { *; }

# Keep Compose composables
-keep class com.neycrol.ipv6ddns.MainActivity { *; }
-keep class com.neycrol.ipv6ddns.MainActivity$* { *; }

# Keep service classes
-keep class com.neycrol.ipv6ddns.service.** { *; }

# Keep data classes
-keepclassmembers class com.neycrol.ipv6ddns.data.** { *; }

# Keep serialization for kotlinx.coroutines
-keepnames class kotlinx.coroutines.internal.MainDispatcherFactory {}
-keepnames class kotlinx.coroutines.CoroutineExceptionHandler {}

# Keep kotlinx.serialization
-keepattributes *Annotation*
-keepclassmembers class kotlinx.serialization.json.** { *; }

# Keep AndroidX Lifecycle
-keep class androidx.lifecycle.** { *; }
-keep class * implements androidx.lifecycle.ViewModel
-keep class * extends androidx.lifecycle.ViewModel
-keepclassmembers class * extends androidx.lifecycle.ViewModel {
    <init>();
}

# Keep Kotlin metadata
-keep class kotlin.Metadata { *; }
-dontwarn kotlin.Metadata
-keep class * implements kotlin.reflect.KProperty
-keep class * implements kotlin.reflect.KFunction
-keepclassmembers class * {
    private <methods>;
}

# Keep Compose runtime
-keep class androidx.compose.** { *; }
-dontwarn androidx.compose.**

# Keep AndroidX DataStore preferences
-keep class androidx.datastore.preferences.** { *; }
-keep interface androidx.datastore.preferences.core.** { *; }
-keepclassmembers class androidx.datastore.preferences.core.** { *; }
