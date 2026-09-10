package net.vulkanic.world;

import java.nio.file.Files;
import java.nio.file.Path;

import org.junit.jupiter.api.Test;

import static org.junit.jupiter.api.Assertions.assertTrue;

final class ParticleSemanticContractTest {
	@Test
	void legacyMarkerEntryPointRejectsVulkanBeforeJavaGeometryOrResourceAccess() {
		String key = net.vulkanic.bridge.RustGalVulkanWholeFrameMode.propertyName();
		String previous = System.getProperty(key);
		try {
			System.setProperty(key, "true");
			var error = org.junit.jupiter.api.Assertions.assertThrows(IllegalStateException.class,
				() -> RustGalWorldPrimitiveRenderer.enqueueBlockMarker(null, null,
					0, 0, 0, 0, 0, 0, 1, .5F, -1, null, 0, 1, 0, 1));
			assertTrue(error.getMessage().contains("typed particle semantics"));
		} finally {
			if (previous == null) System.clearProperty(key); else System.setProperty(key, previous);
		}
	}
	@Test
	void nativeMarkersAreNotCollectedTwiceByWholeFrameRendering() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
		assertTrue(!source.contains("enqueueRustGalBlockMarkers("),
			"whole-frame extraction must not duplicate the ordinary visible particle collector");
	}
	@Test
	void normalBlockMarkerRoutePreservesFrozenParticleInputsAndRejectsFallback() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/particle/BlockMarker.java"));
		int start = source.indexOf("boolean enqueueRustGal(");
		int legacy = source.indexOf("return RustGalWorldPrimitiveRenderer.enqueueBlockMarker(", start);
		String semantic = source.substring(start, legacy);
		assertTrue(!semantic.contains("mattmc.dev.nativeBlockMarkerGeometry")
			&& semantic.contains("usesRustWholeFrameVulkan()"));
		for (String input : new String[] {"this.sprite.contents().name()", "this.sprite.semanticAnimationResource()",
			"this.getFacingCameraMode().setRotation", "this.oRoll", "this.getQuadSize(f)",
			"this.getU0()", "this.getU1()", "this.getV0()", "this.getV1()", "this.getLightColor(f)",
			"this.alpha, this.rCol, this.gCol, this.bCol", "ParticleSurface.TERRAIN_OPAQUE",
			"ParticleSurface.TERRAIN_TRANSLUCENT"}) {
			assertTrue(semantic.contains(input), "missing Frozen particle input: " + input);
		}
		assertTrue(!semantic.contains("FULL_BRIGHT") && !semantic.contains("billboardVertices")
			&& !semantic.contains("WorldMaterialQuadRecord"));
		assertTrue(semantic.contains("if (!queued)") && semantic.contains("throw new IllegalStateException")
			&& semantic.contains("return true;"), "rejection cannot fall through to Java geometry");
	}
	@Test
	void normalTerrainVulkanStopsAtSemanticInputsWithoutPrivateGeometrySwitch() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		int start = source.indexOf("public static boolean enqueueTerrainParticle(");
		int end = source.indexOf("private static void enqueueNativeParticleLocked", start);
		String terrain = source.substring(start,end);
		assertTrue(!terrain.contains("nativeTerrainParticleGeometry"));
		int semantic = terrain.indexOf("PENDING_PARTICLE_QUADS.add(");
		int normalReturn = terrain.indexOf("if (!Boolean.getBoolean(\"mattmc.dev.graphicsAuditSliceMetrics\")) return true;");
		int geometry = terrain.indexOf("billboardVertices(");
		assertTrue(semantic >= 0 && normalReturn > semantic && geometry > normalReturn,
			"normal Vulkan must return after semantic publication, before diagnostic Java geometry");
		assertTrue(terrain.contains("if (!nativeTerrain) PENDING_MATERIAL_QUADS.add("),
			"Java-expanded material geometry must be excluded even during Vulkan diagnostics");
	}
	@org.junit.jupiter.api.BeforeAll
	static void bootstrap() {
		net.minecraft.SharedConstants.tryDetectVersion();
		net.minecraft.server.Bootstrap.bootStrap();
	}
	@Test
	void signedAndZeroParticleSizesPreserveVanillaGeometry() throws Exception {
		var validate = RustGalWorldPrimitiveRenderer.class.getDeclaredMethod("validateParticleQuadSemantics",
			net.minecraft.resources.ResourceLocation.class, float.class, float.class, float.class,
			float.class, float.class, float.class, float.class, float.class,
			float.class, float.class, float.class, float.class);
		validate.setAccessible(true);
		var vertices = RustGalWorldPrimitiveRenderer.class.getDeclaredMethod("billboardVertices",
			org.joml.Quaternionf.class, float.class, float.class, float.class, float.class, float[].class);
		vertices.setAccessible(true);
		String property = net.vulkanic.bridge.RustGalVulkanWholeFrameMode.propertyName();
		String previous = System.getProperty(property);
		try {
			System.setProperty(property, "true");
			for (float size : new float[] {0.25F, 0F, -0.0F, -0.012F, -0.25F}) {
				validate.invoke(null, null, 1F, 2F, 3F, 0F, 0F, 0F, 1F, size, 0F, 1F, 0F, 1F);
				var state = new net.minecraft.client.renderer.state.QuadParticleRenderState();
				state.add(net.minecraft.client.particle.SingleQuadParticle.Layer.OPAQUE,
					1, 2, 3, 0, 0, 0, 1, size, 0, 1, 0, 1, -1, 240);
				float[] result = new float[12];
				vertices.invoke(null, new org.joml.Quaternionf(), 1F, 2F, 3F, size, result);
				// Frozen's ordinary quad corners are scaled with the signed size,
				// never clamped, discarded, or replaced by its absolute value.
				org.junit.jupiter.api.Assertions.assertArrayEquals(new float[] {
					1 + size, 2 - size, 3, 1 + size, 2 + size, 3,
					1 - size, 2 + size, 3, 1 - size, 2 - size, 3}, result, 1e-7F);
			}
			for (float size : new float[] {Float.NaN, Float.POSITIVE_INFINITY, Float.NEGATIVE_INFINITY}) {
				org.junit.jupiter.api.Assertions.assertThrows(java.lang.reflect.InvocationTargetException.class,
					() -> validate.invoke(null, null, 1F, 2F, 3F, 0F, 0F, 0F, 1F, size, 0F, 1F, 0F, 1F));
				var state = new net.minecraft.client.renderer.state.QuadParticleRenderState();
				org.junit.jupiter.api.Assertions.assertThrows(IllegalArgumentException.class,
					() -> state.add(net.minecraft.client.particle.SingleQuadParticle.Layer.OPAQUE,
						1, 2, 3, 0, 0, 0, 1, size, 0, 1, 0, 1, -1, 240));
			}
		} finally {
			if (previous == null) System.clearProperty(property); else System.setProperty(property, previous);
		}
	}
	@Test
	void terrainParticleAtlasBindingPreservesAtlasRatherThanSpriteLocalCoordinates() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/particle/TerrainParticle.java"));
		int enqueue = source.indexOf("boolean enqueueRustGal(");
		int nextMethod = source.indexOf("protected float getU0()", enqueue);
		String producer = source.substring(enqueue, nextMethod);
		assertTrue(producer.contains("this.getU0(),") && producer.contains("this.getV1(),")
			&& !producer.contains("Math.min") && !producer.contains("Math.max")
			&& !producer.contains("getUOffset") && !producer.contains("getVOffset"),
			"a full-atlas binding must keep the vanilla producer's atlas coordinates");
	}
	@Test
	void quadParticleAdmissionRollsBackPartialRustMaterialStreams() throws Exception {
		String source = Files.readString(Path.of(
			"src/main/java/net/minecraft/client/renderer/state/QuadParticleRenderState.java"));
		int checkpoint = source.indexOf("markMaterialQuadBatch()");
		int rollback = source.indexOf("rollbackMaterialQuadBatch(checkpoint)", checkpoint);
		assertTrue(checkpoint >= 0 && rollback > checkpoint,
			"particle-group admission must roll back a partial semantic prefix on rejection");
	}

	@Test
	void quadParticleSnapshotRejectsMalformedValuesBeforeStorage() throws Exception {
		String source = Files.readString(Path.of(
			"src/main/java/net/minecraft/client/renderer/state/QuadParticleRenderState.java"));
		int add = source.indexOf("public void add(");
		int finite = source.indexOf("Rust whole-frame particle admission requires finite copied quad semantics", add);
		int store = source.indexOf("computeIfAbsent(layer", finite);
		assertTrue(add >= 0 && finite > add && store > finite,
			"Rust particle snapshots must reject malformed quad state before storing it");
	}

	@Test
	void individualParticleProducersCheckMaterialCapacityBeforeAppend() throws Exception {
		String source = Files.readString(Path.of(
			"src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		int terrain = source.indexOf("public static boolean enqueueTerrainParticle(");
		int terrainCapacity = source.indexOf("PENDING_MATERIAL_QUADS.size() >= MAX_RUST_WORLD_MATERIAL_QUADS", terrain);
		int ordinary = source.indexOf("public static void enqueueParticleQuad(");
		int ordinaryCapacity = source.indexOf("PENDING_MATERIAL_QUADS.size() >= MAX_RUST_WORLD_MATERIAL_QUADS", ordinary);
		int atlas = source.indexOf("public static boolean enqueueParticleQuadForAtlas(");
		int atlasCapacity = source.indexOf("PENDING_MATERIAL_QUADS.size() >= MAX_RUST_WORLD_MATERIAL_QUADS", atlas);
		assertTrue(terrainCapacity > terrain && ordinaryCapacity > ordinary && atlasCapacity > atlas,
			"all individual particle producers must check the bounded Rust material stream");
	}

	@Test
	void terrainParticlesRetainTheExplicitParticleSourceIdentity() throws Exception {
		String source = Files.readString(Path.of(
			"src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		int terrain = source.indexOf("public static boolean enqueueTerrainParticle(");
		int ordinary = source.indexOf("public static void enqueueParticleQuad(", terrain);
		int sourceProgram = source.indexOf("MATERIAL_SOURCE_PARTICLES", terrain);
		assertTrue(terrain >= 0 && sourceProgram > terrain && sourceProgram < ordinary,
			"terrain particles must be admitted to Rust's explicit particle source writer");
	}

	@Test
	void particleAtlasAdmissionValidatesViewportBeforePublishingAsset() throws Exception {
		String source = Files.readString(Path.of(
			"src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		int method = source.indexOf("public static boolean enqueueParticleQuadForAtlas(");
		int viewport = source.indexOf("ensureBoundedParticleViewportLocked()", method);
		int asset = source.indexOf("ensureParticleAtlasAssetLocked(atlasLocation, textureId)", method);
		assertTrue(method >= 0 && viewport > method && asset > viewport,
			"particle atlas payloads must not publish before frame-local viewport admission");
	}

	@Test
	void particleQuadAdmissionPreservesOrientedUvsButRejectsZeroAndUnboundedSpans() throws Exception {
		var method = RustGalWorldPrimitiveRenderer.class.getDeclaredMethod("validateParticleQuadSemantics",
			net.minecraft.resources.ResourceLocation.class, float.class, float.class, float.class,
			float.class, float.class, float.class, float.class, float.class,
			float.class, float.class, float.class, float.class);
		method.setAccessible(true);
		for (float[] uv : new float[][]{{0,1,0,1}, {1,0,0,1}, {0,1,1,0}, {1,0,1,0}}) {
			method.invoke(null, null, 0F,0F,0F, 0F,0F,0F,1F, 0.25F, uv[0],uv[1],uv[2],uv[3]);
		}
		for (float[] uv : new float[][]{{0,0,0,1}, {0,1,1,1}, {0,4097,0,1},
			{4097,0,0,1}, {Float.MAX_VALUE,-Float.MAX_VALUE,0,1}, {Float.NaN,1,0,1}}) {
			var error = org.junit.jupiter.api.Assertions.assertThrows(java.lang.reflect.InvocationTargetException.class,
				() -> method.invoke(null, null, 0F,0F,0F, 0F,0F,0F,1F, 0.25F, uv[0],uv[1],uv[2],uv[3]));
			org.junit.jupiter.api.Assertions.assertInstanceOf(IllegalArgumentException.class, error.getCause());
		}
	}

	@Test
	void terrainParticleControlsAreAppliedAtTheSharedRustAdmissionBoundary() throws Exception {
		String source = Files.readString(Path.of(
			"src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		int method = source.indexOf("public static boolean shouldRouteTerrainParticle(");
		int disabled = source.indexOf("rustGalWorldMaterial.terrainParticle.disabled", method);
		int legacy = source.indexOf("rustGalWorldMaterial.terrainParticle.legacyControl", disabled);
		int unavailable = source.indexOf("Rust whole-frame terrain particle route is unavailable under", legacy);
		int texture = source.indexOf("terrainParticleTextureId(blockState)", unavailable);
		assertTrue(method >= 0 && disabled > method && legacy > disabled && unavailable > legacy && texture > unavailable,
			"direct whole-frame terrain collection must honor particle controls before texture admission");
	}

	@Test
	void reloadableParticleTexturesUseCopiedSemanticImagesInsteadOfJavaGpuState() throws Exception {
		String source = Files.readString(Path.of(
			"src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		int eligibility = source.indexOf("public static boolean canUseParticleAtlas");
		int snapshot = source.indexOf("RustGalGuiRawImageAssets.semanticSnapshotUnstaged(atlasLocation)", eligibility);
		int fallback = source.indexOf("if (!(texture instanceof TextureAtlas candidate))", eligibility);
		int encoded = source.indexOf("encodeSemanticImageSnapshot(snapshot)", fallback);
		int registered = source.indexOf("registerWorldMeshTexture(", encoded);
		assertTrue(eligibility >= 0 && snapshot > eligibility,
			"reloadable particle textures must be admitted from a bounded semantic CPU snapshot");
		assertTrue(fallback >= 0 && encoded > fallback && registered > encoded,
			"reloadable particle snapshots must be encoded and registered as Rust-owned assets");
		assertTrue(source.substring(eligibility, fallback).contains("return snapshot != null"),
			"particle eligibility must not claim a reloadable texture without copied pixels");
	}

	@Test
	void noRenderParticleStateKeepsAnExplicitRustSemanticNoOp() throws Exception {
		String source = Files.readString(Path.of(
			"src/main/java/net/minecraft/client/particle/NoRenderParticleGroup.java"));
		int state = source.indexOf("EMPTY_RENDER_STATE = new ParticleGroupRenderState()");
		int submit = source.indexOf("public void submit(", state);
		int semantic = source.indexOf("public void submitSemantic(", submit);
		assertTrue(state >= 0 && submit > state && semantic > submit,
			"the intentionally empty particle state must not inherit the Java-only semantic fallback");
	}
}
