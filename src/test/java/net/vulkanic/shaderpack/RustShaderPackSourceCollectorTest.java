package net.vulkanic.shaderpack;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.net.URI;
import java.util.List;
import java.util.Map;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;
import static org.junit.jupiter.api.Assertions.assertThrows;

class RustShaderPackSourceCollectorTest {
	@TempDir
	Path temporaryDirectory;

	@Test
	void diskShaderPackSelectionIsCopiedWithoutIrisRuntimeObjects() throws Exception {
		Path config = temporaryDirectory.resolve("iris.properties");
		Files.writeString(config, "enableShaders=true\nshaderPack=  ExamplePack  \n");
		assertEquals("ExamplePack", RustShaderPackSourceCollector.configuredPackNameFromProperties(config).orElseThrow());
		Files.writeString(config, "enableShaders=true\nshaderPack=(internal)\n");
		assertTrue(RustShaderPackSourceCollector.configuredPackNameFromProperties(config).isEmpty());
		Files.writeString(config, "enableShaders=false\n");
		assertTrue(RustShaderPackSourceCollector.configuredPackNameFromProperties(config).isEmpty());
		Files.writeString(config, "enableShaders=false\nshaderPack=ComplementaryHungLoIfied.zip\n");
		assertTrue(RustShaderPackSourceCollector.configuredPackNameFromProperties(config).isEmpty());
		Files.writeString(config, "enableShaders=true\nshaderPack=ComplementaryHungLoIfied.zip\n");
		assertEquals("ComplementaryHungLoIfied.zip", RustShaderPackSourceCollector.configuredPackNameFromProperties(config).orElseThrow());
	}

	@Test
	void diskPackOptionsAreBoundedAndFilteredToScalarRustOptions() throws Exception {
		Files.writeString(temporaryDirectory.resolve("ExamplePack.txt"),
			"shadowQuality=2\nprofile.fast=PROFILE\ninvalid.option=drop\nempty=\n");
		assertEquals(Map.of("shadowQuality", "2"),
			RustShaderPackSourceCollector.readWholeFramePackOptions(temporaryDirectory, "ExamplePack"));
	}

	@Test
	void wholeFrameEnvironmentUsesStableRustOwnedStageIdentities() {
		Map<String, String> defines = RustShaderPackSourceCollector.wholeFrameEnvironmentDefines();
		assertEquals("1", defines.get("IS_IRIS"));
		assertEquals("12000", defines.get("IRIS_VERSION"));
		assertEquals("12105", defines.get("MC_VERSION"));
		assertEquals("8", defines.get("MC_RENDER_STAGE_TERRAIN_SOLID"));
		assertEquals("23", defines.get("MC_RENDER_STAGE_ENTITIES"));
	}

	@Test
	void collectsOnlyOrderedShaderSourceAndConfigurationFiles() throws Exception {
		Files.createDirectories(temporaryDirectory.resolve("lib"));
		Files.createDirectories(temporaryDirectory.resolve("lib/antialiasing"));
		Files.createDirectories(temporaryDirectory.resolve("mattmc"));
		Files.writeString(temporaryDirectory.resolve("program.fsh"), "fragment");
		Files.writeString(temporaryDirectory.resolve("lib/common.glsl"), "common");
		Files.writeString(temporaryDirectory.resolve("lib/antialiasing/jitter.glsl"), "jitter");
		Files.writeString(temporaryDirectory.resolve("mattmc/terrain-resource-bindings.properties"), "tex=material_atlas");
		Files.writeString(temporaryDirectory.resolve("shaders.properties"), "profile.test=TEST");
		Files.write(temporaryDirectory.resolve("albedo.png"), new byte[] {1, 2, 3});

		RustShaderPackSourceCollector.SourceGeneration source = RustShaderPackSourceCollector.collect(
			temporaryDirectory,
			"test-pack",
			7L
		);

		assertEquals(7L, source.generation());
		assertEquals(5, source.files().size());
		assertEquals("lib/antialiasing/jitter.glsl", source.files().get(0).path());
		assertEquals("lib/common.glsl", source.files().get(1).path());
		assertEquals("mattmc/terrain-resource-bindings.properties", source.files().get(2).path());
		assertEquals("program.fsh", source.files().get(3).path());
		assertEquals("shaders.properties", source.files().get(4).path());
	}

