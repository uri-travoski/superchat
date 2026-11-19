# R8 rules for the SuperChat release APK (isMinifyEnabled + isShrinkResources, see build.gradle.kts).
#
# The Rust library (libatomic_android.so) and the Kotlin shell call each other by name:
#  - Rust looks up app.superchat.Native and calls its static methods through JNI (call_static_method with the method name and
#    signature as strings), and Rust exports Java_app_superchat_Native_* symbols that the JVM binds to the `external fun`s.
#  - MainActivity is a NativeActivity that loads the library named in the manifest; the services, receivers and activities are
#    found through the manifest.
# None of that is visible to R8, so every class of the app's own package keeps its names and members (the app's code is small;
# the size comes from the libraries, which R8 still shrinks).
-keep class app.superchat.** { *; }
-keepclasseswithmembernames class * { native <methods>; }
-keepattributes *Annotation*, Signature, InnerClasses, EnclosingMethod
# Stack traces in a crash report stay readable.
-keepattributes SourceFile, LineNumberTable
-renamesourcefileattribute SourceFile
