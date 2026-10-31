import org.apache.commons.lang3.SystemUtils

plugins {
    idea
    java
    id("gg.essential.loom") version "1.13.44"
    id("dev.architectury.architectury-pack200") version "0.1.3"
    id("com.github.johnrengelman.shadow") version "8.1.1"
}

// ---------------------------------------------------------------------------
// Version: ../package.json is the single source of truth (launcher version).
// ---------------------------------------------------------------------------
val launcherVersion: String = run {
    val pkg = file("../package.json")
    if (!pkg.exists()) return@run "0.0.0"
    // package.json is JSON, not a .properties file: pull the top-level "version".
    val match = Regex("\"version\"\\s*:\\s*\"([^\"]+)\"").find(pkg.readText())
    match?.groupValues?.get(1) ?: "0.0.0"
}

group = "es.spectral"
version = launcherVersion

val baseGroup: String by project
val mcVersion: String by project
val modid: String by project
val transformerFile = file("src/main/resources/accesstransformer.cfg")

// Every mixin config of the mod (one per feature area): listed in the jar
// manifest and registered for dev runs, so adding a config needs no build edit.
val espectralMixinConfigs: List<String> = fileTree("src/main/resources") { include("mixins.$modid*.json") }
    .files.map { it.name }.sorted()

// Java 8 toolchain: 1.8.9 is compiled against MCP stable 22 names, the game
// itself runs on Java 8. Gradle itself runs on a 21+ JDK (see gradle.properties).
java {
    toolchain.languageVersion.set(JavaLanguageVersion.of(8))
}