	@Test
	void collectsBoundedBinaryAssetsAlongsideButSeparateFromSource() throws Exception {
		Files.createDirectories(temporaryDirectory.resolve("textures"));
		Files.writeString(temporaryDirectory.resolve("gbuffers_terrain.fsh"), "fragment");
		Files.write(temporaryDirectory.resolve("textures/noise.png"), new byte[] {1, 2, 3});
		Files.writeString(temporaryDirectory.resolve("textures/noise.png.mcmeta"), "{\"animation\":{}}");
		Files.write(temporaryDirectory.resolve("textures/volume.raw"), new byte[] {4, 5});
		Files.writeString(temporaryDirectory.resolve("notes.txt"), "not a runtime texture asset");

		RustShaderPackSourceCollector.SourceGeneration generation =
			RustShaderPackSourceCollector.collectWithAssets(temporaryDirectory, "assets-pack", 8L);

		assertEquals(List.of("gbuffers_terrain.fsh"), generation.files().stream().map(file -> file.path()).toList());
		assertEquals(
			List.of("textures/noise.png", "textures/noise.png.mcmeta", "textures/volume.raw"),
			generation.assets().stream().map(file -> file.path()).toList()
		);
		assertEquals(21L, generation.assetTotalBytes());
	}

	@Test
	void namespacedPackTextureDeclarationsRetainCanonicalCopiedAssetIdentity() throws Exception {
		Files.createDirectories(temporaryDirectory.resolve("minecraft/textures"));
		Files.writeString(temporaryDirectory.resolve("shaders.properties"),
			"texture.noise=minecraft:textures/noise.png\n");
		Files.write(temporaryDirectory.resolve("minecraft/textures/noise.png"), new byte[] {9, 8, 7, 6});

		RustShaderPackSourceCollector.SourceGeneration generation =
			RustShaderPackSourceCollector.collectWithAssets(temporaryDirectory, "namespaced-pack", 9L);

		assertEquals(List.of("minecraft/textures/noise.png"),
			generation.assets().stream().map(file -> file.path()).toList());
	}

	@Test
	void rejectsOversizedSourceBeforeTransport() throws Exception {
		Path source = temporaryDirectory.resolve("large.vsh");
		try (java.io.OutputStream output = Files.newOutputStream(source)) {
			output.write(new byte[RustShaderPackSourceCollector.MAX_FILE_BYTES + 1]);
		}
		assertThrows(IOException.class, () -> RustShaderPackSourceCollector.collect(temporaryDirectory, "test", 1L));
	}

	@Test
	void disabledGenerationIsExplicitAndContainsNoStaleFiles() {
		RustShaderPackSourceCollector.SourceGeneration source = RustShaderPackSourceCollector.disabled(3L);
		assertEquals("disabled", source.packName());
		assertEquals(3L, source.generation());
		assertEquals(0, source.files().size());
		assertEquals(0, source.assets().size());
		assertEquals(0L, source.assetTotalBytes());
	}

	@Test
	void collectsTheSameRelativeSourcePathsFromAnArchiveRoot() throws Exception {
		Path archive = temporaryDirectory.resolve("test-pack.zip");
		URI uri = URI.create("jar:" + archive.toUri());
		try (java.nio.file.FileSystem fileSystem = java.nio.file.FileSystems.newFileSystem(uri, Map.of("create", "true"))) {
			Path root = fileSystem.getPath("/shaders");
			Files.createDirectories(root.resolve("lib"));
			Files.writeString(root.resolve("gbuffers_terrain.fsh"), "fragment");
			Files.writeString(root.resolve("lib/common.glsl"), "common");
			RustShaderPackSourceCollector.SourceGeneration source = RustShaderPackSourceCollector.collect(root, "archive-pack", 9L);
			assertEquals(2, source.files().size());
			assertEquals("gbuffers_terrain.fsh", source.files().get(0).path());
			assertEquals("lib/common.glsl", source.files().get(1).path());
		}
	}

	@Test
	void runtimeOptionSnapshotIsSortedBoundedAndReserved() throws Exception {
		RustShaderPackSourceCollector.SourceGeneration source = RustShaderPackSourceCollector.collect(
			temporaryDirectory,
			"configured-pack",
			11L
		);
		RustShaderPackSourceCollector.SourceGeneration configured =
			RustShaderPackSourceCollector.withRuntimeOptionSnapshot(source, Map.of(
				"Z_OPTION", "low",
				"A_OPTION", "1"
			));

		assertEquals(1, configured.files().size());
		assertEquals(RustShaderPackSourceCollector.RUNTIME_OPTIONS_PATH, configured.files().get(0).path());
		assertEquals("A_OPTION=1\nZ_OPTION=low\n", new String(configured.files().get(0).contentsUtf8()));
		assertThrows(IOException.class, () -> RustShaderPackSourceCollector.withRuntimeOptionSnapshot(
			configured,
			Map.of("BAD-OPTION", "1")
		));
		RustShaderPackSourceCollector.SourceGeneration fresh = RustShaderPackSourceCollector.collect(
			temporaryDirectory,
			"configured-pack",
			12L
		);
		assertThrows(IOException.class, () -> RustShaderPackSourceCollector.withRuntimeOptionSnapshot(
			fresh,
			Map.of("VALID_OPTION", "not a token")
		));
		assertThrows(IOException.class, () -> RustShaderPackSourceCollector.withRuntimeOptionSnapshot(
			fresh,
			Map.of("VALID_OPTION", "")
		));
	}

	@Test
	void resolvedSourceSnapshotReplacesOnlyKnownShaderSources() throws Exception {
		Files.writeString(temporaryDirectory.resolve("gbuffers_terrain.vsh"), "#define QUALITY 1\n");
		Files.writeString(temporaryDirectory.resolve("shaders.properties"), "profile.default=QUALITY=1\n");
		RustShaderPackSourceCollector.SourceGeneration raw = RustShaderPackSourceCollector.collect(
			temporaryDirectory,
			"configured-pack",
			14L
		);

		RustShaderPackSourceCollector.SourceGeneration resolved =
			RustShaderPackSourceCollector.withResolvedSourceSnapshot(raw, Map.of(
				"gbuffers_terrain.vsh", "#define QUALITY 3 // Iris resolved option\n".getBytes(java.nio.charset.StandardCharsets.UTF_8),
				"missing.vsh", "must not be introduced".getBytes(java.nio.charset.StandardCharsets.UTF_8)
			));

		assertEquals(
			"#define QUALITY 3 // Iris resolved option\n",
			new String(resolved.files().stream()
				.filter(file -> file.path().equals("gbuffers_terrain.vsh"))
				.findFirst().orElseThrow().contentsUtf8(), java.nio.charset.StandardCharsets.UTF_8)
		);
		assertEquals(
			"profile.default=QUALITY=1\n",
			new String(resolved.files().stream()
				.filter(file -> file.path().equals("shaders.properties"))
				.findFirst().orElseThrow().contentsUtf8(), java.nio.charset.StandardCharsets.UTF_8)
		);
		assertEquals(2, resolved.files().size());
	}

	@Test
	void runtimeConstantSnapshotIsSortedBoundedAndSeparateFromMacros() throws Exception {
		RustShaderPackSourceCollector.SourceGeneration source = RustShaderPackSourceCollector.collect(
			temporaryDirectory,
			"configured-pack",
			13L
		);
		RustShaderPackSourceCollector.SourceGeneration configured =
			RustShaderPackSourceCollector.withRuntimeConstantSnapshot(source, Map.of(
				"shadowDistance", "320.0",
				"A_CONST", "1"
			));

		assertEquals(1, configured.files().size());
		assertEquals(RustShaderPackSourceCollector.RUNTIME_CONSTANTS_PATH, configured.files().get(0).path());
		assertEquals("A_CONST=1\nshadowDistance=320.0\n", new String(configured.files().get(0).contentsUtf8()));
	}