loom {
    // Gradle's task-validation needs an annotated getter; loom 1.x has one,
    // which is why this build does not use the 0.10.0 template line.
    log4jConfigs.from(file("log4j2.xml"))

    forge {
        // Architectury's pack200 adapter: Forge 1.8.9's universal jar is packed.
        pack200Provider.set(dev.architectury.pack200.java.Pack200Adapter())
        espectralMixinConfigs.forEach { mixinConfig(it) }
        if (transformerFile.exists()) {
            println("Installing access transformer")
            accessTransformer(transformerFile)
        }
    }

    mixin {
        // 1.8.9 ships Mixin 0.7.11, whose refmap format only the legacy AP
        // writes; enabling it also adds the AP's own classpath (ASM, Guava,
        // Gson) so a plain "annotationProcessor(mixin)" line is not needed.
        useLegacyMixinAp.set(true)
        defaultRefmapName.set("mixins.$modid.refmap.json")
    }

    runs {
        named("client") {
            configName = "Minecraft Client"
            if (SystemUtils.IS_OS_MAC_OSX) {
                vmArgs.remove("-XstartOnFirstThread")
            }
            property("mixin.debug", "true")
            programArg("--tweakClass")
            programArg("org.spongepowered.asm.launch.MixinTweaker")
            // Agent bridge under test:
            //   ./gradlew runClient -PespectralAgent=1 -PbridgeUrl=... -PbridgeToken=...
            if (project.hasProperty("espectralAgent")) {
                property("espectral.agent", project.property("espectralAgent").toString())
            }
            if (project.hasProperty("bridgeUrl")) {
                property("espectral.bridge.url", project.property("bridgeUrl").toString())
            }
            if (project.hasProperty("bridgeToken")) {
                property("espectral.bridge.token", project.property("bridgeToken").toString())
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Generated assets. The launcher repo stays the single source of truth:
//   - suite registry: ../native/engine/data/suite-registry.json
//   - texts: the Fabric mod's generated lang JSON (scripts/sync-suite-registry.mjs
//     writes them from the registry + FIXED_STRINGS + ../branding-mod/lang-extra.json),
//     converted to 1.8.9 `.lang` files, plus src/lang-1.8.9.json for strings
//     only this port shows;
//   - title-screen sprites from the Fabric mod.
// ---------------------------------------------------------------------------
val generatedAssets = layout.buildDirectory.dir("generated/espectral")
val fabricAssets = file("../branding-mod/src/main/resources/assets/espectral-menu")

val generateEspectralAssets by tasks.registering {
    val registry = file("../native/engine/data/suite-registry.json")
    val extra = file("src/lang-1.8.9.json")
    inputs.file(registry)
    inputs.file(extra)
    inputs.dir(fabricAssets.resolve("lang"))
    inputs.dir(fabricAssets.resolve("textures/gui/sprites"))
    outputs.dir(generatedAssets)
    doLast {
        val out = generatedAssets.get().asFile.resolve("assets/$modid")
        out.deleteRecursively()
        out.resolve("lang").mkdirs()
        registry.copyTo(out.resolve("suite-registry.json"), overwrite = true)
        fabricAssets.resolve("textures/gui/sprites").copyRecursively(out.resolve("textures/gui"), overwrite = true)
        @Suppress("UNCHECKED_CAST")
        val extras = groovy.json.JsonSlurper().parse(extra) as Map<String, Map<String, String>>
        for ((fabric, legacy, block) in listOf(Triple("en_us", "en_US", "en"), Triple("es_es", "es_ES", "es"))) {
            @Suppress("UNCHECKED_CAST")
            val strings = (groovy.json.JsonSlurper().parse(fabricAssets.resolve("lang/$fabric.json"))
                as Map<String, String>).toSortedMap()
            strings.putAll(extras[block] ?: emptyMap())
            // 1.8.9 .lang: one `key=value` per line, UTF-8, no escapes.
            out.resolve("lang/$legacy.lang").writeText(
                strings.entries.joinToString("\n", postfix = "\n") { (k, v) -> "$k=${v.replace("\n", " ")}" },
                Charsets.UTF_8,
            )
        }
    }
}

sourceSets.main {
    output.setResourcesDir(sourceSets.main.flatMap { it.java.classesDirectory })
    resources.srcDir(generatedAssets)
}

repositories {
    mavenCentral()
    maven("https://repo.spongepowered.org/maven/")
}

val shadowImpl: Configuration by configurations.creating {
    configurations.implementation.get().extendsFrom(this)
}

dependencies {
    minecraft("com.mojang:minecraft:$mcVersion")
    mappings("de.oceanlabs.mcp:mcp_stable:22-1.8.9")
    forge("net.minecraftforge:forge:$mcVersion-11.15.1.2318-$mcVersion")

    // Mixin 0.7.11 is what Forge 1.8.9 ships; 0.8.5 AP reads/writes refmaps
    // for the same level.
    shadowImpl("org.spongepowered:mixin:0.7.11-SNAPSHOT") {
        isTransitive = false
    }

    testImplementation("junit:junit:4.13.2")
}

// 1.8.9's Launchwrapper casts the system class loader to URLClassLoader, so the
// game itself must run on the Java 8 toolchain even though Gradle runs on 21.
tasks.named<JavaExec>("runClient") {
    javaLauncher.set(javaToolchains.launcherFor {
        languageVersion.set(JavaLanguageVersion.of(8))
    })
    jvmArgs("-Xmx2G")
}

tasks.withType(JavaCompile::class) {
    options.encoding = "UTF-8"
    options.compilerArgs.add("-Xlint:deprecation")
}

tasks.withType(Test::class) {
    useJUnit()
}

tasks.withType(org.gradle.jvm.tasks.Jar::class) {
    archiveBaseName.set("espectral-menu-$mcVersion")
    manifest.attributes.run {
        this["FMLCorePluginContainsFMLMod"] = "true"
        this["ForceLoadAsMod"] = "true"
        this["TweakClass"] = "org.spongepowered.asm.launch.MixinTweaker"
        this["MixinConfigs"] = espectralMixinConfigs.joinToString(",")
        if (transformerFile.exists()) this["FMLAT"] = "${modid}_at.cfg"
        this["Implementation-Version"] = project.version.toString()
        this["Espectral-Launcher-Version"] = launcherVersion
    }
}

tasks.processResources {
    dependsOn(generateEspectralAssets)
    inputs.property("version", project.version)
    inputs.property("mcversion", mcVersion)
    inputs.property("modid", modid)
    inputs.property("basePackage", baseGroup)

    filesMatching(listOf("mcmod.info", "mixins.$modid*.json", "espectral-menu.properties")) {
        expand(inputs.properties)
    }

    rename("accesstransformer.cfg", "META-INF/${modid}_at.cfg")
}

val remapJar by tasks.named<net.fabricmc.loom.task.RemapJarTask>("remapJar") {
    archiveClassifier.set("")
    from(tasks.shadowJar)
    input.set(tasks.shadowJar.get().archiveFile)
}

tasks.jar {
    archiveClassifier.set("without-deps")
    destinationDirectory.set(layout.buildDirectory.dir("intermediates"))
}

tasks.shadowJar {
    destinationDirectory.set(layout.buildDirectory.dir("intermediates"))
    archiveClassifier.set("non-obfuscated-with-deps")
    configurations = listOf(shadowImpl)
}

tasks.assemble.get().dependsOn(tasks.remapJar)