	@Test
	void disabledBooleanOptionsRemainUndefinedForDefinedGuards() {
		Map<String, String> options = new java.util.TreeMap<>();
		RustShaderPackSourceCollector.putEnabledBooleanOption(options, "POM", false);
		RustShaderPackSourceCollector.putEnabledBooleanOption(options, "TAA", true);

		assertEquals(Map.of("TAA", "1"), options);
	}

	@Test
	void stageSelectorsAreNeverCopiedAsShaderPackOptions() {
		assertTrue(RustShaderPackSourceCollector.isShaderStageSelector("VERTEX_SHADER"));
		assertTrue(RustShaderPackSourceCollector.isShaderStageSelector("FRAGMENT_SHADER"));
		assertFalse(RustShaderPackSourceCollector.isShaderStageSelector("POM"));
	}

	@Test
	void vanillaWholeFrameEnvironmentDoesNotPublishDistantHorizons() {
		assertFalse(RustShaderPackSourceCollector.wholeFrameEnvironmentDefines().containsKey("DISTANT_HORIZONS"));
	}

	@Test
	void ordinaryWholeFrameEnvironmentOmitsDistantHorizonsWithoutAnOwnedWriter() {
		assertFalse(RustShaderPackSourceCollector.wholeFrameEnvironmentDefines(false).containsKey("DISTANT_HORIZONS"));
		assertEquals("1", RustShaderPackSourceCollector.wholeFrameEnvironmentDefines(true).get("DISTANT_HORIZONS"));
	}

	@Test
	void resourceManagerShaderPathsNormalizeToRustSnapshotRoot() {
		assertEquals("assets/minecraft/shaders/post/probe.fsh",
			RustShaderPackSourceCollector.vanillaShaderSourcePath(net.minecraft.resources.ResourceLocation.parse("minecraft:shaders/post/probe.fsh")));
		assertEquals("assets/example/shaders/post/probe.fsh",
			RustShaderPackSourceCollector.vanillaShaderSourcePath(net.minecraft.resources.ResourceLocation.parse("example:shaders/post/probe.fsh")));
		assertThrows(IllegalArgumentException.class, () -> RustShaderPackSourceCollector.vanillaShaderSourcePath(
			net.minecraft.resources.ResourceLocation.parse("example:post/probe.fsh")));
		assertEquals("program/custom.fsh",
			RustShaderPackSourceCollector.normalizeShaderSourcePath("shaders/program/custom.fsh"));
		assertEquals("program/custom.fsh",
			RustShaderPackSourceCollector.normalizeShaderSourcePath("program/custom.fsh"));
		assertThrows(IllegalArgumentException.class,
			() -> RustShaderPackSourceCollector.normalizeShaderSourcePath(""));
	}

	@Test
	void runtimeEnvironmentSnapshotUsesSeparateReservedSemanticPath() throws Exception {
		RustShaderPackSourceCollector.SourceGeneration source = RustShaderPackSourceCollector.collect(
			temporaryDirectory,
			"configured-pack",
			13L
		);
		RustShaderPackSourceCollector.SourceGeneration configured =
			RustShaderPackSourceCollector.withRuntimeEnvironmentSnapshot(source, Map.of(
				"IS_IRIS", "",
				"IRIS_VERSION", "12000"
			));

		assertEquals(1, configured.files().size());
		assertEquals(RustShaderPackSourceCollector.RUNTIME_ENVIRONMENT_PATH, configured.files().get(0).path());
		assertEquals("IRIS_VERSION=12000\nIS_IRIS=1\n", new String(configured.files().get(0).contentsUtf8()));
		assertThrows(IOException.class, () -> RustShaderPackSourceCollector.withRuntimeEnvironmentSnapshot(
			configured,
			Map.of("MC_VERSION", "12105")
		));
	}
}
