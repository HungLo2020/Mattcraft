package net.vulkanic.bridge;

import net.vulkanic.gui.AbsorptionHeartRequest;
import net.vulkanic.gui.AbsorptionHeartVariant;
import net.vulkanic.gui.AirBubbleRequest;
import net.vulkanic.gui.AirBubbleState;
import net.vulkanic.gui.ArmorIconState;
import net.vulkanic.gui.GuiHeartState;
import net.vulkanic.gui.HungerIconRequest;
import net.vulkanic.gui.HungerIconState;
import net.vulkanic.gui.HungerIconVariant;
import net.vulkanic.gui.MountHeartRequest;
import net.vulkanic.gui.MountHeartState;
import net.vulkanic.gui.MountHeartVariant;
import net.vulkanic.gui.PlayerHeartRequest;
import net.vulkanic.gui.PlayerHeartVariant;
import net.vulkanic.gui.RustGalFrameCoordinator;
import net.vulkanic.gui.RustGalGuiRenderer;
import net.vulkanic.world.RustGalWorldPrimitiveRenderer;
import net.vulkanic.world.WorldRenderRoutePolicy;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.assertSame;

import java.lang.foreign.Arena;
import java.lang.foreign.MemorySegment;
import java.util.List;
import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertArrayEquals;
import static org.junit.jupiter.api.Assertions.assertNotEquals;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

class VulkanicGalBridgeAbiTest {
	@Test
	void itemRasterPreservesBoundedAuthoredUvSubrectangles() {
		var quad = new VulkanicGalBridge.GuiAffineQuadRecord(1,7L,
			0,0,16,0,0,16,0,0.25F,0,0.75F,0.5F,-1,100,100)
			.withMaterialMode(1).withItemRasterScale(2).withSequence(3).withClip(0,0,20,20);
		assertEquals(0.25F,quad.u0());
		assertEquals(0.75F,quad.u1());
		assertEquals(0.5F,quad.v1());
		for (float u0 : new float[] {-0.001F,0.75F,Float.NaN}) {
			assertThrows(IllegalArgumentException.class, () -> new VulkanicGalBridge.GuiAffineQuadRecord(1,7L,
				0,0,16,0,0,16,0,u0,0,0.75F,0.5F,-1,100,100).withMaterialMode(1).withItemRasterScale(2));
		}
	}
	@Test
	void affineMaterialSurvivesSchedulingAndClipping() {
		var unlit = new VulkanicGalBridge.GuiAffineQuadRecord(1, 7L,
			0, 0, 16, 0, 0, 16, 0, 0, 0, 1, 1, -1, 100, 100);
		assertEquals(0, unlit.materialMode());
		var geometry = new VulkanicGalBridge.GuiItemRasterGeometryRecord(4,2,12,2,4,14);
		var item = unlit.withMaterialMode(1).withItemRasterScale(3).withItemRasterGeometry(geometry)
			.withSequence(4).withStratum(10).withClip(0, 0, 20, 20);
		assertEquals(geometry, item.itemRasterGeometry());
		assertEquals(geometry, item.withMaterialMode(2).withItemRasterScale(2).itemRasterGeometry());
		assertThrows(IllegalArgumentException.class, () -> unlit.withItemRasterGeometry(geometry));
		float[] copied = geometry.corners();
		copied[0] = 0;
		assertEquals(4, geometry.x0());
		assertThrows(IllegalArgumentException.class,
			() -> new VulkanicGalBridge.GuiItemRasterGeometryRecord(0,0,16,4,4,16));
		assertEquals(3, item.itemRasterScale());
		assertThrows(IllegalArgumentException.class, () -> unlit.withItemRasterScale(3));
		assertThrows(IllegalArgumentException.class, () -> item.withItemRasterScale(257));
		assertEquals(1, item.materialMode());
		assertEquals(4, item.sequence());
		assertEquals(10, item.stratum());
		assertEquals(20, item.clipWidth());
		assertEquals(2, item.withMaterialMode(2).withSequence(9).materialMode());
		assertThrows(IllegalArgumentException.class, () -> unlit.withMaterialMode(3));
	}

	private static String readRustFfiModules() throws Exception {
		Path root = Path.of("src/main/rust/render/vulkanic/ffi");
		StringBuilder source = new StringBuilder();
		try (java.util.stream.Stream<Path> paths = Files.walk(root)) {
			for (Path path : paths
				.filter(file -> Files.isRegularFile(file) && file.toString().endsWith(".rs"))
				.sorted()
				.toList()) {
				source.append(Files.readString(path)).append('\n');
			}
		}
		return source.toString();
	}

	@Test
	void javaLayoutsAreQueriedFromRustAbi() {
		for (VulkanicGalBridge.Struct struct : VulkanicGalBridge.Struct.values()) {
			assertTrue(struct.byteSize() > 0, "byte size should be reported for " + struct);
			assertTrue(struct.alignment() > 0, "alignment should be reported for " + struct);
			try (Arena arena = Arena.ofConfined()) {
				MemorySegment segment = struct.allocate(arena);
				assertEquals(struct.byteSize(), segment.byteSize(), "allocation should use Rust byte size for " + struct);
			}
		}
	}

	@Test
	void copiedEffectsUseSemanticIdentityWithoutLegacyEffectMarkers() throws Exception {
		String gameRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
		assertEquals(0, occurrences(gameRenderer, "RustGalGuiRenderer.enqueuePostEffectInvert"),
			"the inverse resource graph must not be replaced by a GUI marker");
		assertTrue(gameRenderer.contains("this.postEffectId.toString()"));
		assertEquals(0, occurrences(gameRenderer, "RustGalGuiRenderer.enqueuePostEffectCreeper"));
		assertEquals(0, occurrences(gameRenderer, "RustGalGuiRenderer.enqueuePostEffectSpider"));
		assertTrue(gameRenderer.contains("&& !net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()"),
			"Java PostChain processing must be disabled while Rust owns whole-frame Vulkan presentation");
		String guiRenderer = Files.readString(Path.of("src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"));
		assertTrue(guiRenderer.contains("enqueuePostEffectInvert")
			&& guiRenderer.contains("!isWholeFrameVulkanEnabled()"),
			"Rust GUI post-effect markers must be admitted during pre-selection whole-frame handoff");
	}

	@Test
	void javaPostChainCannotAdmitOrProcessPassesDuringRustPresentation() throws Exception {
		String postChain = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/PostChain.java"));
		int addToFrame = postChain.indexOf("public void addToFrame");
		int addGuard = postChain.indexOf("Java post-chain admission is unavailable while Rust owns whole-frame presentation", addToFrame);
		int process = postChain.indexOf("public void process");
		int processGuard = postChain.indexOf("Java post-chain processing is unavailable while Rust owns whole-frame presentation", process);
		assertTrue(addToFrame >= 0 && addGuard > addToFrame,
			"Java post-chain frame-graph admission must fail closed during Rust presentation");
		assertTrue(process >= 0 && processGuard > process,
			"deprecated Java post-chain execution must fail closed during Rust presentation");
	}

	@Test
	void wholeFrameMinecraftLoopDoesNotClearOrPresentThroughJava() throws Exception {
		String minecraft = Files.readString(Path.of("src/main/java/net/minecraft/client/Minecraft.java"));
		int shell = minecraft.indexOf("boolean rustWholeFrameShell");
		int compatibilityBranch = minecraft.indexOf("} else {", shell);
		assertTrue(shell >= 0 && compatibilityBranch > shell, "frame loop must branch on the Rust whole-frame shell");
		int javaClear = minecraft.indexOf("createCommandEncoder().clearColorAndDepthTextures", compatibilityBranch);
		int javaPresent = minecraft.indexOf("renderTarget.blitToScreen()", compatibilityBranch);
		assertTrue(javaClear > compatibilityBranch, "Java clear must remain compatibility-only");
		assertTrue(javaPresent > compatibilityBranch, "Java presentation must remain compatibility-only");
		assertTrue(minecraft.indexOf("createCommandEncoder().clearColorAndDepthTextures", shell) == javaClear,
			"whole-frame shell must not acquire a Java command encoder before the compatibility branch");
	}

	@Test
	void wholeFrameScreenEffectsUseSemanticGuiSubmissions() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/ScreenEffectRenderer.java"));
		int semanticStart = source.indexOf("renderRustVulkanScreenEffects");
		int semanticEnd = source.indexOf("private void renderItemActivationAnimation", semanticStart);
		assertTrue(semanticStart >= 0 && semanticEnd > semanticStart, "Rust screen-effect producer must remain explicit");
		String semantic = source.substring(semanticStart, semanticEnd);
		assertTrue(semantic.contains("submitRustSemanticTiledBlit"));
		assertTrue(semantic.contains("submitRustSemanticBlit"));
		assertFalse(semantic.contains("RenderType."), "Rust screen effects must not construct Java RenderType draws");
		assertFalse(semantic.contains("getBuffer("), "Rust screen effects must not acquire Java vertex consumers");
		int activation = source.indexOf("private void renderItemActivationAnimation");
		int lighting = source.indexOf("boolean rustSemanticItem", activation);
		assertTrue(lighting > activation && source.substring(lighting, source.indexOf("ItemStackRenderState itemStackRenderState", lighting))
			.contains("RustGalVulkanWholeFrameMode.enabled()"),
			"Rust item activation must fence Java lighting during pre-selection handoff");
		assertTrue(source.contains("Java underwater screen effect is unavailable while Rust owns whole-frame presentation")
			|| source.contains("Java underwater screen effect is unavailable on selected Vulkan"),
			"the legacy underwater helper must fail closed before consulting Iris or Java buffers");
	}

	@Test
	void selectedVulkanScreenEffectsCannotFallThroughToJavaBuffersOrIrisWaterPolicy() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/ScreenEffectRenderer.java"));
		assertTrue(source.contains("net.vulkanic.VulkanicAPI.isVulkanBackendSelected()")
			&& source.contains("Java screen-effect rendering is unavailable on selected Vulkan")
			&& source.contains("Java underwater screen effect is unavailable on selected Vulkan"),
			"selected Vulkan screen effects must fail closed before Java buffers or Iris water policy");
		int semantic = source.indexOf("renderRustVulkanScreenEffects");
		assertTrue(semantic >= 0 && source.substring(semantic).contains("submitRustSemanticTiledBlit"),
			"selected Vulkan screen effects must retain their semantic GUI producer");
	}

	@Test
	void selectedVulkanLightmapDoesNotInvokeJavaGpuHooks() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/LightTexture.java"));
		int hooks = source.indexOf("HookRegistry.getLightTextureHooks()");
		int guard = source.lastIndexOf("!VulkanicAPI.isVulkanBackendSelected()", hooks);
		assertTrue(hooks >= 0 && guard >= 0 && guard < hooks
			&& source.indexOf("!net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()", guard) < hooks,
			"Rust Vulkan lightmap semantics must not invoke Java GPU extension hooks");
	}

	@Test
	void wholeFramePanoramaCannotFallBackToJavaCubeMapRendering() throws Exception {
		String panorama = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/PanoramaRenderer.java"));
		int enqueue = panorama.indexOf("RustGalPanoramaRenderer.enqueue");
		int fallback = panorama.indexOf("this.cubeMap.render", enqueue);
		assertTrue(enqueue >= 0 && fallback > enqueue, "panorama must retain the non-Vulkan compatibility renderer");
		String branch = panorama.substring(enqueue, fallback);
		assertTrue(branch.contains("isWholeFrameVulkanEnabled()"),
			"Rust whole-frame panorama admission must guard the Java compatibility renderer");
		assertTrue(branch.contains("panorama asset is unavailable"),
			"an unavailable Rust panorama must fail closed rather than silently render in Java");
	}

	@Test
	void seededWorldBackgroundUsesVanillaFogClearRatherThanBiomeSkyColor() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		int seedStart = source.indexOf("private static void seedBackgroundFrameLocked");
		int seedEnd = source.indexOf("private static void auditSemanticInputGap", seedStart);
		assertTrue(seedStart >= 0 && seedEnd > seedStart, "world-background seed must remain explicit");
		String seed = source.substring(seedStart, seedEnd);
		assertTrue(seed.contains("Vector3f fogColor = shaderPackFogColor(level, camera)")
			&& seed.contains("int fogColorArgb = ARGB.color("),
			"the seed must copy vanilla fog RGB for LevelRenderer's clear semantics");
		assertTrue(seed.contains("fogColorArgb,")
			&& !seed.contains("ARGB.red(skyColor), ARGB.green(skyColor), ARGB.blue(skyColor)"),
			"biome sky colour belongs to the finite Rust sky-disc uniform, never the background clear");
	}

	@Test
	void selectedVulkanPanoramaCannotRegisterOrRenderThroughJavaCubemap() throws Exception {
		String cubeMap = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/CubeMap.java"));
		String panorama = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/PanoramaRenderer.java"));
		assertTrue(cubeMap.contains("VulkanicAPI.isVulkanBackendSelected()"),
			"selected Vulkan cubemaps must remain semantic resources");
		assertTrue(cubeMap.contains("requires PanoramaRenderer to attach its semantic command")
			&& cubeMap.contains("Java cube-map rendering is not a fallback"),
			"selected Vulkan direct cubemap rendering must fail closed before Java draws or detached Rust work can accumulate");
		assertTrue(panorama.contains("VulkanicAPI.isVulkanBackendSelected()")
			&& panorama.contains("panorama asset is unavailable"),
			"selected Vulkan panorama rendering must not fall through to Java");
	}

	@Test
	void wholeFrameGuardianBeamCannotFallBackToJavaCustomGeometry() throws Exception {
		String guardian = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/entity/GuardianRenderer.java"));
		String primitive = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		int admission = guardian.indexOf("submitNodeCollector.submitGuardianBeam");
		int fallback = guardian.indexOf("submitCustomGeometry", admission);
		assertTrue(admission >= 0 && fallback > admission, "Guardian beam must retain compatibility geometry after semantic admission");
		String branch = guardian.substring(admission, fallback);
		assertTrue(branch.contains("currentGuardianBeamRoute().usesRustWholeFrameVulkan()"));
		assertTrue(branch.contains("Rust whole-frame Guardian beam route rejected"));
		String routePolicy = Files.readString(Path.of("src/main/java/net/vulkanic/world/WorldRenderRoutePolicy.java"));
		int guardianRoute = routePolicy.indexOf("public static Route currentGuardianBeamRoute()");
		assertTrue(guardianRoute >= 0);
		String guardianRouteBody = routePolicy.substring(guardianRoute, routePolicy.indexOf("\n\t}\n", guardianRoute));
		assertTrue(guardianRouteBody.contains("RustGalVulkanWholeFrameMode.enabled()"),
			"Guardian beam ownership must stay Rust-owned during the pre-selection whole-frame handoff");
		assertTrue(primitive.contains("Rust VulkanicGAL Guardian beam requires a seeded bounded world primitive frame"));
	}

	@Test
	void wholeFrameEndCrystalBeamCannotFallBackToJavaCustomGeometry() throws Exception {
		String dragon = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/entity/EnderDragonRenderer.java"));
		String levelRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/LevelRenderer.java"));
		String rustMaterials = Files.readString(Path.of("src/main/rust/render/vulkanic/world_primitive_frontend/material_registry.rs"));
		String primitive = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		int admission = dragon.indexOf("submitNodeCollector.submitCrystalBeam");
		int fallback = dragon.indexOf("submitCustomGeometry", admission);
		assertTrue(admission >= 0 && fallback > admission, "End Crystal beam must retain compatibility geometry after semantic admission");
		String branch = dragon.substring(admission, fallback);
		assertTrue(branch.contains("if (rustCrystalBeam)"));
		assertTrue(dragon.contains("currentCrystalBeamRoute()")
			&& dragon.contains("usesRustWholeFrameVulkan()"),
			"End Crystal beam semantic admission must follow explicit route policy during handoff");
		assertTrue(branch.contains("End Crystal beam route rejected"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.EndCrystalRenderState"));
		assertTrue(levelRenderer.contains("\"model-mesh\", \"rust-vulkan-whole-frame:end-crystal\""));
		assertTrue(levelRenderer.contains("\"conduit\".equals(MODEL_MESH_SCENARIO)"));
		assertTrue(rustMaterials.contains("WORLD_MATERIAL_TEXTURE_CRYSTAL_BEAM"));
		int crystal = primitive.indexOf("MATERIAL_TEXTURE_CRYSTAL_BEAM, MATERIAL_MODE_TRANSLUCENT");
		String crystalRecord = crystal < 0 ? "" : primitive.substring(crystal, Math.min(primitive.length(), crystal + 320));
		assertTrue(crystal >= 0 && crystalRecord.contains("DEPTH_POLICY_TEST_NO_WRITE")
			&& !crystalRecord.contains("DEPTH_POLICY_TEST_WRITE"),
			"crystal beam translucency must use explicit depth-test/no-write semantics");
		assertTrue(primitive.contains("Rust crystal beam requires a seeded bounded world primitive frame"));
	}

	@Test
	void conduitModelPartCaptureSeparatesGameplayIdentityFromAtlasDiagnostic() throws Exception {
		String capture = Files.readString(Path.of("src/main/java/net/minecraft/client/dev/DeterministicCameraCapture.java"));
		int modelPartGate = capture.indexOf("hasCurrentModelPartMeshTraversal");
		int queuedCheck = capture.indexOf("boolean queued =", modelPartGate);
		assertTrue(modelPartGate >= 0 && queuedCheck > modelPartGate);
		String gate = capture.substring(queuedCheck, Math.min(capture.length(), queuedCheck + 500));
		assertTrue(gate.contains("expectedModelMeshDiagnosticTextureId().equals(diagnostic.textureId())"));
		assertTrue(gate.contains("\"model-part\".equals(diagnostic.provenance())"));
	}

	@Test
	void disabledWholeFrameDebugGeometryCannotFallBackToJava() throws Exception {
		String boxes = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/blockentity/BlockEntityWithBoundingBoxRenderer.java"));
		assertTrue(boxes.contains("Rust whole-frame bounding-box route is unavailable"));
		assertTrue(boxes.contains("Rust whole-frame invisible-block route is unavailable"));
		assertTrue(boxes.contains("!submitNodeCollector.isSemanticCoverageOnly()"),
			"coverage-only bounding-box extraction must not enqueue duplicate Rust line work");
		assertTrue(boxes.contains("if (submitNodeCollector.isSemanticCoverageOnly()) return;"),
			"coverage-only invisible-block extraction must acknowledge the Rust route without staging work");
		String testInstance = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/blockentity/TestInstanceRenderer.java"));
		assertTrue(testInstance.contains("Rust whole-frame error-marker route is unavailable"));
		assertTrue(testInstance.contains("ordered.submitTextSemantic(") && !testInstance.contains("ordered.submitText("),
			"Rust error-marker labels must use explicit semantic world text without a legacy fallback");
	}

	@Test
	void wholeFrameMapGeometryCannotFallBackToJava() throws Exception {
		String map = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/MapRenderer.java"));
		assertTrue(map.contains("Rust whole-frame map route is unavailable"));
		assertTrue(map.contains("Rust whole-frame map-decoration route is unavailable"));
		assertTrue(map.contains("ordered.submitTextSemantic(") && !map.contains("ordered.submitText("),
			"map-decoration labels must use explicit semantic world text without a legacy fallback");
		String hand = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/ItemInHandRenderer.java"));
		assertTrue(hand.contains("Rust whole-frame first-person map route is unavailable"));
	}

	@Test
	void disabledWholeFrameTaczRouteCannotFallBackToJavaGeometry() throws Exception {
		String tacz = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/special/TaczGlock17SpecialRenderer.java"));
		assertTrue(tacz.contains("Rust whole-frame TACZ route is unavailable"));
		assertTrue(tacz.contains("RustGalVulkanWholeFrameMode.enabled()"));
	}

	@Test
	void wholeFrameVoxelMapWaypointsCannotSilentlyDropRejectedSemanticQuads() throws Exception {
		String voxel = Files.readString(Path.of("src/main/java/net/voxelmap/VoxelConstants.java"));
		int waypointStart = voxel.indexOf("submitRustWaypointSemantics");
		assertTrue(waypointStart >= 0);
		String source = voxel.substring(waypointStart);
		assertTrue(source.contains("Rust whole-frame waypoint icon route rejected semantic quad"));
		assertTrue(source.contains("Rust whole-frame waypoint label route rejected semantic background"));
		assertTrue(source.contains("collector.submitTextSemantic("),
			"Rust-only VoxelMap waypoint labels must use the explicit world-text semantic callback");
		assertTrue(source.contains("currentTexturedBillboardRoute().usesRustWholeFrameVulkan()) return;"),
			"VoxelMap waypoint semantics must defer until the textured-billboard route is admitted");
	}

	@Test
	void disabledWholeFrameShadowAndLeashRoutesCannotSilentlyDropSubmissions() throws Exception {
		String dispatcher = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/feature/FeatureRenderDispatcher.java"));
		assertTrue(dispatcher.contains("Rust whole-frame entity-shadow route is unavailable"));
		assertTrue(dispatcher.contains("Rust whole-frame entity-leash route is unavailable"));
	}

	@Test
	void disabledWholeFrameDebugHitboxRouteCannotSilentlyDropSubmissions() throws Exception {
		String dispatcher = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/feature/FeatureRenderDispatcher.java"));
		assertTrue(dispatcher.contains("validateRustHitboxRoute"));
		assertTrue(dispatcher.contains("Rust whole-frame debug-hitbox route is unavailable"));
		String levelRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/LevelRenderer.java"));
		assertTrue(levelRenderer.contains("featureRenderDispatcher.validateRustHitboxRoute"));
	}

	@Test
	void blockOnlyFeatureExtractionCannotSilentlyDropDisabledRustFeatures() throws Exception {
		String dispatcher = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/feature/FeatureRenderDispatcher.java"));
		int method = dispatcher.indexOf("void renderBlockFeaturesOnly");
		assertTrue(method >= 0);
		String source = dispatcher.substring(method);
		assertTrue(source.contains("Rust whole-frame entity-shadow route is unavailable"));
		assertTrue(source.contains("Rust whole-frame entity-flame route is unavailable"));
		assertTrue(source.contains("Rust whole-frame entity-leash route is unavailable"));
	}

	@Test
	void overlayTextureDoesNotTouchIrisStateDuringRustWholeFrame() throws Exception {
		String overlay = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/texture/OverlayTexture.java"));
		assertTrue(occurrences(overlay, "RustGalVulkanWholeFrameMode.enabled()") >= 4);
		assertTrue(overlay.contains("RustGalVulkanWholeFrameMode.enabled()"),
			"semantic whole-frame entity data must not bind or unbind the Java/Iris overlay texture");
	}

	@Test
	void semanticTextureViewsDoNotPublishJavaTexturesToIrisTracker() throws Exception {
		String abstractTexture = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/texture/AbstractTexture.java"));
		String reloadableTexture = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/texture/ReloadableTexture.java"));
		assertEquals(4, occurrences(abstractTexture, "RustGalVulkanWholeFrameMode.enabled()"));
		assertTrue(abstractTexture.contains("!net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()")
			&& abstractTexture.contains("!net.vulkanic.VulkanicAPI.isVulkanBackendSelected()"));
		assertTrue(reloadableTexture.contains("!net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()")
			&& reloadableTexture.contains("!net.vulkanic.VulkanicAPI.isVulkanBackendSelected()"),
			"semantic resource-pack uploads must not register Java GPU handles with Iris");
		String renderStateShard = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/RenderStateShard.java"));
		assertEquals(4, occurrences(renderStateShard, "RustGalVulkanWholeFrameMode.enabled()"));
		assertTrue(renderStateShard.contains("var ctx = VulkanicAPI.getCommandContext();"),
			"the compatibility texture setup must remain isolated behind its whole-frame guard");
	}

	@Test
	void rustWholeFrameRenderStateShardsCannotBindJavaTextureUnits() throws Exception {
		String renderStateShard = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/RenderStateShard.java"));
		int textureShard = renderStateShard.indexOf("public static class TextureStateShard");
		int multiShard = renderStateShard.indexOf("public static class MultiTextureStateShard");
		assertTrue(textureShard >= 0 && multiShard >= 0);
		String textureSource = renderStateShard.substring(textureShard);
		String multiSource = renderStateShard.substring(multiShard, textureShard);
		assertTrue(textureSource.contains("RustGalVulkanWholeFrameMode.enabled()")
			&& textureSource.contains("return;"),
			"single-texture compatibility setup must be inert while Rust owns the frame");
		assertTrue(multiSource.contains("RustGalVulkanWholeFrameMode.enabled()")
			&& multiSource.contains("return;"),
			"multi-texture compatibility setup must be inert while Rust owns the frame");
		String renderType = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/RenderType.java"));
		assertTrue(renderType.contains("Java immediate RenderType drawing is unavailable while Rust owns whole-frame presentation")
			|| renderType.contains("Java immediate RenderType drawing is unavailable on selected Vulkan"),
			"immediate Java RenderType submission must remain unavailable on the Rust route");
	}

	@Test
	void selectedVulkanRenderStateShardsCannotBindJavaTextureUnits() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/RenderStateShard.java"));
		assertTrue(source.contains("Java Vulkan render-state setup is unavailable on selected Vulkan")
			&& source.contains("Java Vulkan render-state cleanup is unavailable on selected Vulkan"),
			"selected Vulkan render-state entrypoints must fail closed before compatibility state");
		assertTrue(source.contains("net.vulkanic.VulkanicAPI.isVulkanBackendSelected()")
			&& source.contains("TextureTracker.INSTANCE.onSetShaderTexture"),
			"texture binding hooks must remain fenced behind selected-Vulkan admission");
	}

	@Test
	void selectedVulkanImmediateRenderTypesCannotIssueJavaDraws() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/RenderType.java"));
		int draw = source.indexOf("public void draw(MeshData meshData)");
		assertTrue(draw >= 0
			&& source.indexOf("isVulkanBackendSelected()", draw) > draw
			&& source.indexOf("setupRenderState();", draw) > source.indexOf("isVulkanBackendSelected()", draw),
			"selected Vulkan immediate RenderType draws must fail before Java state or uploads");
		assertTrue(source.contains("Java immediate RenderType drawing is unavailable on selected Vulkan"));
		String wrapped = Files.readString(Path.of("src/main/java/net/irisshaders/iris/layer/OuterWrappedRenderType.java"));
		assertTrue(wrapped.contains("Java wrapped RenderType drawing is unavailable on the Rust Vulkan route"),
			"Iris wrapped RenderTypes must not bypass the base RenderType selected-Vulkan draw fence");
		String textureUtil = Files.readString(Path.of("src/main/java/net/blaze3d/platform/TextureUtil.java"));
		String voxelMapReadback = Files.readString(Path.of("src/main/java/net/voxelmap/util/GLUtils.java"));
		assertTrue(textureUtil.contains("Java texture readback is unavailable on the Rust Vulkan route")
			&& voxelMapReadback.contains("Java VoxelMap texture readback is unavailable on the Rust Vulkan route"),
			"diagnostic readback utilities must not open Java GPU encoders on selected Vulkan");
	}

	@Test
	void selectedVulkanShaderManagerCannotPrecompileJavaPipelines() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/ShaderManager.java"));
		int apply = source.indexOf("protected void apply(");
		int precompile = source.indexOf("VulkanicAPI.precompileRenderPipeline", apply);
		assertTrue(apply >= 0 && precompile > apply
			&& source.substring(apply, precompile).contains("isVulkanBackendSelected()"),
			"selected Vulkan shader reload must keep Java pipeline compilation unavailable");
		assertTrue(source.substring(apply, precompile).contains("this.compilationCache.close()"),
			"selected Vulkan shader reload must retain source cache ownership without Java pipeline compilation");
	}

	@Test
	void selectedVulkanIrisHandRoutesCannotEnterJavaProjectionOrFeatureDraws() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/irisshaders/iris/pathways/HandRenderer.java"));
		assertTrue(source.contains("Java Iris hand rendering is unavailable on selected Vulkan")
			&& source.contains("Java Iris translucent hand rendering is unavailable on selected Vulkan"),
			"selected Vulkan Iris hand routes must fail closed before Java projection and feature draws");
		assertTrue(source.indexOf("RustGalVulkanWholeFrameMode.enabled()") >= 0
			&& source.indexOf("RustGalVulkanWholeFrameMode.enabled()", source.indexOf("RustGalVulkanWholeFrameMode.enabled()") + 1) >= 0,
			"Iris hand routes must also fail closed when Rust whole-frame ownership is active independently");
	}

	@Test
	void selectedVulkanIrisRenderSystemCannotInitializeJavaProjectionOrCapabilities() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/irisshaders/iris/gl/IrisRenderSystem.java"));
		int init = source.indexOf("public static void initRenderer()");
		int capabilities = source.indexOf("VulkanicAPI.getGraphicsCapabilities()", init);
		assertTrue(init >= 0 && capabilities > init
			&& source.substring(init, capabilities).contains("isVulkanBackendSelected()")
			&& source.substring(init, capabilities).contains("perspectiveProjectionMatrixBuffer = null"),
			"selected Vulkan Iris initialization must not construct Java capabilities or projection UBOs");
	}

	@Test
	void selectedVulkanIrisGpuCreationAndMutationGatesFailClosed() throws Exception {
		String renderSystem = Files.readString(Path.of("src/main/java/net/irisshaders/iris/gl/IrisRenderSystem.java"));
		String storage = Files.readString(Path.of("src/main/java/net/irisshaders/iris/gl/buffer/ShaderStorageBuffer.java"));
		int creation = renderSystem.indexOf("private static void rejectJavaGpuObjectCreation");
		int mutation = renderSystem.indexOf("private static void rejectJavaGpuMutation");
		assertTrue(creation >= 0 && renderSystem.indexOf("isVulkanBackendSelected()", creation) > creation,
			"selected Vulkan must reject central Iris Java GPU object creation before allocation");
		assertTrue(mutation >= 0 && renderSystem.indexOf("isVulkanBackendSelected()", mutation) > mutation,
			"selected Vulkan must reject central Iris Java GPU mutation before issuing commands");
		assertTrue(storage.contains("VulkanicAPI.isVulkanBackendSelected()")
			&& storage.contains("Java Iris shader-storage buffers are unavailable on selected Vulkan")
			&& storage.contains("Java Iris shader-storage buffer resizing is unavailable on selected Vulkan"),
			"selected Vulkan must reject Iris shader-storage allocation and resizing before backend initialization");
		for (String method : new String[] {"supportsSSBO", "supportsImageLoadStore", "supportsBufferBlending"}) {
			int start = renderSystem.indexOf("public static boolean " + method + "(");
			int end = renderSystem.indexOf("\n\t}", start);
			assertTrue(start >= 0 && end > start,
				"Iris capability query must remain a concrete method: " + method);
			String body = renderSystem.substring(start, end);
			assertTrue(body.contains("rejectJavaGpuQuery("),
				"selected Vulkan must fence borrowed Iris capability state before " + method);
		}
	}

	@Test
	void selectedVulkanIrisSamplerConstructionCannotBorrowJavaGpuState() throws Exception {
		String samplers = Files.readString(Path.of("src/main/java/net/irisshaders/iris/samplers/IrisSamplers.java"));
		assertTrue(samplers.contains("Java Iris sampler state is unavailable on the Rust Vulkan route"),
			"Iris sampler construction must fail closed on selected Vulkan");
		int guard = samplers.indexOf("private static void ensureJavaCompatibilityRoute");
		int init = samplers.indexOf("public static void initRenderer");
		int renderTargets = samplers.indexOf("public static void addRenderTargetSamplers");
		int shadow = samplers.indexOf("public static boolean addShadowSamplers");
		assertTrue(guard >= 0 && init > guard && renderTargets > guard && shadow > guard,
			"all Iris sampler GPU entrypoints must share the selected-Vulkan ownership guard");
		assertTrue(samplers.contains("ensureJavaCompatibilityRoute();\n\t\tSHADOW_SAMPLER_NEAREST")
			&& samplers.contains("ensureJavaCompatibilityRoute();\n\t\tboolean usesShadows"),
			"sampler object creation and shadow texture-ID suppliers must be guarded before registration");
	}

	@Test
	void selectedVulkanIrisPipelineCannotConstructJavaTargetsBeforeBackendInitialization() throws Exception {
		String iris = Files.readString(Path.of("src/main/java/net/irisshaders/iris/Iris.java"));
		String shadows = Files.readString(Path.of("src/main/java/net/irisshaders/iris/shadows/ShadowRenderTargets.java"));
		int createPipeline = iris.indexOf("private static WorldRenderingPipeline createPipeline");
		int pipelineGuard = iris.indexOf("VulkanicAPI.isVulkanBackendSelected()", createPipeline);
		assertTrue(createPipeline >= 0 && pipelineGuard > createPipeline,
			"selected Vulkan must keep Iris from constructing its Java shader pipeline before backend initialization");
		assertTrue(shadows.contains("if (VulkanicAPI.isVulkanBackendSelected())"),
			"selected Vulkan must reject direct Iris shadow-target construction before Java GPU allocation");
	}

	@Test
	void selectedVulkanMappableRingBuffersCannotAllocateJavaBuffersBeforeInitialization() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/MappableRingBuffer.java"));
		int constructor = source.indexOf("public MappableRingBuffer(");
		int rotation = source.indexOf("public void rotate()");
		assertTrue(constructor >= 0 && source.indexOf("isVulkanBackendSelected()", constructor) > constructor,
			"selected Vulkan must reject Java mappable ring-buffer allocation before backend initialization");
		assertTrue(rotation >= 0 && source.indexOf("isVulkanBackendSelected()", rotation) > rotation,
			"selected Vulkan must reject Java mappable ring-buffer rotation before fence submission");
		int current = source.indexOf("public GpuBuffer currentBuffer()");
		int fence = source.indexOf("GpuFence gpuFence", current);
		assertTrue(current >= 0 && source.indexOf("isVulkanBackendSelected()", current) < fence,
			"late Java ring-buffer access must fail before fence waits or buffer reuse");
	}

	@Test
	void selectedVulkanProjectionBuffersCannotAllocateJavaUbosBeforeInitialization() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/PerspectiveProjectionMatrixBuffer.java"));
		int constructor = source.indexOf("public PerspectiveProjectionMatrixBuffer(");
		int allocation = source.indexOf("VulkanicAPI.createBuffer", constructor);
		assertTrue(constructor >= 0 && source.indexOf("isVulkanBackendSelected()", constructor) > constructor
			&& allocation > constructor,
			"selected Vulkan projection buffers must become inert before Java UBO allocation");
		assertTrue(source.contains("Java projection UBO rendering is unavailable on selected Vulkan"),
			"selected Vulkan projection UBO use must fail closed");
	}

	@Test
	void selectedVulkanCachedProjectionBuffersCannotAllocateJavaUbosBeforeInitialization() throws Exception {
		String perspective = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/CachedPerspectiveProjectionMatrixBuffer.java"));
		String ortho = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/CachedOrthoProjectionMatrixBuffer.java"));
		assertTrue(perspective.contains("isVulkanBackendSelected()")
			&& perspective.contains("Java cached projection UBO rendering is unavailable on selected Vulkan"),
			"selected Vulkan cached perspective projections must be inert before Java UBO allocation");
		assertTrue(ortho.contains("isVulkanBackendSelected()")
			&& ortho.contains("Java cached orthographic UBO rendering is unavailable on selected Vulkan"),
			"selected Vulkan cached orthographic projections must be inert before Java UBO allocation");
	}

	@Test
	void selectedVulkanUniformStorageCannotAllocateJavaUbosBeforeInitialization() throws Exception {
		String global = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/GlobalSettingsUniform.java"));
		String dynamic = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/DynamicUniformStorage.java"));
		assertTrue(global.contains("isVulkanBackendSelected()")
			&& global.contains("Java global-settings UBO is unavailable on selected Vulkan"),
			"selected Vulkan global settings must not allocate a Java UBO");
		assertTrue(dynamic.contains("isVulkanBackendSelected()")
			&& dynamic.contains("Java dynamic-uniform storage is unavailable on selected Vulkan"),
			"selected Vulkan dynamic uniforms must not allocate Java ring storage");
		int update = global.indexOf("public void update(");
		int stack = global.indexOf("MemoryStack.stackPush()", update);
		assertTrue(update >= 0 && global.indexOf("RustGalVulkanWholeFrameMode.enabled()", update) < stack,
			"global settings updates must stop before Java UBO writes during Rust handoff");
	}

	@Test
	void selectedVulkanRenderTargetsCannotAllocateJavaAttachmentsBeforeInitialization() throws Exception {
		String main = Files.readString(Path.of("src/main/java/net/blaze3d/pipeline/MainTarget.java"));
		String target = Files.readString(Path.of("src/main/java/net/blaze3d/pipeline/RenderTarget.java"));
		assertTrue(main.contains("isVulkanBackendSelected()")
			&& main.contains("acquired presentation images"),
			"selected Vulkan main target must retain dimensions without creating Java attachments");
		int allocation = target.indexOf("VulkanicAPI.createTexture", target.indexOf("public void createBuffers"));
		int guard = target.indexOf("isVulkanBackendSelected()", target.indexOf("public void createBuffers"));
		assertTrue(guard >= 0 && allocation > guard,
			"selected Vulkan render targets must reject Java color/depth allocation before creation");
	}

	@Test
	void selectedVulkanLightingAndTerrainUploadsCannotAllocateJavaResourcesBeforeInitialization() throws Exception {
		String lighting = Files.readString(Path.of("src/main/java/net/blaze3d/platform/Lighting.java"));
		String mesh = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/chunk/CompiledSectionMesh.java"));
		assertTrue(lighting.contains("isVulkanBackendSelected()")
			&& lighting.contains("this.buffer = null"),
			"selected Vulkan lighting must not allocate the Java lighting UBO");
		assertTrue(mesh.contains("isVulkanBackendSelected()")
			&& mesh.contains("Java terrain mesh uploads are unavailable while Rust owns whole-frame presentation")
			&& mesh.contains("Java terrain index uploads are unavailable while Rust owns whole-frame presentation"),
			"selected Vulkan terrain uploads must fail closed before Java buffers or encoders are created");
	}

	@Test
	void selectedVulkanImmediateVertexUploadsCannotReopenJavaBuffers() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/blaze3d/vertex/VertexFormat.java"));
		int upload = source.indexOf("private static GpuBuffer uploadToBuffer(");
		int selectedGuard = source.indexOf("isVulkanBackendSelected()", upload);
		int allocation = source.indexOf("VulkanicAPI.createBuffer", upload);
		assertTrue(upload >= 0 && selectedGuard > upload && selectedGuard < allocation
			&& source.contains("Java immediate vertex uploads are unavailable on selected Vulkan"),
			"selected Vulkan immediate uploads must fail before Java vertex/index buffers are created");
	}

	@Test
	void selectedVulkanCubemapLoadsCannotAllocateJavaTextures() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/texture/CubeMapTexture.java"));
		int load = source.indexOf("protected void doLoad(");
		int guard = source.indexOf("isVulkanBackendSelected()", load);
		int allocation = source.indexOf("VulkanicAPI.createTexture", load);
		assertTrue(load >= 0 && guard > load && allocation > guard
			&& source.contains("Rust owns cubemap admission for Vulkan panoramas"),
			"selected Vulkan cubemap loads must remain CPU/semantic-owned before Java texture allocation");
	}

	@Test
	void selectedVulkanTracyCaptureCannotAllocateJavaResources() throws Exception {
		String tracy = Files.readString(Path.of("src/main/java/net/blaze3d/TracyFrameCapture.java"));
		String minecraft = Files.readString(Path.of("src/main/java/net/minecraft/client/Minecraft.java"));
		int constructor = tracy.indexOf("public TracyFrameCapture()");
		int allocation = tracy.indexOf("VulkanicAPI.createTexture", constructor);
		assertTrue(constructor >= 0 && tracy.indexOf("isVulkanBackendSelected()", constructor) > constructor
			&& allocation > constructor
			&& tracy.contains("Java Tracy frame capture is unavailable on selected Vulkan"),
			"selected Vulkan Tracy capture must fail before Java capture textures/buffers are allocated");
		assertTrue(minecraft.contains("!net.vulkanic.VulkanicAPI.isVulkanBackendSelected()"),
			"Minecraft must not construct Java Tracy capture resources on selected Vulkan");
	}

	@Test
	void selectedVulkanScreenshotReadbackCannotAllocateJavaBuffers() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/Screenshot.java"));
		int method = source.indexOf("public static void takeScreenshot(");
		int guard = source.indexOf("isVulkanBackendSelected()", method);
		int allocation = source.indexOf("VulkanicAPI.createBuffer", method);
		assertTrue(method >= 0 && guard > method && allocation > guard
			&& source.contains("Java screenshot readback is unavailable on selected Vulkan"),
			"selected Vulkan screenshot readback must fail before Java staging-buffer allocation");
	}

	@Test
	void selectedVulkanParticlesCannotFallBackToJavaRenderPasses() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/feature/ParticleFeatureRenderer.java"));
		int render = source.indexOf("public void render(");
		int selectedGuard = source.indexOf("isVulkanBackendSelected()", render);
		int javaPass = source.indexOf("createRenderPass", render);
		assertTrue(render >= 0 && selectedGuard > render && selectedGuard < javaPass
			&& source.contains("Java particle rendering is not a fallback"),
			"selected Vulkan particles must fail closed before Java particle render passes");
	}

	@Test
	void selectedVulkanGraphicsBenchmarkCannotRunJavaWorkloads() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/dev/GraphicsSubsystemBenchmark.java"));
		int redirect = source.indexOf("if ((net.vulkanic.VulkanicAPI.isVulkanBackendSelected()");
		int javaWorkload = source.indexOf("precompileVulkanSubsystemPipelines", redirect);
		assertTrue(redirect >= 0 && javaWorkload > redirect
			&& source.substring(redirect, javaWorkload).contains("RustGraphicsSubsystemBenchmark.run"),
			"selected Vulkan benchmark requests must not execute Java buffer or render-pass workloads");
	}

	@Test
	void selectedVulkanGenericBufferAndFramegraphRoutesCannotReopenJavaState() throws Exception {
		String buffers = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/MultiBufferSource.java"));
		String descriptor = Files.readString(Path.of("src/main/java/net/blaze3d/resource/RenderTargetDescriptor.java"));
		String loading = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/screens/LoadingOverlay.java"));
		assertTrue(buffers.contains("isVulkanBackendSelected()")
			&& buffers.contains("Java Vulkan buffer-source rendering is unavailable on selected Vulkan"),
			"selected Vulkan buffer sources must not admit Java vertex assembly");
		assertTrue(descriptor.contains("isVulkanBackendSelected()"),
			"selected Vulkan framegraph target preparation must not clear Java attachments");
		assertTrue(loading.contains("isVulkanBackendSelected()"),
			"selected Vulkan loading overlay must not register Java logo textures");
		assertTrue(loading.contains("KEEP_BACKEND_SEAM_REFERENCE")
			&& loading.contains("!net.vulkanic.VulkanicAPI.isVulkanBackendSelected()")
			&& loading.contains("!net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()"),
			"loading-overlay backend seam probes must remain disabled under Rust ownership");
	}

	@Test
	void selectedVulkanIrisCompatibilityTexturesCannotUploadJavaGpuState() throws Exception {
		String custom = Files.readString(Path.of("src/main/java/net/irisshaders/iris/targets/backed/NativeImageBackedCustomTexture.java"));
		String noise = Files.readString(Path.of("src/main/java/net/irisshaders/iris/targets/backed/NativeImageBackedNoiseTexture.java"));
		String color = Files.readString(Path.of("src/main/java/net/irisshaders/iris/pathways/colorspace/ColorSpaceFragmentConverter.java"));
		assertTrue(custom.contains("isVulkanBackendSelected()")
			&& custom.contains("Java Iris custom texture uploads are unavailable on selected Vulkan"),
			"selected Vulkan Iris custom texture construction must not upload to a Java texture");
		assertTrue(noise.contains("isVulkanBackendSelected()")
			&& noise.contains("Java Iris noise texture uploads are unavailable on selected Vulkan"),
			"selected Vulkan Iris noise texture construction must not upload to a Java texture");
		assertTrue(color.contains("isVulkanBackendSelected()"),
			"selected Vulkan Iris color-space conversion must not construct Java post-process resources");
	}

	@Test
	void selectedVulkanVoxelMapAtlasCannotAllocateJavaTextures() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/voxelmap/textures/TextureAtlas.java"));
		int stitch = source.indexOf("public void stitch()");
		int stitchGuard = source.indexOf("isVulkanBackendSelected()", stitch);
		int stitchAllocation = source.indexOf("VulkanicAPI.createTexture", stitch);
		int stitchNew = source.indexOf("public void stitchNew()");
		int newGuard = source.indexOf("isVulkanBackendSelected()", stitchNew);
		int newAllocation = source.indexOf("VulkanicAPI.createTexture", stitchNew);
		assertTrue(stitchGuard > stitch && stitchAllocation > stitchGuard
			&& newGuard > stitchNew && newAllocation > newGuard,
			"selected Vulkan VoxelMap atlas stitching must stay CPU/semantic-owned before Java texture allocation");
		String map = Files.readString(Path.of("src/main/java/net/voxelmap/Map.java"));
		assertTrue(map.contains("!net.vulkanic.VulkanicAPI.isVulkanBackendSelected()"),
			"selected Vulkan VoxelMap map setup must not allocate its Java offscreen target");
		String allocated = Files.readString(Path.of("src/main/java/net/voxelmap/util/AllocatedTexture.java"));
		assertTrue(allocated.contains("Java VoxelMap allocated texture views are unavailable on the Rust Vulkan route"),
			"selected Vulkan VoxelMap texture wrappers must not publish Java texture views");
	}

	@Test
	void selectedVulkanVoxelMapEntityImagesCannotOpenJavaRenderPasses() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/voxelmap/entityrender/EntityMapImageManager.java"));
		int method = source.indexOf("public Sprite requestImageForMob(");
		int guard = source.indexOf("isVulkanBackendSelected()", method);
		int renderPass = source.indexOf("createRenderPass", method);
		assertTrue(method >= 0 && guard > method && renderPass > guard,
			"selected Vulkan VoxelMap entity images must fail before Java model buffers or render passes");
	}

	@Test
	void selectedVulkanIrisCompositeRenderersCannotOpenJavaPasses() throws Exception {
		String composite = Files.readString(Path.of("src/main/java/net/irisshaders/iris/pipeline/CompositeRenderer.java"));
		String shadow = Files.readString(Path.of("src/main/java/net/irisshaders/iris/shadows/ShadowCompositeRenderer.java"));
		assertTrue(composite.contains("isVulkanBackendSelected()")
			&& composite.contains("Java Iris composite rendering is unavailable on selected Vulkan"),
			"selected Vulkan Iris composites must fail before Java compute/render passes");
		assertTrue(shadow.contains("isVulkanBackendSelected()")
			&& shadow.contains("Java Iris shadow-composite rendering is unavailable on selected Vulkan"),
			"selected Vulkan Iris shadow composites must fail before Java compute/render passes");
	}

	@Test
	void selectedVulkanIrisFinalAndShadowPassesCannotOpenJavaRenderPasses() throws Exception {
		String finalPass = Files.readString(Path.of("src/main/java/net/irisshaders/iris/pipeline/FinalPassRenderer.java"));
		String shadow = Files.readString(Path.of("src/main/java/net/irisshaders/iris/shadows/ShadowRenderer.java"));
		assertTrue(finalPass.contains("isVulkanBackendSelected()")
			&& finalPass.contains("Java Iris Vulkan final-pass rendering is unavailable until the Rust whole-frame route is admitted"),
			"selected Vulkan Iris final pass must fail before Java rendering");
		assertTrue(shadow.contains("isVulkanBackendSelected()")
			&& shadow.contains("Java Iris shadow renderer entrypoint is unavailable on selected Vulkan"),
			"selected Vulkan Iris shadow pass must fail before Java rendering");
	}

	@Test
	void rustWholeFrameItemSubmissionDoesNotPublishIrisCapturedState() throws Exception {
		String itemState = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/item/ItemStackRenderState.java"));
		int setup = itemState.indexOf("boolean captureIrisRenderState =");
		int restore = itemState.indexOf("CapturedRenderingState.INSTANCE.setCurrentBlockEntity(lastBState)", setup);
		int helper = itemState.indexOf("private void iris$setupId", setup);
		assertTrue(setup >= 0 && restore > setup && helper > restore);
		String submission = itemState.substring(setup, helper);
		assertTrue(submission.contains("!net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()"),
			"Rust item semantics must disable Iris captured-state save/restore");
		assertTrue(submission.contains("if (captureIrisRenderState)"),
			"Iris item identity publication must remain compatibility-only");
		String helperSource = itemState.substring(helper);
		assertTrue(helperSource.contains("net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()")
			&& helperSource.contains("net.vulkanic.VulkanicAPI.isVulkanBackendSelected()"),
			"the Iris item identity helper must fail closed under Rust presentation");
	}

	@Test
	void rustWholeFrameEntityItemLayersDoNotPublishIrisCapturedState() throws Exception {
		for (String file : List.of(
			"CapeLayer.java", "EquipmentLayerRenderer.java", "SimpleEquipmentLayer.java", "WingsLayer.java")) {
			String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/entity/layers", file));
			assertTrue(source.contains("EntityRenderDispatcher.isSemanticSubmission()"),
				file + " must distinguish semantic collection from compatibility rendering");
			assertTrue(source.contains("RustGalVulkanWholeFrameMode.enabled()"),
				file + " must suppress Iris item identity publication on the Rust route");
		}
	}

	@Test
	void rustWholeFrameDispatcherHandoffCannotCaptureIrisStateBeforeBackendSelection() throws Exception {
		String entity = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/entity/EntityRenderDispatcher.java"));
		String blockEntity = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/blockentity/BlockEntityRenderDispatcher.java"));
		for (String source : List.of(entity, blockEntity)) {
			int selected = source.indexOf("boolean selectedVulkan =");
			int rust = source.indexOf("boolean rustWholeFrame =", selected);
			int capture = source.indexOf("!selectedVulkan && !rustWholeFrame", rust);
			assertTrue(selected >= 0 && rust > selected && capture > rust,
				"dispatcher must derive Iris-capture suppression before backend selection settles");
			assertTrue(source.substring(rust, capture).contains("RustGalVulkanWholeFrameMode.enabled()"),
				"whole-frame handoff must suppress Iris capture even before selected-Vulkan becomes observable");
		}
	}

	@Test
	void rustWholeFrameModelStorageShimsAreInert() throws Exception {
		String storage = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/SubmitNodeStorage.java"));
		assertTrue(storage.contains("private static boolean rustWholeFrame()"),
			"semantic storage must expose one explicit Rust ownership predicate");
		assertEquals(10, occurrences(storage, "if (SubmitNodeStorage.rustWholeFrame()) return;"),
			"every compatibility ModelStorage capture/set hook must fail closed for Rust");
	}

	@Test
	void rustWholeFrameBlockLayerLookupLazilyLoadsIrisMapping() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/ItemBlockRenderTypes.java"));
		assertTrue(source.contains("private static final class IrisLayerSet"),
			"Iris block-layer mapping must be isolated behind a lazy compatibility holder");
		assertTrue(source.contains("private static final ChunkSectionLayer[] VALUE = createIrisLayerSet()"),
			"the compatibility mapping must initialize only when its holder is touched");
		assertFalse(source.contains("static {\n\t\tLAYER_SET_VANILLA"),
			"Rust terrain classification must not eagerly initialize Iris block material mappings");
		int rustGuard = source.indexOf("RustGalVulkanWholeFrameMode.enabled()");
		int holderUse = source.indexOf("IrisLayerSet.VALUE", rustGuard);
		assertTrue(rustGuard >= 0 && holderUse > rustGuard
			&& source.contains("!net.vulkanic.VulkanicAPI.isVulkanBackendSelected()"),
			"Iris block-layer mapping must remain behind the Rust whole-frame guard");
	}

	@Test
	void rustWholeFrameRejectsJavaBufferAcquisitionBeforePresentationActivation() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/MultiBufferSource.java"));
		int method = source.indexOf("public VertexConsumer getBuffer(RenderType renderType)");
		int guard = source.indexOf("RustGalVulkanWholeFrameMode.enabled()", method);
		int builder = source.indexOf("new BufferBuilder", method);
		assertTrue(method >= 0 && guard > method && builder > guard,
			"Java buffer construction must be fenced before Rust presentation activation completes");
		assertTrue(source.contains("Java Vulkan buffer-source rendering is unavailable on selected Vulkan")
			|| source.contains("Java Vulkan buffer-source rendering is unavailable while Rust owns whole-frame presentation"));
	}

	@Test
	void abortedWholeFrameCancelsOutstandingSchedulerWork() throws Exception {
		String coordinator = Files.readString(Path.of("src/main/java/net/vulkanic/gui/RustGalFrameCoordinator.java"));
		assertTrue(coordinator.contains("frameCancelled = false"));
		assertTrue(coordinator.contains("SCHEDULER.cancelFrame(frameId, \"frame-aborted\")"),
			"submit/present failures must cancel the acquired frame's queued semantic work");
		assertTrue(coordinator.contains("!executeCounted && frameId != 0L && !frameCancelled"),
			"aborted-frame cleanup must not double-cancel an acquire-skipped frame");
		assertTrue(coordinator.contains("bridge.cancelFrame(frameId, correlationId)"),
			"aborted frames must release the native acquired presentation image, not only scheduler work");
		String frameFfi = readRustFfiModules();
		assertTrue(frameFfi.contains("mattmc_vulkanic_gal_frame_cancel"),
			"the native frame cancellation ABI must be present");
		String frameSource = Files.readString(Path.of("src/main/rust/render/vulkanic/ffi/frame.rs"));
		String cancel = frameSource.substring(frameSource.indexOf("fn mattmc_vulkanic_gal_frame_cancel("),
			frameSource.indexOf("fn mattmc_vulkanic_gal_frame_shutdown("));
		int discard = cancel.indexOf("discard_prepared_post_effects(&mut context.gal)");
		assertTrue(discard >= 0 && discard < cancel.indexOf("destroy_all_frame_targets(context)"),
			"cancelled post-effect uploads and image states must be discarded before target retirement");
	}

	private static int occurrences(String source, String needle) {
		int count = 0;
		for (int offset = 0; (offset = source.indexOf(needle, offset)) >= 0; offset += needle.length()) count++;
		return count;
	}

	@Test
	void lateDebugFrameGraphPassCannotReopenJavaVulkanAfterRustPresentation() throws Exception {
		String levelRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/LevelRenderer.java"));
		assertTrue(levelRenderer.contains("Java late-debug pass is unavailable while Rust owns presentation"),
			"late debug Java draws must fail closed if the Rust shell is already presenting");
		assertTrue(levelRenderer.contains("Java main world pass is unavailable while Rust owns presentation"),
			"main world Java draws must fail closed if the Rust shell is already presenting");
		assertTrue(levelRenderer.contains("Java LevelRenderer.renderLevel is unavailable while Rust owns whole-frame Vulkan"),
			"legacy LevelRenderer entry must not become a hidden Java Vulkan presenter");
		int renderLevel = levelRenderer.indexOf("public void renderLevel(");
		int selectedGuard = levelRenderer.indexOf("RustGalVulkanWholeFrameMode.enabled()", renderLevel);
		int irisSetup = levelRenderer.indexOf("DHCompat.checkFrame()", renderLevel);
		assertTrue(renderLevel >= 0 && selectedGuard > renderLevel && irisSetup > selectedGuard,
			"selected Vulkan must reject LevelRenderer before any Iris frame state is touched");
		String lightTexture = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/LightTexture.java"));
		assertTrue(lightTexture.contains("RustGalVulkanWholeFrameMode.enabled()")
			&& lightTexture.contains("use semantic lightmap inputs"),
			"selected Vulkan lightmap updates must stay on semantic inputs instead of Java GPU/Iris state");
	}

	@Test
	void guiBlurBoundaryTravelsAsSemanticWholeFrameDataAndNeverRunsJavaPostProcess() throws Exception {
		String bridge = Files.readString(Path.of("src/main/java/net/vulkanic/bridge/VulkanicGalBridge.java"));
		String coordinator = Files.readString(Path.of("src/main/java/net/vulkanic/gui/RustGalFrameCoordinator.java"));
		assertTrue(bridge.contains("guiBlurBeforeStratum"));
		assertTrue(bridge.contains("Struct.WHOLE_FRAME_SUBMIT.setInt(request, 31, guiBlurBeforeStratum)"));
		assertTrue(bridge.contains("Struct.WHOLE_FRAME_SUBMIT.setInt(request, 32, guiBlurRadius)"));
		assertTrue(coordinator.contains("renderState.blurBeforeStratumIndex()"));
		assertTrue(coordinator.contains("guiBlurBeforeStratum"));
		assertTrue(coordinator.contains("getMenuBackgroundBlurriness()"));
		String gameRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
		assertTrue(gameRenderer.contains("blur boundary is semantic frame data"));
		assertFalse(gameRenderer.contains("gui-blur-post-process"));
	}

	@Test
	void fabulousTransparencyCannotSilentlyCollapseIntoTheSingleTargetWholeFrameGraph() throws Exception {
		String gameRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
		assertTrue(gameRenderer.contains("Minecraft.useShaderTransparency()"));
		assertTrue(gameRenderer.contains("? \"minecraft:transparency\" : null"),
			"Fabulous transparency must cross the whole-frame boundary as an explicit Rust post-effect identity");
		assertTrue(gameRenderer.contains("six distinct") && gameRenderer.contains("Rust-owned external"));
		assertTrue(gameRenderer.contains("Fabulous transparency is unavailable"));
		int shellStart = gameRenderer.indexOf("renderRustVulkanWholeFrameShell");
		int transparencyGate = gameRenderer.indexOf("Fabulous transparency is unavailable", shellStart);
		int profilerSetup = gameRenderer.indexOf("ProfilerFiller profilerFiller", shellStart);
		int indexedMeshEnqueue = gameRenderer.indexOf("enqueueRustGalIndexedMeshFeaturesForWholeFrame", shellStart);
		int guiExtraction = gameRenderer.indexOf("semantic-gui-extraction", shellStart);
		assertTrue(shellStart >= 0 && transparencyGate > indexedMeshEnqueue && guiExtraction > transparencyGate
			&& profilerSetup > shellStart,
			"the Fabulous gate must inspect copied semantic work before GUI submission, while allowing opaque-only extraction");
	}

	@Test
	void legacyGameRendererEntryPointCannotBecomeASecondVulkanPresenter() throws Exception {
		String gameRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
		int renderStart = gameRenderer.indexOf("public void render(DeltaTracker deltaTracker, boolean bl)");
		int guard = gameRenderer.indexOf("Java GameRenderer.render is unavailable while Rust Vulkan owns the whole frame", renderStart);
		int legacyWorld = gameRenderer.indexOf("this.renderLevel(deltaTracker)", renderStart);
		assertTrue(renderStart >= 0 && guard > renderStart,
			"the legacy renderer must fail closed when Rust owns Vulkan presentation");
		assertTrue(legacyWorld < 0 || guard < legacyWorld,
			"the Java renderer must not reach world rendering before the ownership guard");
	}

	@Test
	void animatedEnergySwirlUsesRustPresenterOwnershipDuringHandoff() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/SubmitNodeCollection.java"));
		int animated = source.indexOf("submitAnimatedModelSemanticTexture(");
		int energy = source.indexOf("enqueueEnergySwirlModel(", animated);
		int unavailable = source.indexOf("Rust whole-frame animated model route has no semantic UV-animation mesh", energy);
		assertTrue(animated >= 0 && energy > animated && unavailable > energy);
		String body = source.substring(animated, unavailable);
		assertTrue(body.contains("rustWholeFramePresenterActive()"),
			"powered creeper/wither swirl admission must honor the Rust presenter shell during backend handoff");
		assertTrue(body.contains("WorldRenderRoutePolicy.currentModelMeshRoute(true).usesRustWholeFrameVulkan()"),
			"animated energy-swirl work must remain behind the explicit Rust model-mesh route");
	}

	@Test
	void directTextureModelFamiliesHonorRustPresenterHandoffOwnership() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/SubmitNodeCollection.java"));
		int start = source.indexOf("public <S> void submitModelSemanticTexture(");
		int end = source.indexOf("public <S> void submitAnimatedModelSemanticTexture(", start);
		String body = source.substring(start, end);
		String[] families = {"SkullModelBase", "TridentModel", "SkeletonModel", "DrownedModel", "WitherBossModel"};
		for (String family : families) {
			assertTrue(body.contains("model instanceof net.minecraft.client.model." + family),
				"missing direct-texture model family: " + family);
		}
		assertTrue(body.contains("object instanceof net.minecraft.client.model.SkullModelBase.State")
			&& body.contains("block_entity/skull"),
			"skull direct-texture admission must use SkullModelBase.State, the state actually emitted by SkullBlockRenderer");
		assertTrue(body.contains("rustWholeFramePresenterActive()"),
			"direct-texture model admissions must use the shared Rust presenter ownership predicate");
	}

	@Test
	void modelPartRustRouteFailsClosedForShellOwnedUnavailableParts() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/SubmitNodeCollection.java"));
		int start = source.indexOf("private void submitModelPartInternal(");
		int end = source.indexOf("\n\t@Override\n\tpublic boolean submitEndPortal", start);
		String body = source.substring(start, end);
		assertTrue(body.contains("currentModelPartMeshRoute(rustEligible)"));
		assertTrue(body.contains("rustRoute == net.vulkanic.world.WorldRenderRoutePolicy.Route.DISABLED\n\t\t\t\t&& rustWholeFramePresenterActive()"),
			"an unavailable ModelPart must not reopen Java geometry while the Rust shell owns the frame");
		assertTrue(body.contains("enqueueModelPartMesh("),
			"eligible ModelPart submissions must retain their explicit Rust semantic producer");
	}

	@Test
	void blockAndMovingModelRoutesHonorRustPresenterShellOwnership() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/SubmitNodeCollection.java"));
		int start = source.indexOf("public void submitBlock(");
		int end = source.indexOf("\n\t@Override\n\tpublic void submitItem(", start);
		String body = source.substring(start, end);
		assertTrue(body.contains("rustWholeFramePresenterActive()")
			&& body.contains("currentBlockDisplayRoute()")
			&& body.contains("currentPrimedTntRoute()")
			&& body.contains("currentFallingBlockRoute()")
			&& body.contains("currentPistonMovingBlockRoute()"),
			"block-display, TNT, and moving-block submissions must use the shared Rust ownership predicate");
		assertTrue(body.contains("enqueueBlockModelMesh("),
			"block-model submissions must retain their explicit Rust semantic producer");
	}

	@Test
	void itemMeshRouteFailsClosedForShellOwnedUnavailableItems() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/SubmitNodeCollection.java"));
		int start = source.indexOf("private void submitItemInternal(");
		int end = source.indexOf("\n\t@Override\n\tpublic void submitCustomGeometry(", start);
		String body = source.substring(start, end);
		assertTrue(body.contains("currentItemEntityMeshRoute(rustEligible)"));
		assertTrue(body.contains("rustRoute == net.vulkanic.world.WorldRenderRoutePolicy.Route.DISABLED\n\t\t\t\t\t&& rustWholeFramePresenterActive()"),
			"an unavailable item mesh must not reopen Java item rendering while the Rust shell owns the frame");
		assertTrue(body.contains("enqueueItemEntityMesh("),
			"eligible item submissions must retain the explicit Rust indexed-mesh producer");
	}

	@Test
	void particleCallbacksCannotReopenJavaWhenRustShellOwnsPresentation() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/SubmitNodeCollection.java"));
		int start = source.indexOf("public void submitParticleGroup(");
		int end = source.indexOf("\n\t@Override\n\tpublic void submitParticleGroupSemantic", start);
		String body = source.substring(start, end);
		assertTrue(body.contains("currentMaterialRoute().usesRustWholeFrameVulkan()"));
		assertTrue(body.contains("if (rustWholeFramePresenterActive())"),
			"unsupported particle callbacks must fail closed under Rust shell ownership");
		assertTrue(body.contains("Java particle callbacks are not a fallback"));
	}

	@Test
	void worldBorderCannotFallBackToJavaVulkanAfterRustSemanticRejection() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/WorldBorderRenderer.java"));
		String levelRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/LevelRenderer.java"));
		String routePolicy = Files.readString(Path.of("src/main/java/net/vulkanic/world/WorldRenderRoutePolicy.java"));
		int enqueue = source.indexOf("RustGalWorldPrimitiveRenderer.enqueueWorldBorder");
		int ownership = source.indexOf("RustGalVulkanWholeFrameMode.enabled()", enqueue);
		int javaPass = source.indexOf("VulkanicAPI.createRenderPass", ownership);
		assertTrue(enqueue >= 0 && ownership > enqueue && javaPass > ownership,
			"world-border Java rendering must be behind an explicit non-Rust ownership gate");
		assertTrue(source.contains("Java Vulkan fallback is unavailable"),
			"an admitted Rust frame must fail closed when world-border semantic admission fails");
		assertTrue(source.contains("Java world-border buffer rebuild is unavailable while Rust owns whole-frame presentation"),
			"legacy world-border buffer rebuilds must fail closed instead of dereferencing the Rust-null buffer");
		assertTrue(source.contains("if (VulkanicAPI.isVulkanBackendSelected()\n\t\t\t|| net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled())"),
			"the final world-border Java fallback guard must honor Rust whole-frame shell ownership");
		assertTrue(source.contains("boolean rustWholeFrame = net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled();"),
			"world-border rejection must remain active for the Rust whole-frame shell");
		assertTrue(levelRenderer.contains("boolean accepted = net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueWorldBorder"),
			"the whole-frame world-border caller must retain the semantic admission result");
		assertTrue(levelRenderer.contains("Rust whole-frame world-border route rejected visible semantic work"),
			"visible rejected world-border work must not be silently omitted");
		for (String method : new String[] {"currentWorldBorderRoute()", "currentBackgroundRoute()"}) {
			int route = routePolicy.indexOf("public static Route " + method);
			assertTrue(route >= 0, "missing route policy " + method);
			String body = routePolicy.substring(route, routePolicy.indexOf("\n\t}\n", route));
			assertTrue(body.contains("RustGalVulkanWholeFrameMode.enabled()"),
				method + " must remain Rust-owned during the pre-selection whole-frame handoff");
		}
	}

	@Test
	void wholeFrameFingerprintSeparatesParticleMaterialWorkFromGenericMaterialWork() throws Exception {
		String coordinator = Files.readString(Path.of("src/main/java/net/vulkanic/gui/RustGalFrameCoordinator.java"));
		assertTrue(coordinator.contains("long particleQuads = frame.materialQuads().stream()"));
		assertTrue(coordinator.contains("RustGalWorldPrimitiveRenderer.MATERIAL_SOURCE_PARTICLES"));
		assertTrue(coordinator.contains("+ \" particle_quads=\" + particleQuads"));
	}

	@Test
	void malformedBackendKindIsRejectedDeterministically() {
		try (Arena arena = Arena.ofConfined()) {
			MemorySegment request = VulkanicGalBridge.Struct.CONTEXT_CREATE.allocate(arena);
			VulkanicGalBridge.Abi.writeHeader(request, VulkanicGalBridge.Struct.CONTEXT_CREATE);
			VulkanicGalBridge.Struct.CONTEXT_CREATE.setInt(request, 1, 9999);
			VulkanicGalBridge.Struct.CONTEXT_CREATE.setInt(request, 2, 0);
			VulkanicGalBridge.Abi.writeBytes(arena, request, VulkanicGalBridge.Struct.CONTEXT_CREATE, 3, "bad-backend");
			MemorySegment result = VulkanicGalBridge.Struct.CONTEXT_RESULT.allocate(arena);

			int status = VulkanicGalBridge.Native.contextCreate(request, result);

			assertEquals(-5, status);
			assertEquals(-5, VulkanicGalBridge.Struct.CONTEXT_RESULT.getInt(result, 1));
		}
	}

	@Test
	void rustGalGuiRoutePolicySeparatesOpenGlJavaVulkanAndWholeFrameVulkan() throws Exception {
		assertEquals(
			RustGalGuiRenderer.GuiExecutionRoute.RUST_OPENGL_BORROWED_CONTEXT,
			RustGalGuiRenderer.selectExecutionRouteForTests(false, false, false, false)
		);
		assertEquals(
			RustGalGuiRenderer.GuiExecutionRoute.RUST_VULKAN_WHOLE_FRAME,
			RustGalGuiRenderer.selectExecutionRouteForTests(false, true, false, false),
			"GUI must select the Rust Vulkan route during the pre-selection whole-frame handoff"
		);
		assertEquals(
			RustGalGuiRenderer.GuiExecutionRoute.DISABLED,
			RustGalGuiRenderer.selectExecutionRouteForTests(true, false, false, false)
		);
		assertEquals(
			RustGalGuiRenderer.GuiExecutionRoute.RUST_VULKAN_WHOLE_FRAME,
			RustGalGuiRenderer.selectExecutionRouteForTests(true, true, false, false)
		);
		assertEquals(
			RustGalGuiRenderer.GuiExecutionRoute.JAVA_COMPATIBILITY,
			RustGalGuiRenderer.selectExecutionRouteForTests(false, false, false, true)
		);
		assertEquals(
			RustGalGuiRenderer.GuiExecutionRoute.DISABLED,
			RustGalGuiRenderer.selectExecutionRouteForTests(true, true, false, true),
			"GUI legacy controls must fail closed while Rust Vulkan owns the frame"
		);
		assertEquals(
			RustGalGuiRenderer.GuiExecutionRoute.DISABLED,
			RustGalGuiRenderer.selectExecutionRouteForTests(true, true, true, false)
		);
		String guiSource = Files.readString(Path.of("src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"));
		int currentRoute = guiSource.indexOf("public static GuiExecutionRoute currentExecutionRoute()");
		int routeSelection = guiSource.indexOf("selectExecutionRoute(", currentRoute);
		assertTrue(currentRoute >= 0 && routeSelection > currentRoute
			&& guiSource.indexOf("isWholeFrameVulkanEnabled()", routeSelection) > routeSelection,
			"GUI ownership must use whole-frame handoff enablement before presenter activation");
	}

	@Test
	void blockOutlineRouteKeepsJavaPathsOutsideRustVulkanShell() throws Exception {
		assertEquals(
			WorldRenderRoutePolicy.Route.RUST_OPENGL_BORROWED_CONTEXT,
			WorldRenderRoutePolicy.selectRouteForTests(false, false, false, false)
		);
		assertEquals(
			WorldRenderRoutePolicy.Route.RUST_OPENGL_BORROWED_CONTEXT,
			WorldRenderRoutePolicy.selectRouteForTests(false, true, false, false)
		);
		assertEquals(
			WorldRenderRoutePolicy.Route.DISABLED,
			WorldRenderRoutePolicy.selectRouteForTests(true, false, false, false)
		);
		assertEquals(
			WorldRenderRoutePolicy.Route.RUST_VULKAN_WHOLE_FRAME,
			WorldRenderRoutePolicy.selectRouteForTests(true, true, false, false)
		);
		assertEquals(
			WorldRenderRoutePolicy.Route.DISABLED,
			WorldRenderRoutePolicy.selectRouteForTests(false, false, true, false)
		);
		assertTrue(
			Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"))
				.contains("shouldUseRustOpenGlCrack()")
		);
		assertTrue(
			Files.readString(Path.of("src/main/java/net/vulkanic/world/WorldRenderRoutePolicy.java"))
				.contains("currentBlockDisplayRoute()")
		);
		assertTrue(
			Files.readString(Path.of("src/main/java/net/vulkanic/world/WorldRenderRoutePolicy.java"))
				.contains("currentFallingBlockRoute()")
		);
		String shaderAffectedRoutePolicy = Files.readString(Path.of("src/main/java/net/vulkanic/world/WorldRenderRoutePolicy.java"));
		for (String method : new String[] {
			"currentBlockOutlineRoute()", "currentCrackRoute()",
			"currentBlockDisplayRoute()", "currentFallingBlockRoute()",
			"currentPistonMovingBlockRoute()", "currentPrimedTntRoute()",
			"currentStaticTerrainRoute()", "currentDistantHorizonsOpaqueRoute()",
			"currentItemEntityOwnershipRoute()", "currentArrowOwnershipRoute()",
			"currentModelMeshRoute(boolean eligible)", "currentModelPartMeshRoute(boolean eligible)",
			"currentEntityFlameRoute()", "currentEntityShadowRoute()", "currentEntityLeashRoute()"
		}) {
			int route = shaderAffectedRoutePolicy.indexOf("public static Route " + method);
			assertTrue(route >= 0, "missing route policy " + method);
			String body = shaderAffectedRoutePolicy.substring(route, shaderAffectedRoutePolicy.indexOf("\n\t}\n", route));
			assertTrue(body.contains("RustGalVulkanWholeFrameMode.enabled()"),
				method + " must remain Rust-owned during the pre-selection whole-frame handoff");
		}
		assertTrue(shaderAffectedRoutePolicy.contains("BooleanSupplier irisPackActive"),
			"shader-affected routing must keep Iris state behind a lazy supplier boundary");
		assertTrue(shaderAffectedRoutePolicy.contains("net.irisshaders.iris.Iris::isPackInUseQuick"),
			"borrowed OpenGL may query Iris only through the lazy compatibility supplier");
		assertTrue(shaderAffectedRoutePolicy.contains("if (!selected.usesRustOpenGl())"),
			"both Vulkan routes must return before consulting Iris runtime state");
		assertTrue(shaderAffectedRoutePolicy.contains("irisPackActive.getAsBoolean() ? Route.JAVA_COMPATIBILITY : selected"),
			"only the borrowed OpenGL route may resolve Iris pack activity");
		assertFalse(shaderAffectedRoutePolicy.contains("if (selected.usesRustOpenGl() && irisPackActive)"),
			"shader-aware routing must not restore the old eagerly-evaluated Iris boolean path");
		assertTrue(
			Files.readString(Path.of("src/main/java/net/vulkanic/world/WorldRenderRoutePolicy.java"))
				.contains("currentPistonMovingBlockRoute()")
		);
		String routeEnum = Files.readString(Path.of("src/main/java/net/vulkanic/world/WorldRenderRoutePolicy.java"));
		int javaCompatibility = routeEnum.indexOf("public boolean usesJavaCompatibility()");
		assertTrue(javaCompatibility >= 0);
		String javaCompatibilityBody = routeEnum.substring(javaCompatibility, routeEnum.indexOf("\n\t\t}", javaCompatibility));
		assertTrue(javaCompatibilityBody.contains("!RustGalVulkanWholeFrameMode.enabled()"),
			"Java compatibility must be unavailable during the pre-selection Rust whole-frame handoff");
		String encoder = Files.readString(Path.of("src/main/java/net/vulkanic/backends/vulkan/VulkanNativeCommandEncoder.java"));
		assertTrue(encoder.contains("private static boolean legacyImmediatePassIgnored()"),
			"the Java Vulkan encoder must isolate Iris ImmediateState behind a legacy-only helper");
		assertTrue(encoder.contains("if (RustGalVulkanWholeFrameMode.enabled()\n            || VulkanicAPI.isVulkanBackendSelected()) {\n            return null;\n        }\n        int samplerObject = IrisRenderSystem.getBoundSamplerOnUnit(samplerUnit);"),
			"selected Vulkan and Rust whole-frame mode must not recover Iris sampler-object state");
		int samplerHelper = encoder.indexOf("private static Integer currentBoundSamplerObject");
		assertTrue(samplerHelper >= 0
			&& encoder.indexOf("VulkanicAPI.isVulkanBackendSelected()", samplerHelper) >= 0,
			"selected Vulkan must keep the encoder's sampler-object recovery helper Iris-free");
		assertTrue(encoder.contains("&& !VulkanicAPI.isVulkanBackendSelected()\n            && net.irisshaders.iris.vertices.ImmediateState.temporarilyIgnorePass"),
			"selected Vulkan must not inspect Iris ImmediateState from the Java encoder");
		String weatherRoutePolicy = Files.readString(Path.of("src/main/java/net/vulkanic/world/WorldRenderRoutePolicy.java"));
		assertTrue(weatherRoutePolicy.contains("currentWeatherRoute()"));
		for (String method : new String[] {"currentWeatherRoute()", "currentCloudRoute()", "currentMaterialRoute()", "currentWorldTextRoute()"}) {
			int route = weatherRoutePolicy.indexOf("public static Route " + method);
			assertTrue(route >= 0, "missing route policy " + method);
			String body = weatherRoutePolicy.substring(route, weatherRoutePolicy.indexOf("\n\t}\n", route));
			assertTrue(body.contains("RustGalVulkanWholeFrameMode.enabled()"),
				method + " must remain Rust-owned during the pre-selection whole-frame handoff");
		}
		assertFalse(
			weatherRoutePolicy.contains("mattmc.dev.rustGalWeather.v1"),
			"weather must follow whole-frame ownership without a separate opt-in"
		);
		assertTrue(
			weatherRoutePolicy.contains("return selectShaderAffectedRoute(VulkanicAPI.isVulkanBackendSelected(), rustWholeFrameShellActive());"),
			"weather must use the shared shader-aware policy once the bounded Rust weather feature is selected"
		);
		String weatherRenderer = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		String vanillaWeatherRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/WeatherEffectRenderer.java"));
		assertTrue(weatherRenderer.contains("enqueueWorldWeather(WeatherRenderState state, Vec3 cameraPos, boolean depthWrite)"));
		assertTrue(weatherRenderer.contains("refreshBorrowedOpenGlFrameSeed"));
		assertTrue(weatherRenderer.contains("route=rust-vulkan-whole-frame"));
		assertTrue(weatherRenderer.contains("Rust VulkanicGAL weather submission failed after Rust route selection"));
		assertTrue(weatherRenderer.contains("Rust whole-frame weather route requires a seeded semantic viewport (bounded)"),
			"weather semantic admission must reject oversized viewports before queueing material quads");
		assertTrue(vanillaWeatherRenderer.contains("Rust whole-frame weather route is unavailable while Rust owns presentation"),
			"disabled weather must not reopen Java rendering after Rust presentation begins");
		int rainParticles = vanillaWeatherRenderer.indexOf("public void tickRainParticles(");
		int rainParticlePolicy = vanillaWeatherRenderer.indexOf("shouldRenderWeatherParticles", rainParticles);
		assertTrue(rainParticles >= 0 && rainParticlePolicy > rainParticles
			&& vanillaWeatherRenderer.substring(rainParticles, rainParticlePolicy).contains("RustGalVulkanWholeFrameMode.enabled()"),
			"Rust whole-frame weather setup must not consult Iris when the optional Rust weather route is unavailable");
		assertFalse(weatherRenderer.contains("return RustGalFrameCoordinator.executeWorldPrimitiveFrame(minecraft, frame, \"minecraft.world.weather\")"));
		String cloudRoutePolicy = Files.readString(Path.of("src/main/java/net/vulkanic/world/WorldRenderRoutePolicy.java"));
		assertTrue(cloudRoutePolicy.contains("currentCloudRoute()"));
		assertFalse(cloudRoutePolicy.contains("mattmc.dev.rustGalClouds.v1"));
		assertTrue(weatherRoutePolicy.contains("currentDistantHorizonsOpaqueRoute()"));
		assertFalse(
			weatherRoutePolicy.contains("mattmc.dev.rustGalDistantHorizons.opaqueV1"),
			"DH must rely on its real render-list admission, not an opt-in flag before preflight"
		);
		assertTrue(
			Files.readString(Path.of("src/main/java/com/seibel/distanthorizons/core/render/renderer/LodRenderer.java"))
				.contains("buffers.buildRenderList(renderParams)"),
			"DH route selection must inspect the real render list before Rust ownership"
		);
		assertTrue(
			Files.readString(Path.of("src/main/java/com/seibel/distanthorizons/core/render/renderer/LodRenderer.java"))
				.contains("markRustNonWaterRouteSelected()"),
			"DH must select Rust only after visible material streams pass the explicit semantic admission"
		);
		String cloudRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/CloudRenderer.java"));
		assertTrue(cloudRenderer.contains("enqueueRustGalClouds"));
		assertTrue(cloudRenderer.contains("this.texture.cells()"));
		String cloudWorldRenderer = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		assertTrue(cloudWorldRenderer.contains("Rust VulkanicGAL cloud route requires a seeded bounded world primitive frame"),
			"cloud semantic admission must reject oversized viewports before queueing cloud faces");
		assertFalse(cloudRenderer.contains("enqueueWorldCloudFaces(\n\t\t\tthis.texture,"));
		String levelRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/LevelRenderer.java"));
		assertTrue(levelRenderer.contains("currentCloudRoute().usesRustWholeFrameVulkan()"));
		assertTrue(levelRenderer.contains("Rust whole-frame cloud route is unavailable while Rust owns presentation"),
			"disabled clouds must not reopen Java cloud rendering after Rust presentation begins");
		assertEquals(
			WorldRenderRoutePolicy.Route.JAVA_COMPATIBILITY,
			WorldRenderRoutePolicy.selectWholeFrameRouteForTests(false, false, false, false),
			"static terrain must stay compatibility-owned unless Rust Vulkan whole-frame is active"
		);
		assertEquals(
			WorldRenderRoutePolicy.Route.RUST_VULKAN_WHOLE_FRAME,
			WorldRenderRoutePolicy.selectWholeFrameRouteForTests(true, true, false, false)
		);
		String worldRoutePolicy = Files.readString(Path.of("src/main/java/net/vulkanic/world/WorldRenderRoutePolicy.java"));
		assertTrue(worldRoutePolicy.contains("currentStaticTerrainRoute()"));
		assertTrue(worldRoutePolicy.contains("mattmc.dev.rustGalStaticTerrain.disabled"));
		assertTrue(worldRoutePolicy.contains("staticTerrainBuildRequiresRustWholeFrameMetadata()"));
		assertTrue(
			Files.readString(Path.of("src/main/java/net/sodium/client/render/chunk/compile/tasks/ChunkBuilderMeshingTask.java"))
				.contains("WorldRenderRoutePolicy.staticTerrainBuildRequiresRustWholeFrameMetadata()"),
			"chunk builds must prepare Rust terrain metadata before backend-selected state is stable"
		);
	}

	@Test
	void shaderAffectedWeatherRouteKeepsIrisAndNormalJavaVulkanCompatibilityOwned() {
		assertEquals(
			WorldRenderRoutePolicy.Route.RUST_VULKAN_WHOLE_FRAME,
			WorldRenderRoutePolicy.selectShaderAffectedRouteForTests(true, true, true, false, false),
			"a selected Rust Vulkan whole-frame route owns weather even with a shader pack configured"
		);
		assertEquals(
			WorldRenderRoutePolicy.Route.DISABLED,
			WorldRenderRoutePolicy.selectShaderAffectedRouteForTests(true, false, false, false, false),
			"unadmitted Vulkan must remain unavailable rather than reopening Java rendering"
		);
		assertEquals(
			WorldRenderRoutePolicy.Route.JAVA_COMPATIBILITY,
			WorldRenderRoutePolicy.selectShaderAffectedRouteForTests(false, false, true, false, false),
			"Iris OpenGL must remain Java-compatible until Rust owns the complete shader frame"
		);
		assertEquals(
			WorldRenderRoutePolicy.Route.RUST_OPENGL_BORROWED_CONTEXT,
			WorldRenderRoutePolicy.selectShaderAffectedRouteForTests(false, false, false, false, false),
			"non-Iris OpenGL may select Rust's explicit borrowed-context route"
		);
		assertEquals(
			WorldRenderRoutePolicy.Route.DISABLED,
			WorldRenderRoutePolicy.selectShaderAffectedRouteForTests(true, true, false, true, false)
		);
		assertEquals(
			WorldRenderRoutePolicy.Route.DISABLED,
			WorldRenderRoutePolicy.selectShaderAffectedRouteForTests(true, true, false, false, true)
			, "legacy route controls must fail closed while Rust Vulkan owns the whole frame"
		);
		assertEquals(
			WorldRenderRoutePolicy.Route.DISABLED,
			WorldRenderRoutePolicy.selectRouteForTests(true, true, false, true),
			"generic legacy route controls must not reopen Java Vulkan rendering"
		);
		assertEquals(
			WorldRenderRoutePolicy.Route.DISABLED,
			WorldRenderRoutePolicy.selectWholeFrameRouteForTests(true, true, false, true),
			"whole-frame legacy route controls must remain unavailable under Rust Vulkan"
		);
	}

	@Test
	void arrowRouteIsWholeFrameOnlyAndStaysExplicitOutsideItsBoundedEligibility() throws Exception {
		assertEquals(
			WorldRenderRoutePolicy.Route.JAVA_COMPATIBILITY,
			WorldRenderRoutePolicy.currentArrowRoute(false),
			"per-Arrow admission remains unavailable when the current state is unsupported"
		);
		String routePolicy = Files.readString(Path.of("src/main/java/net/vulkanic/world/WorldRenderRoutePolicy.java"));
		String arrowRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/entity/ArrowRenderer.java"));
		String worldRenderer = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		assertTrue(routePolicy.contains("currentArrowOwnershipRoute()"),
			"Arrow ownership must be independent of per-state admission");
		assertTrue(routePolicy.contains("currentArrowRoute(boolean eligible)"));
		assertTrue(routePolicy.contains("return selectWholeFrameRoute(VulkanicAPI.isVulkanBackendSelected(), rustWholeFrameShellActive());"));
		assertTrue(arrowRenderer.contains("WorldRenderRoutePolicy.currentArrowOwnershipRoute()"));
		assertTrue(arrowRenderer.contains("ArrowSubmitDisposition.RUST_UNAVAILABLE"));
		assertTrue(arrowRenderer.contains("\"rust-vulkan-unavailable\""));
		assertTrue(arrowRenderer.contains("enqueueArrowModel("));
		assertTrue(arrowRenderer.contains("isSemanticCoverageOnly()"));
		assertFalse(arrowRenderer.contains("Rust whole-frame Arrow encountered unsupported semantic state before route selection"),
			"unsupported Rust-owned Arrow state must be explicit unavailable work, not crash-as-routing-control-flow");
		int arrowOwnership = arrowRenderer.indexOf("WorldRenderRoutePolicy.currentArrowOwnershipRoute()");
		int arrowUnavailable = arrowRenderer.indexOf("ArrowSubmitDisposition.RUST_UNAVAILABLE", arrowOwnership);
		int arrowSemanticSubmit = arrowRenderer.indexOf("submitNodeCollector.submitModelSemantic(", arrowUnavailable);
		assertTrue(arrowOwnership >= 0 && arrowUnavailable > arrowOwnership && arrowSemanticSubmit > arrowUnavailable,
			"semantic Arrow submission must remain explicit after Rust-unavailable handling");
		String deterministicCapture = Files.readString(Path.of("src/main/java/net/minecraft/client/dev/DeterministicCameraCapture.java"));
		assertTrue(deterministicCapture.contains("decision.javaDrawn() && !decision.rustSelected() && !decision.rustQueued()"));
		assertTrue(deterministicCapture.contains("decision.rustSelected() && decision.rustQueued() && !decision.javaDrawn()"));
		assertTrue(deterministicCapture.contains("RustGalWholeFrameTerrainSource.isWholeFrameTerrainQueueDrained()"),
			"settled capture readiness must use the Rust whole-frame terrain queue when Java Sodium identities are absent");
		assertTrue(deterministicCapture.contains("server.execute(() -> applySourceEntityIsolationOnServer(server, dimension))"));
		assertTrue(deterministicCapture.contains("private static void applySourceEntityIsolationOnServer"));
		int captureAfterRender = deterministicCapture.indexOf("public static void afterRender(Minecraft minecraft)");
		int captureIsolationAdvance = deterministicCapture.indexOf("prepareSourceEntityIsolationBeforeFrame(minecraft);", captureAfterRender);
		int captureSettledGate = deterministicCapture.indexOf("settledReadyGateSatisfied(minecraft)", captureAfterRender);
		assertTrue(captureIsolationAdvance >= 0 && captureIsolationAdvance < captureSettledGate,
			"whole-frame captures must advance copied-world source isolation before checking source-plan readiness");
		int captureModelSetup = deterministicCapture.indexOf("setupMovingMeshScenarioAfterSettledReady(minecraft)", captureAfterRender);
		int captureSourceReceiptGate = deterministicCapture.indexOf("selectedSourceCaptureRequested() && !requiredRustSourceExecutionObserved()", captureAfterRender);
		assertTrue(captureModelSetup >= 0 && captureModelSetup < captureSourceReceiptGate,
			"selected-source receipt validation must run after copied-world producer setup so a required model can produce it");
		assertTrue(deterministicCapture.contains("else if (!MODEL_MESH_SCENARIO.isEmpty() && !\"hidden\".equals(MODEL_MESH_SCENARIO))"),
			"static ModelPart scenarios must retain the same bounded capture sequence as animated model fixtures");
		assertTrue(deterministicCapture.contains("selectedSourceCaptureRequested() && isModelMeshEntityScenario()"),
			"selected-source entity captures must retain the exact producer-visible pose instead of accepting unrelated later entity work");
		assertTrue(worldRenderer.contains("extractArrowModelMesh"));
		assertTrue(worldRenderer.contains("Rust VulkanicGAL Arrow requires a seeded bounded world primitive frame"));
		assertTrue(worldRenderer.contains("STRATUM_WORLD_ENTITY_MESH"));
		assertTrue(worldRenderer.contains("applyPackedLight(0xffffffff, packedLight)"));
		assertTrue(worldRenderer.contains("normalPacked, 0, 0"));
		assertTrue(worldRenderer.contains("state.nameTag == null"));
		assertTrue(worldRenderer.contains("state.shadowPieces.isEmpty()"));
		String levelRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/LevelRenderer.java"));
		assertTrue(levelRenderer.contains("isVanillaArrowStateEligible(arrowState)"));
		assertFalse(worldRenderer.contains("TextureAtlasSprite sprite, ResourceLocation textureLocation"));
		String modelSubmitter = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/SubmitNodeCollection.java"));
		String livingEntityRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/entity/LivingEntityRenderer.java"));
		int modelInternal = modelSubmitter.indexOf("private <S> void submitModelInternal(");
		int modelInternalEnd = modelSubmitter.indexOf("\n\t@Override\n\tpublic void submitModelPart(", modelInternal);
		String modelInternalSource = modelSubmitter.substring(modelInternal, modelInternalEnd);
		assertTrue(modelInternalSource.contains("rustWholeFramePresenterActive()")
			&& modelInternalSource.contains("enqueueStandaloneGlintModelMesh(")
			&& modelInternalSource.contains("model instanceof net.minecraft.client.model.BeeStingerModel")
			&& modelInternalSource.contains("model instanceof net.minecraft.client.model.EndermanModel")
			&& modelInternalSource.contains("model instanceof net.minecraft.client.model.PhantomModel")
			&& modelInternalSource.contains("model instanceof net.minecraft.client.model.CreeperModel")
			&& modelInternalSource.contains("model instanceof net.minecraft.client.model.SpiderModel")
			&& modelInternalSource.contains("model instanceof net.minecraft.client.model.GuardianParticleModel"),
			"translucent entity overlay model routes must honor Rust presenter-shell ownership during handoff");
		assertTrue(modelInternalSource.contains("this.modelSubmits.totalSubmitCount() >= MAX_RUST_SEMANTIC_MODEL_SUBMITS"),
			"Rust-owned model submissions must retain their bounded queue admission during handoff");
		assertTrue(routePolicy.contains("currentModelMeshRoute(boolean eligible)"));
		assertFalse(
			routePolicy.contains("mattmc.dev.rustGalWorldModelMesh.v1"),
			"eligible indexed entity models must follow Rust whole-frame ownership without an opt-in"
		);
		assertTrue(worldRenderer.contains("model instanceof ChestModel"));
		assertTrue(worldRenderer.contains("modelMeshIneligibilityReason("));
		assertTrue(levelRenderer.contains("eligibility=\" + (ineligibility"));
		assertTrue(worldRenderer.contains("model instanceof ShulkerBoxRenderer.ShulkerBoxModel"));
		assertTrue(worldRenderer.contains("model instanceof LlamaSpitModel"));
		assertTrue(worldRenderer.contains("model instanceof EvokerFangsModel"));
		assertTrue(worldRenderer.contains("model instanceof SkullModel"));
		assertTrue(worldRenderer.contains("isStandaloneModelMeshEligible("));
		assertTrue(worldRenderer.contains("isStandaloneTextureIdentity(textureIdentity)"),
			"generic standalone models must admit copied resource-pack and mod texture namespaces");
		assertTrue(worldRenderer.contains("!textureIdentity.getNamespace().isBlank()"),
			"standalone texture admission must still reject malformed identities");
		assertTrue(worldRenderer.contains("enqueueStandaloneModelMesh("));
		assertTrue(worldRenderer.contains("readModelTexturePayload(textureIdentity)"));
		assertTrue(worldRenderer.contains("getResourceStack(textureLocation)"),
			"semantic model textures must resolve through the complete resource-pack stack");
		assertTrue(worldRenderer.contains("resources.reversed()"),
			"getResourceStack is lowest-priority-first; semantic texture selection must start at the end");
		assertTrue(worldRenderer.contains("path.startsWith(\"textures/\") && path.endsWith(\".png\")"));
		assertTrue(worldRenderer.contains("neither the RenderType nor its backing GPU"));
		assertTrue(worldRenderer.contains("modelMeshRenderSemantics(renderType)"));
		assertTrue(worldRenderer.contains("BlendFunction.TRANSLUCENT.equals(blend.get())"));
		assertTrue(worldRenderer.contains("MATERIAL_MODE_TRANSLUCENT"));
		assertTrue(worldRenderer.contains("renderType.pipeline().isCull() ? CULL_BACK : CULL_NONE"));
		assertTrue(worldRenderer.contains("extractModelPartMesh("));
		assertTrue(worldRenderer.contains("recordModelMeshRouteDecision("));
		assertTrue(worldRenderer.contains("hasCurrentFrameRustModelMeshDecision(Model<?> model, TextureAtlasSprite sprite)"));
		assertTrue(worldRenderer.contains("hasCurrentFrameRustModelMeshDecision(Model<?> model, ResourceLocation textureIdentity)"));
		assertTrue(worldRenderer.contains("PENDING_MODEL_MESH_SEMANTICS.add(new ModelMeshSemanticIdentity("));
		assertTrue(worldRenderer.contains("PENDING_MODEL_MESH_SEMANTICS.contains(new ModelMeshSemanticIdentity("));
		assertTrue(worldRenderer.contains("isVanillaCowModelMeshEligible("));
		assertTrue(worldRenderer.contains("isVanillaMushroomCowModelMeshEligible("));
		assertTrue(worldRenderer.contains("isLobsterModelMeshEligible("));
		assertTrue(worldRenderer.contains("isBisonModelMeshEligible("));
		assertTrue(worldRenderer.contains("isVanillaZombieModelMeshEligible("));
		assertFalse(worldRenderer.contains("decision.frameIndex() == frameIndex"),
			"coverage replay must use the active semantic request, not a delayed capture-frame counter");
		assertTrue(levelRenderer.contains("isSelectedWholeFrameModelSemantic("),
			"coverage replay may suppress a model only after the real same-frame Rust semantic route selected it");
		assertTrue(levelRenderer.contains("hasCurrentFrameRustModelMeshDecision(model, sprite)"),
			"coverage replay must honor exact same-frame atlas model receipts for every admitted Rust family");
		assertTrue(levelRenderer.contains("model instanceof net.minecraft.client.model.ChestModel && state instanceof Float"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.CowRenderState cowRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.PigRenderState pigRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.CreeperRenderState creeperRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.FoxRenderState foxRenderState"));
		assertTrue(levelRenderer.contains("model instanceof net.minecraft.client.model.SpiderModel"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.ArmorStandRenderState armorStandRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.CatRenderState catRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.WolfRenderState wolfRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.VillagerRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.GuardianRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.SnowGolemRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.IronGolemRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.SkeletonRenderState skeletonRenderState"));
		assertTrue(levelRenderer.contains("textures/entity/wandering_trader.png"));
		assertTrue(levelRenderer.contains("textures/entity/guardian/guardian_elder.png"));
		assertTrue(levelRenderer.contains("textures/entity/skeleton/wither_skeleton.png"));
		assertTrue(levelRenderer.contains("textures/entity/spider/cave_spider.png"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.GhastRenderState ghastRenderState"));
		assertTrue(levelRenderer.contains("model instanceof net.minecraft.client.model.BlazeModel"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.WitchRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.EndermanRenderState"));
		assertTrue(levelRenderer.contains("model instanceof net.minecraft.client.model.EndermiteModel"));
		assertTrue(levelRenderer.contains("model instanceof net.minecraft.client.model.SilverfishModel"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.BatRenderState"));
		assertTrue(levelRenderer.contains("model instanceof net.minecraft.client.model.CodModel"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.SalmonRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.PufferfishRenderState"));
		assertTrue(levelRenderer.contains("model instanceof net.minecraft.client.model.TadpoleModel"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.FelineRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.PolarBearRenderState polarBearRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.DolphinRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.TurtleRenderState turtleRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.PandaRenderState pandaRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.BeeRenderState beeRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.AxolotlRenderState axolotlRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.FrogRenderState frogRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.SquidRenderState squidRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.GoatRenderState goatRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.AllayRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.EvokerRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.IllagerRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.ParrotRenderState parrotRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.TropicalFishRenderState tropicalFishRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.RavagerRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.VexRenderState vexRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.SlimeRenderState slimeRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.PhantomRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.WardenRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.WitherRenderState witherRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.ZombieRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.CreakingRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.BreezeRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.CopperGolemRenderState copperGolemRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.StriderRenderState striderRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.HoglinRenderState hoglinRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.CamelRenderState camelRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.HumanoidRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.SkeletonRenderState strayRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.BoggedRenderState boggedRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.ArmadilloRenderState armadilloRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.SnifferRenderState snifferRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.NautilusRenderState nautilusRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.HorseRenderState horseRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.DonkeyRenderState donkeyRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.LlamaRenderState llamaRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.RabbitRenderState rabbitRenderState"));
		assertTrue(levelRenderer.contains("entityRenderState instanceof net.minecraft.client.renderer.entity.state.ZombieRenderState"));
		assertTrue(levelRenderer.contains("state instanceof net.minecraft.client.renderer.entity.state.ZombieRenderState"));
		assertTrue(livingEntityRenderer.contains("Rust whole-frame living-model route selected without a copied indexed mesh request"));
		assertTrue(livingEntityRenderer.contains("isVanillaCowModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaMushroomCowModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("MushroomCowRenderState mushroomCowRenderState"));
		assertTrue(livingEntityRenderer.contains("LobsterRenderState lobsterRenderState"));
		assertTrue(livingEntityRenderer.contains("BisonRenderState bisonRenderState"));
		assertTrue(livingEntityRenderer.contains("isVanillaPigModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaCreeperModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaFoxModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaSpiderModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaArmorStandModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaCatModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaWolfModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaVillagerModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaGuardianModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaSnowGolemModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaIronGolemModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaSkeletonModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaGhastModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaBlazeModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaWitchModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaEndermanModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaEndermiteModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaSilverfishModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaBatModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaCodModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaSalmonModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaPufferfishModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaTadpoleModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaOcelotModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaPolarBearModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaDolphinModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaTurtleModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaPandaModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaBeeModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaAxolotlModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaFrogModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaSquidModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaGoatModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaAllayModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaEvokerModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaVindicatorOrPillagerModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaParrotModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaTropicalFishModelMeshEligible("));
		assertTrue(worldRenderer.contains("isVanillaTropicalFishPatternModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaRavagerModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaVexModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaSlimeModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaMagmaCubeModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaPhantomModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaWardenModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaWitherModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaDrownedModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaCreakingModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaBreezeModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaCopperGolemModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaStriderModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaHoglinModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaCamelModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaPiglinModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaStrayModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaBoggedModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaGiantModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaArmadilloModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaSnifferModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaNautilusModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaHorseModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaDonkeyModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaLlamaModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaRabbitModelMeshEligible("));
		assertTrue(livingEntityRenderer.contains("isVanillaZombieModelMeshEligible("));
		assertTrue(worldRenderer.contains("isVanillaEndermanModelMeshEligible("));
		assertFalse(worldRenderer.contains("&& state.carriedBlock == null"),
			"Enderman carried blocks are separate semantic layer work and must not reject the Rust-owned base body");
		assertTrue(livingEntityRenderer.contains("ZombieRenderState zombieRenderState"));
		assertTrue(worldRenderer.contains("&& !state.displayFireAnimation"));
		assertTrue(deterministicCapture.contains("prepareModelMeshScenarioDifficulty"));
		assertTrue(deterministicCapture.contains("zombie-requires-non-peaceful-difficulty"));
		assertTrue(deterministicCapture.contains("difficultyEffective"));
		assertTrue(deterministicCapture.contains("case \"zombie\" -> \"minecraft:textures/entity/zombie/zombie.png\""));
		assertTrue(worldRenderer.contains("textures/entity/pig/temperate_pig.png"));
		assertTrue(worldRenderer.contains("textures/entity/pig/warm_pig.png"));
		assertFalse(worldRenderer.contains("textures/entity/pig/pig.png"),
			"the direct-texture Pig contract must use the current vanilla variant identities, not the removed pre-variant texture");
		assertTrue(livingEntityRenderer.contains("enqueueStandaloneModelMesh("));
		assertTrue(livingEntityRenderer.contains("EntityRenderDispatcher.isSemanticSubmission()"));
		int wholeFrameEntitySubmitStart = levelRenderer.indexOf("private void submitSelectedWholeFrameMeshEntities");
		int wholeFrameEntitySubmitEnd = levelRenderer.indexOf("private void submitWholeFrameWorldText", wholeFrameEntitySubmitStart);
		String wholeFrameEntitySubmit = levelRenderer.substring(wholeFrameEntitySubmitStart, wholeFrameEntitySubmitEnd);
		assertTrue(wholeFrameEntitySubmit.contains("entityRenderState instanceof net.minecraft.client.renderer.entity.state.CowRenderState"));
		assertTrue(wholeFrameEntitySubmit.contains("entityRenderState instanceof net.minecraft.client.renderer.entity.state.PigRenderState"),
			"an eligible Pig must reach the same real whole-frame entity submit traversal as Cow before its renderer makes the final route decision");
		assertTrue(modelSubmitter.contains("enqueueModelMesh("));
		assertTrue(modelSubmitter.contains("Rust whole-frame model route selected without a copied indexed mesh request"));
		assertTrue(routePolicy.contains("currentModelPartMeshRoute(boolean eligible)"));
		assertFalse(routePolicy.contains("mattmc.dev.rustGalWorldModelPart.v1"));
		assertTrue(worldRenderer.contains("isModelPartMeshEligible("));
		assertTrue(worldRenderer.contains("enqueueModelPartMesh("));
		assertTrue(worldRenderer.contains("modelPartEntityIdentity(textureIdentity)"));
		assertTrue(worldRenderer.contains("\":model_part/\""));
		assertTrue(worldRenderer.contains("if (renderType.isOutline()) return \"outline-render-type\";"),
			"outline-only render types must remain fail-closed while regular outlined meshes use Rust metadata");
		assertTrue(worldRenderer.contains("outlineColor\n\t\t\t\t));"),
			"Rust ModelPart mesh instances must carry the semantic outline color");
		assertTrue(worldRenderer.contains("resolvedModelInstanceColor(tintedColor)"));
		assertTrue(worldRenderer.contains("tintedColor == 0 ? 0xffffffff : tintedColor"));
		assertTrue(worldRenderer.contains("PENDING_MESH_PRODUCERS.add(PendingMeshProducer.MODEL_PART)"));
		assertTrue(worldRenderer.contains("The coordinator calls this only after the owning Rust whole-frame"));
		assertFalse(worldRenderer.contains("instancesByProducer.isEmpty() || !DeterministicCameraCapture.isActiveForDiagnostics()"));
		assertTrue(worldRenderer.contains("if (containsWorldMeshAsset(meshes, mesh.meshKey()))"));
		assertTrue(modelSubmitter.contains("currentModelPartMeshRoute(rustEligible)"));
		assertTrue(modelSubmitter.contains("Rust whole-frame ModelPart route selected without a copied indexed mesh request"));
		assertTrue(levelRenderer.contains("currentModelMeshRoute(true).usesRustWholeFrameVulkan()"));
		assertTrue(levelRenderer.contains("currentModelPartMeshRoute(true).usesRustWholeFrameVulkan()"));
		assertTrue(levelRenderer.contains("submitSelectedWholeFrameModelBlockEntities"));
		assertTrue(levelRenderer.contains("instanceof net.minecraft.client.renderer.blockentity.state.ChestRenderState"));
		assertTrue(levelRenderer.contains("instanceof BedRenderState"));
		assertTrue(levelRenderer.contains("instanceof net.minecraft.client.renderer.blockentity.state.DecoratedPotRenderState"));
		assertTrue(levelRenderer.contains("instanceof BellRenderState"));
		assertTrue(levelRenderer.contains("instanceof net.minecraft.client.renderer.entity.state.LlamaSpitRenderState"));
		assertTrue(levelRenderer.contains("Animated and translucent models remain semantic coverage"));
		assertTrue(levelRenderer.contains("isSelectedWholeFrameModelSemantic(model, state, sprite)"));
		assertTrue(levelRenderer.contains("instanceof net.minecraft.client.renderer.entity.state.LlamaSpitRenderState"));
		assertTrue(levelRenderer.contains("sprite.contents().name().getPath().startsWith(\"entity/bed/\")"));
		assertTrue(levelRenderer.contains("hasCurrentFrameRustModelMeshDecision(model, sprite)"),
			"selected-source coverage must only exempt a model after the real same-frame Rust submit queued it");
		assertTrue(worldRenderer.contains("RustGalFrameCoordinator.isRustShaderPackSourceReady()"),
			"automatic staged source admission must activate the matching Java semantic coverage pass");
		String llamaSpitRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/entity/LlamaSpitRenderer.java"));
		assertTrue(llamaSpitRenderer.contains("enqueueStandaloneModelMesh("));
		assertTrue(llamaSpitRenderer.contains("Rust whole-frame LlamaSpit route selected"));
		assertFalse(llamaSpitRenderer.contains("TextureAtlasSprite"));
		String evokerFangsRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/entity/EvokerFangsRenderer.java"));
		assertTrue(evokerFangsRenderer.contains("enqueueStandaloneModelMesh("));
		assertTrue(evokerFangsRenderer.contains("Rust whole-frame EvokerFangs route selected"));
		assertFalse(evokerFangsRenderer.contains("TextureAtlasSprite"));
		String witherSkullRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/entity/WitherSkullRenderer.java"));
		assertTrue(witherSkullRenderer.contains("enqueueStandaloneModelMesh("));
		assertTrue(witherSkullRenderer.contains("Rust whole-frame WitherSkull route selected"));
		assertFalse(witherSkullRenderer.contains("TextureAtlasSprite"));
		assertTrue(deterministicCapture.contains("\"evoker-fangs\".equals(MODEL_MESH_SCENARIO)"));
		assertTrue(deterministicCapture.contains("\"wither-skull\".equals(MODEL_MESH_SCENARIO)"));
		assertTrue(deterministicCapture.contains("\"cow\".equals(MODEL_MESH_SCENARIO)"));
		assertTrue(deterministicCapture.contains("\"pig\".equals(MODEL_MESH_SCENARIO)"));
		assertTrue(deterministicCapture.contains("\"end-crystal\".equals(MODEL_MESH_SCENARIO)"));
		assertTrue(deterministicCapture.contains("crystal.setBeamTarget"));
		assertTrue(deterministicCapture.contains("EntityType.END_CRYSTAL"));
		assertTrue(modelSubmitter.contains("recordSubmittedWorkIdentity(\n\t\t\t\"crystal-beam\""),
			"End Crystal beam traversal must publish a strict semantic receipt");
		assertTrue(deterministicCapture.contains("&& (MODEL_MESH_SCENARIO.isEmpty() || \"hidden\".equals(MODEL_MESH_SCENARIO))"));
	}

	@Test
	void experienceOrbRouteUsesOnlyTypedRustGeometry() throws Exception {
		String routePolicy = Files.readString(Path.of("src/main/java/net/vulkanic/world/WorldRenderRoutePolicy.java"));
		String orbRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/entity/ExperienceOrbRenderer.java"));
		String levelRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/LevelRenderer.java"));
		String worldRenderer = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		String coordinator = Files.readString(Path.of("src/main/java/net/vulkanic/gui/RustGalFrameCoordinator.java"));
		String capture = Files.readString(Path.of("src/main/java/net/minecraft/client/dev/DeterministicCameraCapture.java"));

		assertTrue(routePolicy.contains("currentExperienceOrbRoute()"));
		assertTrue(routePolicy.contains("mattmc.dev.rustGalWorldExperienceOrb.disabled"));
		assertTrue(routePolicy.contains("selectShaderAffectedRoute(VulkanicAPI.isVulkanBackendSelected(), rustWholeFrameShellActive())"));
		assertTrue(orbRenderer.contains("enqueueNativeExperienceOrb(poseStack.last(), experienceOrbRenderState,"));
		assertTrue(orbRenderer.indexOf("enqueueNativeExperienceOrb(") < orbRenderer.indexOf("poseStack.pushPose()"),
			"Rust must receive the parent transform before Java billboard expansion");
		assertFalse(orbRenderer.contains("enqueueExperienceOrb("));
		assertFalse(worldRenderer.contains("enqueueExperienceOrb("));
		assertFalse(worldRenderer.contains("\"mattmc.dev.nativeExperienceOrbGeometry\""),
			"the former private flag must not reopen Java-expanded Vulkan rendering");
		assertTrue(orbRenderer.contains("Rust whole-frame experience-orb route is unavailable while Rust owns presentation"));
		assertTrue(orbRenderer.contains("RustGalVulkanWholeFrameMode.enabled()"));
		assertTrue(orbRenderer.contains("!submitNodeCollector.isSemanticCoverageOnly()"));
		assertTrue(levelRenderer.contains("boolean experienceOrbs = net.vulkanic.world.WorldRenderRoutePolicy.currentExperienceOrbRoute().usesRustWholeFrameVulkan();"));
		assertTrue(levelRenderer.contains("experienceOrbs && entityRenderState instanceof ExperienceOrbRenderState"));
		assertTrue(worldRenderer.contains("MATERIAL_TEXTURE_EXPERIENCE_ORB"));
		assertTrue(worldRenderer.contains("MATERIAL_MODE_TRANSLUCENT"));
		assertTrue(worldRenderer.contains("ensureBoundedWorldPrimitiveViewportLocked(\"native orb requires a seeded frame\")"));
		assertTrue(worldRenderer.contains("recordWholeFrameExperienceOrbExecution"));
		assertTrue(coordinator.contains("recordWholeFrameExperienceOrbExecution("));
		assertTrue(capture.contains("setupExperienceOrbScenario"));
		assertTrue(capture.contains("rustGalWorldExperienceOrbExecution"));
		assertTrue(capture.contains("hasCurrentExperienceOrbRoute"));
	}

	@Test
	void beaconBeamRouteUsesTheSharedTranslucentMaterialPath() throws Exception {
		String routePolicy = Files.readString(Path.of("src/main/java/net/vulkanic/world/WorldRenderRoutePolicy.java"));
		String beaconRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/blockentity/BeaconRenderer.java"));
		String worldRenderer = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		String coordinator = Files.readString(Path.of("src/main/java/net/vulkanic/gui/RustGalFrameCoordinator.java"));
		String capture = Files.readString(Path.of("src/main/java/net/minecraft/client/dev/DeterministicCameraCapture.java"));
		String beaconEntity = Files.readString(Path.of("src/main/java/net/minecraft/world/level/block/entity/BeaconBlockEntity.java"));

		assertTrue(routePolicy.contains("currentBeaconBeamRoute()"));
		assertTrue(routePolicy.contains("mattmc.dev.rustGalWorldBeaconBeam.disabled"));
		int beaconRoute = routePolicy.indexOf("public static Route currentBeaconBeamRoute()");
		assertTrue(beaconRoute >= 0);
		String beaconRouteBody = routePolicy.substring(beaconRoute, routePolicy.indexOf("\n\t}\n", beaconRoute));
		assertTrue(beaconRouteBody.contains("RustGalVulkanWholeFrameMode.enabled()"),
			"beacon ownership must stay Rust-owned during the pre-selection whole-frame handoff");
		assertTrue(routePolicy.contains("selectWholeFrameRoute(VulkanicAPI.isVulkanBackendSelected(), rustWholeFrameShellActive())"));
		assertTrue(beaconRenderer.contains("enqueueBeaconBeam("));
		assertTrue(beaconRenderer.contains("boolean rustWholeFrame = net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()"),
			"beacon semantic admission must not wait for the backend-selection bit");
		assertTrue(beaconRenderer.contains("RustGalVulkanWholeFrameMode.enabled()")
			&& beaconRenderer.contains("Rust whole-frame beacon-beam route is unavailable while Rust owns presentation"),
			"disabled beacon semantics must not silently disappear after Rust presentation begins");
		assertTrue(beaconRenderer.contains("!submitNodeCollector.isSemanticCoverageOnly()"));
		assertTrue(beaconRenderer.contains("BEAM_LOCATION.equals(resourceLocation)"));
		assertTrue(worldRenderer.contains("MATERIAL_TEXTURE_BEACON_BEAM"));
		assertTrue(worldRenderer.contains("Rust VulkanicGAL BeaconBeam requires a seeded bounded world primitive frame"),
			"beacon semantic admission must reject oversized viewports before queueing material quads");
		assertTrue(worldRenderer.contains("MATERIAL_MODE_TRANSLUCENT"));
		assertTrue(worldRenderer.contains("MATERIAL_SOURCE_UV_LOCAL_TEXTURE"));
		assertTrue(worldRenderer.contains("recordWholeFrameBeaconBeamExecution"));
		assertTrue(coordinator.contains("recordWholeFrameBeaconBeamExecution("));
		assertTrue(capture.contains("setupBeaconBeamScenario"));
		assertTrue(capture.contains("hasCurrentBeaconBeamRoute"));
		assertTrue(beaconEntity.contains("this.levels = valueInput.getIntOr(\"Levels\", 0)"),
			"vanilla beacon activation level must synchronize through the ordinary block-entity update packet");
		assertFalse(beaconRenderer.contains("TextureAtlasSprite"));
	}

	@Test
	void unknownBeaconBeamTexturesCannotReopenJavaGeometryDuringPresenterHandoff() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/blockentity/BeaconRenderer.java"));
		int unknown = source.indexOf("Unknown beam textures retain");
		int compatibility = source.indexOf("Route.JAVA_COMPATIBILITY", unknown);
		String body = source.substring(unknown, compatibility);
		assertTrue(body.contains("isVulkanBackendSelected()")
			&& body.contains("RustGalVulkanWholeFrameMode.enabled()"),
			"unknown beacon textures must be disabled as soon as the Rust presenter shell owns the frame");
	}

	@Test
	void entityFireRouteIsCollectedBeforeWholeFrameFeatureQueuesAreCleared() throws Exception {
		String routePolicy = Files.readString(Path.of("src/main/java/net/vulkanic/world/WorldRenderRoutePolicy.java"));
		String features = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/feature/FeatureRenderDispatcher.java"));
		String levelRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/LevelRenderer.java"));
		String worldRenderer = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		String coordinator = Files.readString(Path.of("src/main/java/net/vulkanic/gui/RustGalFrameCoordinator.java"));
		String capture = Files.readString(Path.of("src/main/java/net/minecraft/client/dev/DeterministicCameraCapture.java"));

		assertTrue(routePolicy.contains("currentEntityFlameRoute()"));
		assertTrue(routePolicy.contains("mattmc.dev.rustGalWorldEntityFlame.disabled"));
		assertTrue(routePolicy.contains("selectWholeFrameRoute(VulkanicAPI.isVulkanBackendSelected(), rustWholeFrameShellActive())"));
		assertTrue(features.contains("collectEntityFlameSemantics(submitNodeCollection.getFlameSubmits(), this.atlasManager)"));
		assertTrue(features.contains("public void renderBlockFeaturesOnly()"));
		assertTrue(features.contains("this.submitNodeStorage.clear();"));
		assertTrue(levelRenderer.contains("!net.vulkanic.world.WorldRenderRoutePolicy.currentEntityFlameRoute().usesRustWholeFrameVulkan()"));
		assertTrue(worldRenderer.contains("collectEntityFlameSemantics("));
		assertTrue(worldRenderer.contains("MATERIAL_TEXTURE_TERRAIN_BLOCK_ATLAS"));
		assertTrue(worldRenderer.contains("MATERIAL_SOURCE_UV_MINECRAFT_BLOCK_ATLAS"));
		assertTrue(worldRenderer.contains("MATERIAL_MODE_CUTOUT"));
		assertTrue(worldRenderer.contains("Rust whole-frame entity-fire route requires a seeded bounded world primitive frame"));
		assertTrue(worldRenderer.contains("Rust whole-frame entity-fire route selected before the copied terrain atlas was registered"));
		assertTrue(worldRenderer.contains("\"entity-flame\", \"rust-vulkan-whole-frame:cutout-quads=\""));
		assertTrue(worldRenderer.contains("MAX_ENTITY_FLAME_SUBMITS")
			&& worldRenderer.contains("MAX_ENTITY_FLAME_QUADS")
			&& worldRenderer.contains("estimatedQuads")
			&& worldRenderer.contains("material capacity exceeded"),
			"entity-fire extraction must preflight bounded submit and material capacity before appending quads");
		assertTrue(worldRenderer.contains("pendingEntityFlameQuadCount"));
		assertTrue(worldRenderer.contains("recordWholeFrameEntityFlameExecution"));
		assertTrue(worldRenderer.contains("currentCaptureCorrelationRenderedFrameIndex(),\n\t\t\t\t\"rust-vulkan-whole-frame\", frameId, submissionId, quads"),
			"entity-fire execution receipts must retain the capture request identity across asynchronous submission");
		assertTrue(coordinator.contains("primitiveFrame.entityFlameQuadCount()"));
		assertTrue(capture.contains("mattmc.dev.rustGalWorldEntityFlame.scenario"));
		assertTrue(capture.contains("igniteEntityFlameCarrier"));
		assertTrue(capture.contains("hasCurrentEntityFlameRoute"));
		assertTrue(capture.contains("entityFlameSemantic"));
		assertTrue(capture.contains("entityFlameExecuted"));
	}

	@Test
	void entityShadowRouteUsesCopiedMaterialQuadsWithoutJavaDrawOrBackendState() throws Exception {
		String routePolicy = Files.readString(Path.of("src/main/java/net/vulkanic/world/WorldRenderRoutePolicy.java"));
		String features = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/feature/FeatureRenderDispatcher.java"));
		String entityDispatcher = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/entity/EntityRenderDispatcher.java"));
		String worldRenderer = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		String coordinator = Files.readString(Path.of("src/main/java/net/vulkanic/gui/RustGalFrameCoordinator.java"));

		assertTrue(routePolicy.contains("currentEntityShadowRoute()"));
		assertTrue(routePolicy.contains("mattmc.dev.rustGalWorldEntityShadow.disabled"));
		assertTrue(routePolicy.contains("selectWholeFrameRoute(VulkanicAPI.isVulkanBackendSelected(), rustWholeFrameShellActive())"));
		assertTrue(features.contains("collectEntityShadowSemantics(submitNodeCollection.getShadowSubmits())"));
		assertTrue(features.contains("public void renderBlockFeaturesOnly()"));
		assertTrue(entityDispatcher.contains("rustWholeFrameShadowRoute"));
		assertTrue(entityDispatcher.contains("!rustWholeFrameShadowRoute"));
		assertTrue(entityDispatcher.contains("submitNodeCollector.submitShadow"));
		assertTrue(worldRenderer.contains("collectEntityShadowSemantics("));
		assertTrue(worldRenderer.contains("MATERIAL_TEXTURE_ENTITY_SHADOW"));
		assertTrue(worldRenderer.contains("textures/misc/shadow.png"));
		assertTrue(worldRenderer.contains("MATERIAL_MODE_TRANSLUCENT"));
		assertTrue(worldRenderer.contains("DEPTH_POLICY_TEST_NO_WRITE"));
		assertTrue(worldRenderer.contains("Rust whole-frame entity-shadow route requires a seeded bounded world primitive frame"));
		assertTrue(worldRenderer.contains("MAX_ENTITY_SHADOW_SUBMITS")
			&& worldRenderer.contains("MAX_ENTITY_SHADOW_PIECES")
			&& worldRenderer.contains("estimatedPieces")
			&& worldRenderer.contains("entity-shadow material capacity exceeded"),
			"entity-shadow extraction must preflight bounded submits, pieces, and material capacity");
		assertTrue(worldRenderer.contains("Rust whole-frame entity-leash route requires a seeded bounded world primitive frame"));
		assertTrue(worldRenderer.contains("MAX_ENTITY_LEASH_SUBMITS")
			&& worldRenderer.contains("MAX_ENTITY_LEASH_QUADS")
			&& worldRenderer.contains("estimatedQuads")
			&& worldRenderer.contains("entity-leash material capacity exceeded"),
			"entity-leash extraction must preflight bounded submits, quads, and material capacity");
		assertTrue(worldRenderer.contains("recordWholeFrameEntityShadowExecution"));
		assertTrue(coordinator.contains("recordWholeFrameEntityShadowExecution("));
		assertTrue(worldRenderer.contains("currentCaptureCorrelationRenderedFrameIndex(),\n\t\t\t\t\"rust-vulkan-whole-frame\", frameId, submissionId, quads"),
			"entity-shadow/leash execution receipts must retain the capture request identity");
		assertFalse(worldRenderer.contains("TextureAtlasSprite entityShadow"));
		assertFalse(worldRenderer.contains("RenderType.entityShadow"));
	}

	@Test
	void itemEntityRouteUsesOnlyTheSharedIndexedMeshPath() throws Exception {
		String routePolicy = Files.readString(Path.of("src/main/java/net/vulkanic/world/WorldRenderRoutePolicy.java"));
		String itemRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/entity/ItemEntityRenderer.java"));
		String submitCollection = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/SubmitNodeCollection.java"));
		String levelRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/LevelRenderer.java"));
		String worldRenderer = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		String deterministicCapture = Files.readString(Path.of("src/main/java/net/minecraft/client/dev/DeterministicCameraCapture.java"));

		assertTrue(routePolicy.contains("currentItemEntityMeshRoute(boolean eligible)"));
		assertTrue(routePolicy.contains("mattmc.dev.rustGalWorldItemEntity.disabled"));
		assertTrue(routePolicy.contains("selectWholeFrameRoute(VulkanicAPI.isVulkanBackendSelected(), rustWholeFrameShellActive())"));
		assertTrue(itemRenderer.contains("beginItemEntitySubmission()"));
		assertTrue(itemRenderer.contains("endItemEntitySubmission()"));
		assertTrue(submitCollection.contains("isIndexedItemSubmissionActive()"));
		assertTrue(submitCollection.contains("itemEntityMeshIneligibility("));
		assertTrue(submitCollection.contains("currentItemEntityMeshRoute(rustEligible)"));
		assertTrue(submitCollection.contains("currentItemEntityOwnershipRoute().usesRustWholeFrameVulkan()")
			&& submitCollection.contains("Rust whole-frame item-entity route has no semantic mesh"),
			"unavailable Rust-owned dropped items must abort instead of silently disappearing");
		assertTrue(submitCollection.contains("enqueueItemEntityMesh("));
		assertTrue(levelRenderer.contains("currentItemEntityMeshRoute(true).usesRustWholeFrameVulkan()"));
		assertTrue(levelRenderer.contains("itemEntities && entityRenderState instanceof ItemEntityRenderState"));
		assertTrue(worldRenderer.contains("displayContext != ItemDisplayContext.GROUND"));
		assertTrue(worldRenderer.contains("foilType == ItemStackRenderState.FoilType.SPECIAL"));
		assertTrue(worldRenderer.contains("ItemStackRenderState deliberately keeps an empty tint array"));
		assertTrue(worldRenderer.contains("itemQuadTintColor(bakedQuad, tintLayers)"));
		assertTrue(worldRenderer.contains("extractItemQuadMesh("));
		assertTrue(worldRenderer.contains("MATERIAL_ID_TRANSLUCENT_TEXTURED"));
		assertTrue(worldRenderer.contains("MATERIAL_ID_GLINT_TEXTURED"));
		assertTrue(worldRenderer.contains("itemIdentity + \":glint\""));
		assertTrue(worldRenderer.contains("firstPersonItemMeshIneligibility"));
		assertTrue(worldRenderer.contains("Rust first-person route selected without a seeded bounded world viewport"));
		assertTrue(worldRenderer.contains("hasFoil && readTexturePayloadForResource(ItemRenderer.ENCHANTED_GLINT_ITEM)"));
		assertTrue(worldRenderer.contains("extractModelPartMesh(modelPart, textureIdentity, sprite"));
		assertTrue(worldRenderer.contains("DEPTH_POLICY_TEST_NO_WRITE"));
		assertTrue(worldRenderer.contains("PendingMeshProducer.ITEM_ENTITY")
			&& worldRenderer.contains("PendingMeshProducer.BLOCK_ENTITY_ITEM"));
		assertTrue(worldRenderer.contains("\"item-entity\".equals(producer)"));
		assertFalse(submitCollection.contains("getItemRenderer().render"));
		assertTrue(deterministicCapture.contains("setupItemEntityScenario"));
		assertTrue(deterministicCapture.contains("new ItemEntity(serverLevel"));
		assertTrue(deterministicCapture.contains("hasCurrentItemEntityRoute"));
		assertTrue(deterministicCapture.contains("if (!movingMeshProducerReady())"));
		assertFalse(deterministicCapture.contains("!wholeFrameAttachmentCaptureReady && !movingMeshProducerReady()"));
		assertTrue(deterministicCapture.contains("movingMeshProducerReady(deterministicRenderedFrameIndex)"));
		assertTrue(deterministicCapture.contains("required-moving-producer-not-in-submission"));
		String worldFrontend = Files.readString(Path.of("src/main/rust/render/vulkanic/world_primitive_frontend.rs"));
		assertTrue(worldFrontend.contains("WORLD_MATERIAL_MODE_TRANSLUCENT"));
		assertTrue(worldFrontend.contains("depth_policy != WORLD_DEPTH_POLICY_TEST_NO_WRITE"));
	}

	@Test
	void specialItemSemanticDispatchPreservesFoilState() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/item/ItemStackRenderState.java"));
		String world = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		assertTrue(source.contains("pendingSemanticItemSubmissionCount()"),
			"special item receipts must include explicit material-quad submissions");
		assertTrue(world.contains("PENDING_MATERIAL_QUADS.size()"),
			"semantic special-item receipt must observe Rust material-quad admission");
		assertTrue(source.contains("layer.foilType != FoilType.NONE"),
			"special renderer semantic dispatch must preserve the resolved layer foil state");
		assertTrue(source.contains("this.foilType != FoilType.NONE"),
			"item-entity special renderer dispatch must preserve foil state for Rust glint extraction");
	}

	@Test
	void mapRendererCannotFallThroughToJavaGeometryOnRustWholeFrame() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/MapRenderer.java"));
		assertTrue(source.contains("currentWorldTextRoute()")
			&& source.contains("boolean rustWorldText"),
			"map labels must follow the explicit world-text route during Vulkan handoff, not only the shell flag");
		assertTrue(source.contains("Rust whole-frame map route rejected the copied map quad"));
		assertTrue(source.contains("Rust whole-frame map route rejected a copied decoration quad"));
		assertTrue(source.contains("!mapAccepted && net.vulkanic.VulkanicAPI.isVulkanBackendSelected()"));
		assertTrue(source.contains("!decorationAccepted && net.vulkanic.VulkanicAPI.isVulkanBackendSelected()"));
		assertTrue(source.contains("boolean mapAccepted"));
		assertTrue(source.contains("boolean decorationAccepted"));
	}

	@Test
	void indexedMeshShaderStateIdentityUsesTheCopiedRuntimeTableKey() throws Exception {
		String worldRenderer = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));

		assertTrue(worldRenderer.contains("int rawStateId = Block.getId(blockState);"));
		assertFalse(worldRenderer.contains("return blockState.toString().hashCode();"));
	}

	@Test
	void staticTerrainBuildMetadataRouteDoesNotDependOnBackendSelectionTiming() {
		String previousWholeFrame = System.getProperty("mattmc.dev.rustGalVulkanWholeFrame");
		String previousDisabled = System.getProperty("mattmc.dev.rustGalStaticTerrain.disabled");
		String previousLegacy = System.getProperty("mattmc.dev.rustGalStaticTerrain.legacyControl");
		try {
			System.setProperty("mattmc.dev.rustGalVulkanWholeFrame", "true");
			System.clearProperty("mattmc.dev.rustGalStaticTerrain.disabled");
			System.clearProperty("mattmc.dev.rustGalStaticTerrain.legacyControl");
			assertTrue(WorldRenderRoutePolicy.staticTerrainBuildRequiresRustWholeFrameMetadata());

			System.setProperty("mattmc.dev.rustGalStaticTerrain.disabled", "true");
			assertFalse(WorldRenderRoutePolicy.staticTerrainBuildRequiresRustWholeFrameMetadata());
			System.clearProperty("mattmc.dev.rustGalStaticTerrain.disabled");

			System.setProperty("mattmc.dev.rustGalStaticTerrain.legacyControl", "true");
			assertFalse(WorldRenderRoutePolicy.staticTerrainBuildRequiresRustWholeFrameMetadata());
		} finally {
			restoreProperty("mattmc.dev.rustGalVulkanWholeFrame", previousWholeFrame);
			restoreProperty("mattmc.dev.rustGalStaticTerrain.disabled", previousDisabled);
			restoreProperty("mattmc.dev.rustGalStaticTerrain.legacyControl", previousLegacy);
		}
	}

	@Test
	void shaderPackSourceTransportIsWholeFrameOnlyAndDoesNotBorrowIrisRuntimeState() throws Exception {
		String coordinator = Files.readString(Path.of("src/main/java/net/vulkanic/gui/RustGalFrameCoordinator.java"));
		String collector = Files.readString(Path.of("src/main/java/net/vulkanic/shaderpack/RustShaderPackSourceCollector.java"));
		String irisCompatibilityCollector = Files.readString(Path.of("src/main/java/net/vulkanic/shaderpack/IrisShaderPackCompatibilityCollector.java"));
		assertTrue(coordinator.contains("RustGalGuiRenderer.isWholeFrameVulkanActive()"));
		assertTrue(coordinator.contains("bridge.updateShaderPackSources("));
		assertTrue(coordinator.contains("bridge.updateShaderPackAssets("));
		assertTrue(coordinator.contains("uploadedShaderPackSourceGeneration < generation"));
		assertTrue(coordinator.contains("source_execution_selected=deferred-until-frame-admission"));
		assertFalse(collector.contains("IrisRenderingPipeline"));
		assertFalse(collector.contains("WorldRenderingPipeline"));
		assertFalse(collector.contains("net.irisshaders"),
			"Rust whole-frame source collection must not retain an Iris package dependency");
		assertFalse(collector.contains("GlImage"));
		assertFalse(collector.contains("MemorySegment"));
		assertTrue(collector.contains("collectWithAssets"));
		assertTrue(collector.contains("file count exceeds \" + MAX_FILES + \" after adding"));
		assertTrue(collector.contains("merged post-effect shader asset count exceeds \" + MAX_ASSET_FILES"));
		assertTrue(collector.contains("MAX_RUNTIME_PROPERTIES = 4096")
			&& collector.contains("entry count exceeds " + "\" + MAX_RUNTIME_PROPERTIES"));
		assertTrue(collector.contains("RustGalVulkanWholeFrameMode.enabled()")
			&& collector.contains("VulkanicAPI.isVulkanBackendSelected()")
			&& collector.contains("wholeFrameShaderConfigEnabled()")
			&& !collector.contains("Iris.getIrisConfig()"),
			"selected Vulkan shader-source discovery must not depend on Iris runtime state");
		int selectedGate = collector.indexOf("net.vulkanic.VulkanicAPI.isVulkanBackendSelected()");
		int compatibilityFallback = collector.indexOf("IrisShaderPackCompatibilityCollector.collectConfiguredPack");
		assertTrue(selectedGate >= 0 && compatibilityFallback > selectedGate,
			"selected Vulkan must choose copied shader sources before the Iris compatibility fallback");
		assertTrue(collector.contains("ShaderPackAssetFileRecord"));
		assertTrue(collector.contains("activeVanillaPostEffectId()"));
		assertTrue(collector.contains("collectVanillaPostEffect("));
		assertTrue(collector.contains("withActiveVanillaPostEffectResources("));
		assertTrue(collector.contains("Iris-owned files win on path collisions"));
		assertTrue(collector.contains("sourceGenerationKey("));
		assertTrue(coordinator.contains("RustShaderPackSourceCollector.collectConfiguredPack(sourceGeneration)"));
		assertTrue(coordinator.contains("pendingShaderPackSourceName = selectionKey"));
		assertTrue(collector.contains("getResourceManager()"));
		assertTrue(collector.contains("post_effect/"));
		assertTrue(collector.contains("readBoundedResource"));
		assertTrue(collector.contains("Math.addExact(sourceBytes")
			&& collector.contains("resource-pack post-effect source payload exceeds")
			&& collector.contains("post-effect texture asset payload exceeds")
			&& collector.contains("resource-pack post-effect asset payload exceeds"),
			"resource-pack post-effect snapshots must enforce aggregate source and asset bounds");
		assertTrue(collector.contains("post-effect texture input path ")
			&& collector.contains("is ambiguous between"),
			"copied post-effect textures must fail closed when distinct namespaces normalize to one asset path");
		assertTrue(irisCompatibilityCollector.contains("resolved include count exceeds")
			&& irisCompatibilityCollector.contains("RustShaderPackSourceCollector.MAX_FILES")
			&& irisCompatibilityCollector.contains("MAX_COMPATIBILITY_ENTRIES = 4_096")
			&& irisCompatibilityCollector.contains("putBounded"));
		assertTrue(irisCompatibilityCollector.contains("Iris.getIrisConfig().areShadersEnabled()")
			&& irisCompatibilityCollector.contains("getBooleanValueOrDefault(name)")
			&& irisCompatibilityCollector.contains("getStringValueOrDefault(name)"));
	}

	private static void restoreProperty(String key, String value) {
		if (value == null) {
			System.clearProperty(key);
		} else {
			System.setProperty(key, value);
		}
	}

	@Test
	void indexedBakedMeshesUseExplicitCcWFrontFaceContract() throws Exception {
		String worldRenderer = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		String rustMeshFrontend = Files.readString(Path.of("src/main/rust/render/vulkanic/world_primitive_frontend.rs"));
		String rustOpenGlLowering = Files.readString(Path.of("src/main/rust/render/vulkanic/backends/opengl/lowering.rs"));
		String rustVulkanResources = Files.readString(Path.of("src/main/rust/render/vulkanic/backends/vulkan/resources.rs"));

		assertTrue(worldRenderer.contains("return dot >= 0.0F ? WORLD_WINDING_CCW : WORLD_WINDING_CW;"),
			"baked quads whose emitted indices face the baked Direction must be tagged as CCW for GAL culling");
		assertTrue(rustMeshFrontend.contains("(WORLD_CULL_BACK, WORLD_WINDING_CCW) => Ok(CullMode::Back)"));
		assertTrue(rustMeshFrontend.contains("(WORLD_CULL_BACK, WORLD_WINDING_CW) => Ok(CullMode::Front)"));
		assertTrue(rustOpenGlLowering.contains(".front_face(if front_face_ccw { glow::CCW } else { glow::CW })"),
			"OpenGL backend must apply the explicit GAL front-face convention instead of inheriting Java/Iris state");
		assertTrue(rustVulkanResources.contains(".front_face(front_face(match (desc.front_face, desc.raster_y_direction)"),
			"Vulkan must lower the explicit logical front face together with the declared raster direction");
		assertTrue(rustVulkanResources.contains("(face, RasterYDirection::Up) => face"),
			"the existing Up convention must preserve logical winding");
		assertTrue(rustVulkanResources.contains("(crate::render::vulkanic::resources::FrontFace::Clockwise, RasterYDirection::Down) => crate::render::vulkanic::resources::FrontFace::CounterClockwise"));
		assertTrue(rustVulkanResources.contains("(crate::render::vulkanic::resources::FrontFace::CounterClockwise, RasterYDirection::Down) => crate::render::vulkanic::resources::FrontFace::Clockwise"),
			"Down must compensate both windings so logical culling does not change");
		assertTrue(rustVulkanResources.contains("FrontFace::CounterClockwise"));
		assertTrue(rustVulkanResources.contains("vk::FrontFace::COUNTER_CLOCKWISE"));
	}

	@Test
	void pistonMovingMeshParticipatesInOpenGlFrameSeedingAndFlush() throws Exception {
		String worldRenderer = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		String blockFeatureRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/feature/BlockFeatureRenderer.java"));
		String frameBenchmark = Files.readString(Path.of("src/main/java/net/minecraft/client/dev/GraphicsFrameBenchmark.java"));
		String deterministicCapture = Files.readString(Path.of("src/main/java/net/minecraft/client/dev/DeterministicCameraCapture.java"));
		String pistonEntity = Files.readString(Path.of("src/main/java/net/minecraft/world/level/block/piston/PistonMovingBlockEntity.java"));
		int meshHelperStart = worldRenderer.indexOf("private static boolean shouldUseRustOpenGlMeshInstances()");
		int meshHelperEnd = worldRenderer.indexOf("public static boolean shouldRouteTerrainParticle", meshHelperStart);
		String meshHelper = worldRenderer.substring(meshHelperStart, meshHelperEnd);
		assertTrue(meshHelper.contains("currentBlockDisplayRoute().usesRustOpenGl()"));
		assertTrue(meshHelper.contains("currentFallingBlockRoute().usesRustOpenGl()"));
		assertTrue(meshHelper.contains("currentPistonMovingBlockRoute().usesRustOpenGl()"));
		assertTrue(meshHelper.contains("currentPrimedTntRoute().usesRustOpenGl()"));

		int frameHelperStart = worldRenderer.indexOf("public static boolean shouldUseRustOpenGlWorldPrimitives()");
		int frameHelperEnd = worldRenderer.indexOf("public static boolean crackDisabledForDiagnostics()", frameHelperStart);
		String frameHelper = worldRenderer.substring(frameHelperStart, frameHelperEnd);
		assertTrue(frameHelper.contains("currentPistonMovingBlockRoute().usesRustOpenGl()"));
		assertTrue(frameHelper.contains("currentPrimedTntRoute().usesRustOpenGl()"));
		assertTrue(worldRenderer.contains("movingBlockLightColor(movingBlockRenderState, blockState, movingBlockRenderState.blockPos)"));
		assertTrue(worldRenderer.contains("return dot >= 0.0F ? WORLD_WINDING_CCW : WORLD_WINDING_CW;"));
		assertTrue(worldRenderer.contains("public static PrimitiveFrame withViewport(PrimitiveFrame frame, int viewportWidth, int viewportHeight)"));
		int viewportNormalizer = worldRenderer.indexOf("public static PrimitiveFrame withViewport(PrimitiveFrame frame, int viewportWidth, int viewportHeight)");
		int normalizedFlags = worldRenderer.indexOf("instance.flags()", viewportNormalizer);
		int normalizedBlockEntity = worldRenderer.indexOf("instance.blockEntityId()", normalizedFlags);
		assertTrue(normalizedFlags > viewportNormalizer && normalizedBlockEntity > normalizedFlags,
			"viewport normalization must preserve mesh outline flags and block-entity identity");
		int materialNormalizer = worldRenderer.indexOf("List<VulkanicGalBridge.WorldMaterialQuadRecord> materialQuads", viewportNormalizer);
		int normalizedMaterialBlockEntity = worldRenderer.indexOf("quad.blockEntityId()", materialNormalizer);
		assertTrue(materialNormalizer > viewportNormalizer && normalizedMaterialBlockEntity > materialNormalizer,
			"viewport normalization must preserve material-quad block-entity identity");
		assertTrue(worldRenderer.contains("List<VulkanicGalBridge.WorldMeshInstanceRecord> firstPersonMeshInstances")
			&& worldRenderer.contains("List.copyOf(firstPersonMeshInstances)"),
			"viewport normalization must rebuild the dedicated first-person mesh domain");
		assertFalse(worldRenderer.contains("currentFallingBlockRoute().usesRustOpenGl() && net.irisshaders.iris.Iris.isPackInUseQuick()"));
		assertFalse(blockFeatureRenderer.contains("Iris.isPackInUseQuick()"));
		String coordinator = Files.readString(Path.of("src/main/java/net/vulkanic/gui/RustGalFrameCoordinator.java"));
		assertTrue(coordinator.contains("RustGalWorldPrimitiveRenderer.withViewport("));
		assertTrue(frameBenchmark.contains("Direction direction = pistonDirection();"));
		assertTrue(frameBenchmark.contains("PistonHeadBlock.FACING, pistonDirection()"));
		assertTrue(frameBenchmark.contains("getEyePosition().add(forward.normalize().scale(4.0))"));
		assertTrue(pistonEntity.contains("mattmc.dev.rustGalWorldMesh.pistonProgress"));
		assertTrue(deterministicCapture.contains("Direction direction = pistonDirection();"));
		assertTrue(deterministicCapture.contains("PistonHeadBlock.FACING, pistonDirection()"));
		assertTrue(deterministicCapture.contains("getEyePosition().add(forward.normalize().scale(4.0))"));
	}

	@Test
	void wholeFrameWorldPrimitiveRouteIsVulkanShellOnlyAndCoarse() throws Exception {
		String bridge = Files.readString(Path.of("src/main/java/net/vulkanic/bridge/VulkanicGalBridge.java"));
		String frameCoordinator = Files.readString(Path.of("src/main/java/net/vulkanic/gui/RustGalFrameCoordinator.java"));
		assertTrue(frameCoordinator.contains("assertWholeFrameFeatureCoverage(primitiveFrame.featureCoverage())"),
			"whole-frame submission must inspect copied feature-family coverage before native submission");
		assertTrue(frameCoordinator.contains("Rust whole-frame feature coverage contains unadmitted semantic families"),
			"unadmitted feature families must fail closed before the single Rust presenter");
		int particleCoverageStart = frameCoordinator.indexOf("private static void assertWholeFrameFeatureCoverage");
		int particleCoverageEnd = frameCoordinator.indexOf("private static void appendCoverage", particleCoverageStart);
		assertTrue(particleCoverageStart >= 0 && particleCoverageEnd > particleCoverageStart);
		String coverageBody = frameCoordinator.substring(particleCoverageStart, particleCoverageEnd);
		assertFalse(coverageBody.contains("appendCoverage(unsupported, \"particle-group\""),
			"supported particle groups must reach Rust semantic admission instead of a blanket Java preflight rejection");
		assertTrue(frameCoordinator.contains("pendingUnsupportedParticleGroups()"),
			"unsupported particle callbacks must still fail closed before whole-frame submission");
		String guiRenderer = Files.readString(Path.of("src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"));
		String worldRenderer = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		String worldRoutePolicy = Files.readString(Path.of("src/main/java/net/vulkanic/world/WorldRenderRoutePolicy.java"));
		String gameRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
		String levelRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/LevelRenderer.java"));
		String sodiumWorldRenderer = Files.readString(Path.of("src/main/java/net/sodium/client/render/SodiumWorldRenderer.java"));
		String renderSectionManager = Files.readString(Path.of("src/main/java/net/sodium/client/render/chunk/RenderSectionManager.java"));
		String terrainRenderer = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalTerrainRenderer.java"));
		String wholeFrameTerrainSource = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWholeFrameTerrainSource.java"));
		assertTrue(wholeFrameTerrainSource.contains("getEffectiveRenderDistance()"));
		assertTrue(wholeFrameTerrainSource.contains("admitSection"));
		assertTrue(wholeFrameTerrainSource.contains("drainVisibilityFrontier"));
		assertTrue(wholeFrameTerrainSource.contains("OcclusionCuller.getVisibilityConnections"));
		assertTrue(wholeFrameTerrainSource.contains("MAX_SEMANTIC_MESH_WORKERS = 8"));
		assertTrue(wholeFrameTerrainSource.contains("new ChunkBuilder(level, ChunkMeshFormats.COMPACT, false, semanticMeshWorkerCount())"),
			"Rust whole-frame compact terrain must use the explicit baked-RGB ABI without a fixed worker-count assumption");
		assertTrue(wholeFrameTerrainSource.contains("workerBuilder.scheduleTask"));
		assertTrue(wholeFrameTerrainSource.contains("renderContext, SortBehavior.STATIC, true"),
			"Rust whole-frame terrain must retain static translucent sort metadata for fluid parity");
		assertTrue(wholeFrameTerrainSource.contains("propagationPending"));
		assertTrue(wholeFrameTerrainSource.contains("Frustum frustum"));
		assertTrue(wholeFrameTerrainSource.contains("drainCompletedBuilds(frustum)"));
		assertFalse(wholeFrameTerrainSource.contains("RenderDevice"),
			"the direct CPU terrain source must not construct Sodium's OpenGL render-device scope");
		assertFalse(wholeFrameTerrainSource.contains("task.execute(this."),
			"whole-frame terrain meshing must not synchronously block the render thread");
		assertFalse(wholeFrameTerrainSource.contains("BUILDS_PER_FRAME"));
		assertTrue(wholeFrameTerrainSource.contains("isWholeFrameSurfaceQueueDrained"));
		assertTrue(wholeFrameTerrainSource.contains("isWholeFrameTerrainQueueDrained"));
		assertTrue(wholeFrameTerrainSource.contains("wholeFrameTerrainQueueSummary"));
		assertTrue(wholeFrameTerrainSource.contains("this.level.hasChunk(section.getX(), section.getZ())"),
			"the direct CPU source must only mesh client-resident chunks and never request server generation");
		assertFalse(wholeFrameTerrainSource.contains("HORIZONTAL_RADIUS"));
		assertFalse(wholeFrameTerrainSource.contains("VERTICAL_RADIUS"));
		String fluidRenderer = Files.readString(Path.of("src/main/java/net/sodium/client/render/chunk/compile/pipeline/DefaultFluidRenderer.java"));
		String blockRenderer = Files.readString(Path.of("src/main/java/net/sodium/client/render/chunk/compile/pipeline/BlockRenderer.java"));
		String deterministicCapture = Files.readString(Path.of("src/main/java/net/minecraft/client/dev/DeterministicCameraCapture.java"));
		String graphicsHarness = Files.readString(Path.of("DevUtils/Common/graphics_harness.py"));
		String rustFfi = readRustFfiModules();
		String rustWorldFrontend = Files.readString(Path.of("src/main/rust/render/vulkanic/world_primitive_frontend.rs"));
		String rustTerrainFrontend = Files.readString(Path.of("src/main/rust/render/vulkanic/terrain/mod.rs"));

		assertTrue(bridge.contains("mattmc_vulkanic_gal_whole_frame_submit"));
		assertTrue(bridge.contains("mattmc_vulkanic_gal_world_border_update_asset"));
		assertTrue(bridge.contains("record WorldBorderAssetRecord"));
		assertTrue(bridge.contains("updateWorldBorderAsset"));
			assertTrue(bridge.contains("record WorldLineSegmentRecord"));
			assertTrue(bridge.contains("record WorldCrackQuadRecord"));
			assertTrue(bridge.contains("record WorldBorderQuadRecord"));
			assertTrue(bridge.contains("record WorldBackgroundRecord"));
			assertTrue(bridge.contains("List<WorldLineSegmentRecord> worldSegments"));
			assertTrue(bridge.contains("List<WorldCrackQuadRecord> worldCrackQuads"));
			assertTrue(bridge.contains("List<WorldBorderQuadRecord> worldBorderQuads"));
			assertTrue(bridge.contains("submitWorldPrimitives("));
			assertTrue(bridge.contains("worldCrackQuads,"));
			assertTrue(frameCoordinator.contains("executeWholeFrameVulkan"));
			assertTrue(frameCoordinator.contains("executeWorldPrimitiveFrame"));
			assertTrue(frameCoordinator.contains("bridge.submitWholeFrame"));
			assertTrue(frameCoordinator.contains("primitiveFrame.crackQuads()"));
			assertTrue(guiRenderer.contains("isWholeFrameVulkanActive()"));
			int worldPrimitiveExecution = frameCoordinator.indexOf("public static boolean executeWorldPrimitiveFrame");
			int worldCrackAssetFlush = frameCoordinator.indexOf("flushPendingWorldAssetsLocked();", worldPrimitiveExecution);
			int worldPrimitiveSubmit = frameCoordinator.indexOf("bridge.submitWorldPrimitives", worldPrimitiveExecution);
			assertTrue(worldCrackAssetFlush > worldPrimitiveExecution && worldCrackAssetFlush < worldPrimitiveSubmit,
				"standalone world primitive execution must flush crack assets before submitting real crack work");
		assertTrue(gameRenderer.contains("renderRustVulkanWholeFrameShell"));
		assertTrue(gameRenderer.contains("private float fovModifier = 1.0F;"),
			"the whole-frame shell must start from the neutral gameplay FOV before its first client tick");
			assertTrue(gameRenderer.contains("RustGalWorldPrimitiveRenderer.enqueueBlockOutline"));
			assertTrue(worldRenderer.contains("Rust whole-frame outline route rejected visible semantic work"),
				"visible rejected outline work must not be silently omitted while Rust owns presentation");
		assertTrue(worldRenderer.contains("Rust whole-frame outline route requires a seeded semantic viewport (bounded)"),
				"outline admission must reject oversized viewports before Rust line submission");
		assertTrue(gameRenderer.contains("RustGalWorldPrimitiveRenderer.enqueueWorldBackground"));
		assertTrue(worldRenderer.contains("Rust whole-frame background route is unavailable"),
			"an active Rust shell must not silently omit the background when its route is unavailable");
		assertTrue(worldRenderer.contains("Rust whole-frame background route requires a seeded semantic viewport (bounded)"),
			"background admission must reject oversized viewports before Rust submission");
		assertTrue(worldRenderer.contains("Rust world semantic prime requires a seeded bounded semantic viewport"),
			"semantic frame priming must reject invalid viewports before downstream extraction");
		assertTrue(worldRenderer.contains("Rust whole-frame weather route rejected visible semantic work"),
			"visible rejected weather work must not silently fall through to the absent Java frame graph");
		assertTrue(worldRenderer.contains("Rust whole-frame cloud route rejected visible semantic work"),
			"visible rejected cloud work must not silently fall through to the absent Java frame graph");
			assertTrue(gameRenderer.contains("FogParameters wholeFrameFog"),
				"whole-frame background must receive the CPU fog semantic rather than raw sky color");
			assertFalse(worldRenderer.contains("level.getSkyColor(camera.getPosition(), partialTick)"),
				"Rust whole-frame background must not substitute raw sky color for the extracted fog semantic");
			assertTrue(gameRenderer.contains("levelRenderer.enqueueRustGalBlockBreakingCracks"));
		assertTrue(gameRenderer.contains("levelRenderer.enqueueRustGalWorldBorder"));
		assertTrue(levelRenderer.contains("RustGalWorldPrimitiveRenderer.enqueueWorldBorder"));
		assertTrue(levelRenderer.contains("reason=no-valid-destroy-progress"),
			"normal OpenGL must draw nothing when no active valid destroy-progress state exists");
		assertTrue(levelRenderer.contains("Rust OpenGL block-breaking crack overlay was selected with valid semantic requests but submitted no work"),
			"Rust submission/validation failure must be explicit instead of falling back to Java in the same frame");
		assertFalse(levelRenderer.contains("java-opengl-retained-after-empty-rust"),
			"normal OpenGL must not use same-frame Java fallback after selecting the Rust crack route");
			String borderRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/WorldBorderRenderer.java"));
			String terrainParticle = Files.readString(Path.of("src/main/java/net/minecraft/client/particle/TerrainParticle.java"));
			assertTrue(borderRenderer.contains("RustGalWorldPrimitiveRenderer.enqueueWorldBorder"));
			assertTrue(worldRenderer.contains("enqueueBlockBreakingCracks"));
			assertTrue(worldRenderer.contains("renderOpenGlBlockBreakingCracks"));
			assertTrue(worldRenderer.contains("state.blockState.isAir()"),
				"real crack extraction must not synthesize full-cube crack quads for stale air states");
			assertTrue(worldRenderer.contains("Rust whole-frame crack route rejected visible semantic work"),
				"visible rejected crack work must not be silently omitted while Rust owns presentation");
			assertTrue(worldRenderer.contains("Rust whole-frame crack route requires a seeded semantic viewport (bounded)"),
				"crack admission must reject oversized viewports before Rust submission");
			assertTrue(worldRenderer.contains("Rust whole-frame crack route received incomplete copied break state"),
				"crack admission must reject incomplete copied break states before shape access");
			assertTrue(worldRenderer.contains("enqueueWorldBorder"));
			assertTrue(worldRenderer.contains("enqueueWorldBackground"));
			assertTrue(worldRenderer.contains("reloadWorldAssets"));
			assertTrue(worldRenderer.contains("enqueueBlockDisplay"));
			assertTrue(worldRenderer.contains("currentBlockDisplayRoute()"));
		assertTrue(worldRenderer.contains("enqueueFallingBlock"));
		assertTrue(worldRenderer.contains("currentFallingBlockRoute()"));
		assertTrue(worldRenderer.contains("requires a seeded bounded world primitive frame"));
		assertTrue(worldRenderer.contains("enqueuePistonMovingBlock"));
		assertTrue(worldRenderer.contains("currentPistonMovingBlockRoute()"));
		assertTrue(worldRenderer.contains("enqueuePrimedTntBlock"));
		assertTrue(worldRenderer.contains("isPrimedTntMeshEligible"));
		assertTrue(worldRenderer.contains("BlockSubmitSource.PRIMED_TNT"));
		assertTrue(worldRenderer.contains("Primed TNT extraction failed after Rust route selection"));
		assertTrue(worldRenderer.contains("recordWorldMeshSubmittedWorkIdentity"));
		assertTrue(worldRenderer.contains("DeterministicCameraCapture.recordSubmittedWorkIdentity(family, identity)"),
			"deterministic gameplay captures must receive the same semantic mesh submission identities as benchmark rows");
			assertTrue(worldRenderer.contains("MovingBlockSubmitSource.FALLING_BLOCK"));
			assertTrue(worldRenderer.contains("MovingBlockSubmitSource.PISTON"));
			String fallingRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/entity/FallingBlockRenderer.java"));
			String pistonRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/blockentity/PistonHeadRenderer.java"));
			assertTrue(fallingRenderer.contains("submitMovingBlockSemantic(")
				&& pistonRenderer.contains("submitMovingBlockSemantic("),
				"falling-block and piston producers must use the explicit semantic moving-block callback");
			assertTrue(worldRenderer.contains("SubmitNodeStorage.BlockSubmitSource.BLOCK_DISPLAY"));
			assertTrue(worldRenderer.contains("blockSubmit.tintPos()"),
				"BlockDisplay extraction must retain the real copied display position for biome-tint semantics");
			assertTrue(worldRenderer.contains("Minecraft.getInstance().level"),
				"BlockDisplay extraction must resolve eligible model tint from the semantic client world, not an empty placeholder");
			String displayRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/entity/DisplayRenderer.java"));
			assertTrue(displayRenderer.contains("BlockPos.containing(blockDisplayEntityRenderState.x"),
				"BlockDisplay submit must carry its render-state position before Rust route selection");
			assertTrue(worldRenderer.contains("RenderShape.MODEL"));
			assertTrue(worldRenderer.contains("specialBlockModelRenderer().get().hasRenderer(blockState.getBlock())"));
			assertTrue(worldRenderer.contains("special-renderer-semantic-submitted")
				&& worldRenderer.contains("Special block renderers")
				&& worldRenderer.contains("already") && worldRenderer.contains("semantic submission"),
				"special block displays must be admitted through their copied semantic model submission rather than rejected");
			String submitNodesSource = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/SubmitNodeCollection.java"));
		assertTrue(submitNodesSource.contains("if (!specialRenderer")
				&& submitNodesSource.contains("blockRoute.usesRustWholeFrameVulkan()")
				&& submitNodesSource.contains("no corresponding mesh instance"),
				"special semantic blocks must not leave an unpaired ordinary/block-display marker in Rust coverage");
		String captureSource = Files.readString(Path.of("src/main/java/net/minecraft/client/dev/DeterministicCameraCapture.java"));
		assertTrue(captureSource.contains("expectedModelMeshDiagnosticTextureId()")
				&& captureSource.contains("\"model\".equals(diagnostic.provenance())"),
				"special model fixtures must require their Rust model execution receipt");
		assertTrue(worldRenderer.contains("WorldMeshAssetRecord"));
		assertTrue(worldRenderer.contains("WorldMeshInstanceRecord"));
		assertTrue(worldRenderer.contains("\"block-model\".equals(producer)"),
				"completed block-model mesh instances must produce a Rust execution receipt");
		assertTrue(worldRenderer.contains("PENDING_BLOCK_MODEL_MESH_KEYS.contains(instance.meshKey())"),
				"block-model execution receipts must survive coarse producer-list indexing");
		assertTrue(worldRenderer.contains("PENDING_MODEL_MESH_KEYS.contains(instance.meshKey())"),
				"ordinary model execution receipts must survive coarse producer-list indexing");
		assertTrue(worldRenderer.contains("hasModelMeshDiagnosticForKey(instance.meshKey())"),
				"model completion must retain a key-based receipt across frame rollover");
		assertTrue(worldRenderer.contains("WorldFeatureCoverageRecord"));
		assertTrue(worldRenderer.contains("enqueueWorldFeatureCoverage"));
		assertTrue(worldRenderer.contains("queuedModelPartSubmits")
			&& worldRenderer.contains("reconcileFeatureCoverage(\"model-part\", coverage.modelPartSubmits(), queuedModelPartSubmits)"),
			"Rust-owned ModelPart mesh producers must reconcile their copied coverage count");
		assertTrue(worldRenderer.contains("reconcileFeatureCoverage")
			&& worldRenderer.contains("reported < 0 || queued < 0")
			&& !worldRenderer.contains("Math.max(0, coverage.modelPartSubmits() - queuedModelPartSubmits)"),
			"coverage reconciliation must reject malformed producer counts without hiding Rust-owned receipts");
		String submitNodes = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/SubmitNodeCollection.java"));
		assertTrue(submitNodes.contains("int[] vertexColors")
			&& submitNodes.contains("int[] vertexLights")
			&& submitNodes.contains("vertices.size() % 4 == 0"),
			"admitted procedural quads must preserve per-vertex modulation and per-quad light boundaries");
		assertTrue(worldRenderer.contains("queuedBlockModelSubmits")
			&& worldRenderer.contains("reconcileFeatureCoverage(\"block-model\", coverage.blockModelSubmits(), queuedBlockModelSubmits)")
			&& worldRenderer.contains("queuedOrdinaryBlockSubmits")
			&& worldRenderer.contains("reconcileFeatureCoverage(\"ordinary-block\", coverage.ordinaryBlockSubmits(), queuedOrdinaryBlockSubmits)")
			&& worldRenderer.contains("blockSubmit.source() == SubmitNodeStorage.BlockSubmitSource.ORDINARY")
			&& worldRenderer.contains("PendingMeshProducer.ORDINARY_BLOCK")
			&& worldRenderer.contains("queuedItemSubmits")
			&& worldRenderer.contains("reconcileFeatureCoverage(\"item\", coverage.itemSubmits(), queuedItemSubmits)"),
			"Rust-owned block-display, block-model, and item mesh producers must reconcile their copied coverage counts");
		assertTrue(worldRenderer.contains("int flameSubmits = WorldRenderRoutePolicy.currentEntityFlameRoute()"),
			"Rust-owned entity flames must be removed from unsupported-family coverage after semantic collection");
		assertTrue(levelRenderer.contains("WorldFeatureCoverageCollector"));
		assertTrue(levelRenderer.contains("submitSemantic("));
		assertTrue(levelRenderer.contains("boolean arrows"),
			"selected-source coverage must receive the same Arrow admission flag as the real Rust traversal");
		int coverageStart = levelRenderer.indexOf("private void collectUnsupportedWholeFrameFeatures");
		int coverageEnd = levelRenderer.indexOf("private static final class WorldFeatureCoverageCollector", coverageStart);
		String coverageMethod = levelRenderer.substring(coverageStart, coverageEnd);
		assertFalse(coverageMethod.contains("renderAllFeatures()"),
			"whole-frame feature coverage must collect semantic counts without issuing Java feature draws");
		assertTrue(coverageMethod.contains("arrows && entityRenderState instanceof net.minecraft.client.renderer.entity.state.ArrowRenderState"));
		assertTrue(coverageMethod.contains("entityRenderState instanceof net.minecraft.client.renderer.entity.state.ZombieRenderState"));
		String entityDispatcher = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/entity/EntityRenderDispatcher.java"));
		String blockEntityDispatcher = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/blockentity/BlockEntityRenderDispatcher.java"));
		assertTrue(entityDispatcher.contains("void submitSemantic("));
		assertTrue(blockEntityDispatcher.contains("void submitSemantic("));
		assertTrue(entityDispatcher.contains("captureIrisRenderState"));
		assertTrue(blockEntityDispatcher.contains("captureIrisRenderState"));
		assertTrue(worldRenderer.contains("flushPendingWorldMeshAssets"));
		assertTrue(worldRenderer.contains("flushPendingWorldBorderAssets"));
		assertTrue(worldRenderer.contains("textures/misc/forcefield.png"));
		assertTrue(worldRenderer.contains("WorldCrackQuadRecord"));
		assertTrue(worldRenderer.contains("WorldBorderQuadRecord"));
		assertTrue(worldRenderer.contains("STRATUM_WORLD_BLOCK_BREAKING_CRACK"));
		assertTrue(worldRenderer.contains("STRATUM_WORLD_BORDER"));
		assertTrue(worldRenderer.contains("CRACK_BLEND_MULTIPLY"));
		assertTrue(worldRenderer.contains("BORDER_BLEND_OVERLAY"));
		assertTrue(worldRoutePolicy.contains("DISABLED"));
		assertTrue(worldRoutePolicy.contains("JAVA_COMPATIBILITY"));
		assertTrue(worldRoutePolicy.contains("RUST_OPENGL_BORROWED_CONTEXT"));
		assertTrue(worldRoutePolicy.contains("RUST_VULKAN_WHOLE_FRAME"));
		assertTrue(worldRenderer.contains("WorldRenderRoutePolicy.currentBackgroundRoute()"));
		assertTrue(worldRenderer.contains("WorldRenderRoutePolicy.currentWorldBorderRoute()"));
		assertTrue(worldRoutePolicy.contains("currentStaticTerrainRoute()"));
		assertTrue(gameRenderer.contains("enqueueRustGalStaticTerrainForWholeFrame"));
		assertTrue(gameRenderer.contains("enqueueRustGalWeatherForWholeFrame"));
		assertTrue(gameRenderer.contains("enqueueRustGalSkyForWholeFrame"));
		assertTrue(levelRenderer.contains("weatherEffectRenderer.extractRenderState"));
		assertTrue(levelRenderer.contains("weatherRenderState.reset()"));
		assertTrue(levelRenderer.contains("RustGalWorldPrimitiveRenderer.enqueueWorldWeather"));
		assertTrue(levelRenderer.contains("skyRenderer.extractRenderState"));
		assertTrue(levelRenderer.contains("RustGalWorldPrimitiveRenderer.enqueueWorldSky"));
		assertTrue(worldRenderer.contains("WorldBackgroundRecord initialBackground = pendingBackground")
			&& worldRenderer.contains("pendingBackground = initialBackground"),
			"sky semantic admission must roll back scalar background state together with failed celestial quads");
		assertTrue(worldRenderer.contains("Rust whole-frame sky route requires a seeded bounded semantic viewport"),
			"sky semantic admission must reject unseeded or oversized viewports before queuing celestial geometry");
		assertTrue(worldRenderer.contains("Rust whole-frame sky route rejected visible semantic work"),
			"visible rejected sky work must not silently fall through to the absent Java frame graph");
		assertTrue(levelRenderer.contains("Rust whole-frame background route is unavailable while Rust owns presentation"),
			"disabled sky must not reopen Java sky rendering after Rust presentation begins");
		assertTrue(levelRenderer.contains("Rust whole-frame sky route is unavailable while Vulkan is selected"),
			"selected Vulkan sky must fail closed before the Java frame-graph renderer");
		assertTrue(levelRenderer.contains("this.cullTerrain(camera, frustum, this.minecraft.player.isSpectator())"));
		assertTrue(levelRenderer.contains("this.rustGalWholeFrameTerrainSource.enqueue("));
		assertFalse(levelRenderer.contains("this.renderer.enqueueRustGalStaticTerrain(camera)"),
			"whole-frame Vulkan terrain must not source visibility from Sodium's GL renderer");
		assertTrue(sodiumWorldRenderer.contains("enqueueRustGalStaticTerrain"));
		assertTrue(renderSectionManager.contains("RustGalTerrainRenderer.acceptChunkBuildOutput(chunkBuildOutput)"));
		assertTrue(wholeFrameTerrainSource.contains("new ChunkBuilder(level, ChunkMeshFormats.COMPACT, false, semanticMeshWorkerCount())"),
			"the direct vanilla terrain consumer requires Frozen's baked-RGB compact mesh contract");
		assertFalse(wholeFrameTerrainSource.contains("new ChunkBuilder(level, ChunkMeshFormats.COMPACT, true, semanticMeshWorkerCount())"),
			"separate-AO compact meshes are not admissible to the direct vanilla Rust terrain shader");
		assertTrue(wholeFrameTerrainSource.contains("RustGalTerrainRenderer.acceptWholeFrameChunkBuildOutput(output)"));
		assertTrue(wholeFrameTerrainSource.contains("output.destroy()"),
			"the direct CPU source must release native intermediate mesh buffers after VulkanicGAL copies them");
		assertFalse(wholeFrameTerrainSource.contains("MAX_RESIDENT_RENDERABLE_SECTIONS"),
			"Rust-owned Vulkan terrain must not conceal incomplete coverage behind a resident-section cap");
		assertTrue(wholeFrameTerrainSource.contains("cpu-source-window-evicted"));
		assertFalse(wholeFrameTerrainSource.contains("RenderDevice"));
		assertFalse(wholeFrameTerrainSource.contains("RenderRegion"));
		assertFalse(wholeFrameTerrainSource.contains("WorldRenderingSettings"));
		assertTrue(blockRenderer.contains("!net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()"),
			"the compact whole-frame source must not read Iris material-map state while producing semantic terrain");
		assertTrue(terrainRenderer.contains("WorldRenderRoutePolicy.currentStaticTerrainRoute().usesRustWholeFrameVulkan()"));
		int genericBuildEntry = terrainRenderer.indexOf("public static void acceptChunkBuildOutput(ChunkBuildOutput output)");
		int backendCheck = terrainRenderer.indexOf("VulkanicAPI.isVulkanBackendSelected()", genericBuildEntry);
		int routeCheck = terrainRenderer.indexOf("currentStaticTerrainRoute().usesRustWholeFrameVulkan()", genericBuildEntry);
		int irisLayout = terrainRenderer.indexOf("TerrainMeshLayout.activeIrisCompatible()", genericBuildEntry);
		assertTrue(genericBuildEntry >= 0 && backendCheck > genericBuildEntry && routeCheck > backendCheck && irisLayout > routeCheck,
			"generic terrain build admission must reject unadmitted Vulkan before any Iris layout lookup");
		assertTrue(terrainRenderer.contains("enqueueWholeFrameTerrainSections"));
		assertTrue(terrainRenderer.contains("TerrainMeshLayout.compact()"));
		assertTrue(terrainRenderer.contains("recordSubmittedWorkIdentity(\"sodium-terrain\", identity)"),
			"the direct CPU terrain producer must participate in the shared settled-work family");
		assertTrue(terrainRenderer.contains("DefaultTerrainRenderPasses.SOLID"));
		assertTrue(terrainRenderer.contains("DefaultTerrainRenderPasses.CUTOUT"));
			assertTrue(terrainRenderer.contains("DefaultTerrainRenderPasses.TRANSLUCENT"));
			assertTrue(terrainRenderer.contains("ChunkSectionLayer.TRANSLUCENT"));
			assertTrue(terrainRenderer.contains("PRIMITIVE_KIND_GENERIC_FLUID"),
				"non-water fluid terrain must retain an explicit generic-fluid semantic lane");
			assertTrue(fluidRenderer.contains(": NativeSectionMeshBuilder.PRIMITIVE_KIND_GENERIC_FLUID")
				&& fluidRenderer.contains("this.rustGalPrimitiveKind != NativeSectionMeshBuilder.PRIMITIVE_KIND_UNSUPPORTED_FLUID"),
				"the fluid callsite must retain non-water atlas sprites instead of classifying them as omitted work");
			assertTrue(terrainRenderer.contains("without a semantic material route"),
				"unknown fluid metadata must fail closed rather than being omitted from a Rust frame");
			assertTrue(terrainRenderer.contains("MATERIAL_ID_WATER_TRANSLUCENT"),
				"built-in water must carry an explicit semantic material identity");
		assertTrue(terrainRenderer.contains("acceptChunkSortOutput"));
		assertTrue(renderSectionManager.contains("RustGalTerrainRenderer.acceptChunkSortOutput(sortOutput)"));
		assertTrue(terrainRenderer.contains("registeredAtlasGeneration"));
		assertTrue(terrainRenderer.contains("atlasTextureUpdatePayload(waterTextureBinding(WorldRenderRoutePolicy.currentStaticTerrainRoute()))"));
		assertTrue(terrainRenderer.contains("the same semantic atlas cannot acquire a different"),
			"eager and section-triggered terrain atlas publication must retain one explicit sampled-row contract");
		assertFalse(terrainRenderer.substring(terrainRenderer.indexOf("static List<VulkanicGalBridge.WorldMeshTextureAssetRecord> atlasTextureUpdatePayload(WaterTextureBinding waterBinding)"))
			.contains("WORLD_MESH_TEXTURE_COORDINATE_ORIGIN_MINECRAFT_TOP_LEFT"),
			"section-triggered terrain atlas publication must not invert rows relative to eager whole-frame publication");
			assertTrue(terrainRenderer.contains("vertex.colorArgb()"));
			assertTrue(terrainRenderer.contains("vertex.light()"));
			assertTrue(terrainRenderer.contains("fnv64Int(hash, vertex.midBlockPacked())"),
				"the mesh generation must cover every semantic field retained by the Rust occupancy source");
			assertTrue(terrainRenderer.contains("WorldMeshSectionRecord section"));
		assertTrue(terrainRenderer.contains("removeSection(int x, int y, int z, String reason)"));
		assertTrue(terrainRenderer.contains("TerrainDiagnostics"));
		assertTrue(terrainRenderer.contains("DeterministicCameraCapture.recordSubmittedWorkIdentity(\"static-terrain\""));
		assertTrue(terrainRenderer.contains("recordVisibleSubmissionIdentity"),
			"static-terrain readiness must identify the section/layer/generation that actually reached Rust");
		assertTrue(terrainRenderer.contains("staticTerrainExecutionSnapshot"),
			"static-terrain captures must distinguish completed static-terrain execution from unrelated world work");
		assertTrue(deterministicCapture.contains("visible-submission-identities"),
			"off-camera streaming must not replace visible semantic submission stability as the capture gate");
		assertTrue(deterministicCapture.contains("rustGalStaticTerrainDiagnostics"));
		assertTrue(deterministicCapture.contains("waiting-for-post-setup-static-terrain-execution"),
			"a static-terrain screenshot must wait for an execution after fixture setup, not merely asset registration");
		assertTrue(deterministicCapture.contains("rustGalStaticTerrainExecution"),
			"static-terrain screenshot acknowledgements must carry route-specific execution correlation");
		assertTrue(deterministicCapture.contains("appendStaticTerrainExecutionCorrelation(json, 2)"),
			"Rust final-output captures must retain static-terrain correlation instead of only the normal screenshot path");
		int finalOutputCaptureStart = deterministicCapture.indexOf("private static boolean captureWholeFrameFinalOutput()");
		int finalOutputCaptureEnd = deterministicCapture.indexOf("public static void beforeTick", finalOutputCaptureStart);
		String finalOutputCapture = deterministicCapture.substring(finalOutputCaptureStart, finalOutputCaptureEnd);
		assertTrue(finalOutputCapture.contains("appendStaticTerrainAtlasReceipt(json, 2)"),
			"Rust final-output captures must retain the atlas receipt for their exact terrain frame");
		assertTrue(finalOutputCapture.contains("appendStaticTerrainTextureProbeReceipt(json, 2)"),
			"Rust final-output captures must retain the projected palette probes required to validate that image");
		assertTrue(finalOutputCapture.contains("attachment-sky-fog.json")
			&& finalOutputCapture.contains("skyFogReceipt")
			&& finalOutputCapture.contains("Files.copy(skyFogSource, skyFogReceiptPath"),
			"a selected final-output image must retain the exact Rust-owned sky/fog inputs rather than a later rolling receipt");
		assertTrue(deterministicCapture.contains("normalRouteFinalOutputCapture")
			&& deterministicCapture.contains("selectedSourceCaptureRequested() || RUST_FINAL_OUTPUT_EVERY_POSE"),
			"the opt-in normal-route diagnostic must retain the exact Rust final image without requiring selected-source execution");
		assertTrue(frameCoordinator.contains("normalRouteFinalOutputCaptureRequested()"),
			"the coordinator must acknowledge the normal presented Rust frame for the opt-in exact-final diagnostic");
		assertTrue(deterministicCapture.contains("requiredRustSourceExecutionDir"),
			"selected-source captures must wait for Rust execution evidence, not only normal-graph preparation");
		assertTrue(deterministicCapture.contains("selected-source-execution-frame-*.json"),
			"the capture gate must require the bounded Rust-written source execution record");
		assertTrue(deterministicCapture.contains("sourceExecutionReceiptHasVisibleWorldWork"),
			"selected-source capture readiness must recognize real world work independent of its producer family");
		assertTrue(deterministicCapture.contains("readJsonLongField(json, \"lod_instances\", 0L) > 0L"),
			"Distant Horizons-only selected-source frames must not be misclassified as zero work");
		assertTrue(graphicsHarness.contains("--world-static-terrain-scenario"));
		assertTrue(rustWorldFrontend.contains("let sky_fog_receipt_name = \"attachment-sky-fog.json\""),
			"the Rust producer receipt must be bounded rather than accumulate once per readiness submission");
		assertTrue(rustWorldFrontend.contains("ops.extend(gui_ops);\n        if let Some(capture) = gameplay_attachment_capture.as_mut()")
			&& rustWorldFrontend.contains("append_normal_presented_output(gal, &mut ops, frame_target)"),
			"a normal final-output readback must occur after the Rust GUI replay, from the acquired presentation target");
		assertTrue(graphicsHarness.contains("static_terrain_workload_complete"));
		assertTrue(renderSectionManager.contains("RustGalTerrainRenderer.removeSection(x, y, z, \"section-removed\")"));
		assertTrue(worldRenderer.contains("DIRTY_WORLD_MESH_TEXTURES"));
		assertTrue(worldRenderer.contains("DIRTY_WORLD_MESH_SORTED_INDICES"));
		assertTrue(worldRenderer.contains("registerStaticTerrainSortedIndex"));
		assertTrue(bridge.contains("record WorldMeshSortedIndexRecord"));
		assertTrue(worldRenderer.contains("dirtyWorldMeshTextureAssetsLocked("));
		assertTrue(worldRenderer.contains("dirtyWorldMeshSortedIndicesLocked("));
		assertTrue(worldRenderer.contains("removeStaticTerrainMeshAsset"));
			assertTrue(rustWorldFrontend.contains("self.mesh_texture_assets.insert(texture_id, texture);")
					&& rustWorldFrontend.contains("destroy_mesh_texture_resources_for_ids(gal, &replaced_texture_ids);"),
				"incremental mesh updates must preserve previously uploaded semantic texture assets");
		assertTrue(rustTerrainFrontend.contains("Static chunk-terrain frontend boundary"));
		assertFalse(worldRenderer.contains("CRACK_DISABLED_CONTROL ||"));
		assertTrue(worldRenderer.contains("shape.forAllEdges"));
		assertFalse(worldRenderer.contains("CommandOp"));
		assertFalse(worldRenderer.contains("Vk"));
		assertFalse(worldRenderer.matches("(?s).*\\bGL_[A-Z0-9_]+.*"));
			assertTrue(rustFfi.contains("FfiWorldLineSegmentRequest"));
			assertTrue(rustFfi.contains("FfiWorldCrackQuadRequest"));
			assertTrue(rustFfi.contains("FfiWorldBorderQuadRequest"));
			assertTrue(rustFfi.contains("FfiWorldBackgroundRequest"));
			assertTrue(rustFfi.contains("FfiWorldBorderAssetUpdateRequest"));
			assertTrue(rustFfi.contains("FfiWorldMeshAssetRecord"));
			assertTrue(rustFfi.contains("FfiWorldMeshInstanceRecord"));
			assertTrue(rustFfi.contains("mattmc_vulkanic_gal_world_mesh_update_assets"));
			assertFalse(rustFfi.contains("BlockDisplay"));
			assertFalse(rustFfi.contains("BlockSubmit"));
		assertTrue(rustFfi.contains("decode_world_border_asset_update"));
		assertTrue(rustFfi.contains("decode_whole_frame_submit"));
		assertTrue(rustFfi.contains("context.world_primitive_frontend"));
			assertTrue(rustWorldFrontend.contains("WORLD_LINE_VERTEX_SHADER_VULKAN"));
			assertTrue(rustWorldFrontend.contains("PrimitiveTopology::Triangles"));
		assertTrue(rustWorldFrontend.contains("WorldCrackQuadRequest"));
		assertTrue(rustWorldFrontend.contains("WorldBorderQuadRequest"));
		assertTrue(rustWorldFrontend.contains("BlendMode::Multiply"));
		assertTrue(rustWorldFrontend.contains("BlendMode::Overlay"));
		assertTrue(rustWorldFrontend.contains("forcefield.png"));
		assertTrue(rustWorldFrontend.contains("apply_world_border_asset_update"));
		assertTrue(rustWorldFrontend.contains("WorldPrimitiveFrontend"));
		assertTrue(rustWorldFrontend.contains("submit_whole_frame"));
			assertTrue(rustWorldFrontend.contains("unsupported_feature"));
			assertTrue(terrainParticle.contains("private boolean alphaTested;"),
				"TerrainParticle must track alpha-test semantics separately from Iris opaque particle-layer routing");
			assertTrue(terrainParticle.contains("|| type == net.minecraft.client.renderer.chunk.ChunkSectionLayer.CUTOUT_MIPPED"),
				"cutout and cutout-mipped block particles must be recognized before Rust material submission");
			assertTrue(terrainParticle.contains("this.alphaTested ? net.vulkanic.bridge.VulkanicGalBridge.ParticleSurface.TERRAIN_CUTOUT")
				&& terrainParticle.contains("this.isOpaque ? net.vulkanic.bridge.VulkanicGalBridge.ParticleSurface.TERRAIN_OPAQUE")
				&& terrainParticle.contains(": net.vulkanic.bridge.VulkanicGalBridge.ParticleSurface.TERRAIN_TRANSLUCENT"),
				"terrain callsites must distinguish cutout, opaque and translucent block semantics before Rust policy selection");
		}

	@Test
	void rustParticleAdmissionRejectsUnsupportedQuadLayersWithoutSilentDrop() throws Exception {
		String particleRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/feature/ParticleFeatureRenderer.java"));
		String quadState = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/state/QuadParticleRenderState.java"));
		String worldRenderer = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		assertTrue(particleRenderer.contains("quad.rustGalUnsupportedLayerCount() > 0"),
			"Rust particle admission must reject QuadParticle layers outside the copied atlas contract");
		assertTrue(particleRenderer.contains("renderer instanceof QuadParticleRenderState quad"),
			"particle admission must inspect semantic QuadParticle state rather than only callback type");
		assertTrue(particleRenderer.contains("rust-vulkan-unavailable"),
			"unsupported particle layers must remain explicit unavailable work");
		assertTrue(quadState.contains("int unsupportedLayers = this.rustGalUnsupportedLayerCount()")
			&& quadState.contains("atlas layer(s) without a bounded copied texture snapshot"),
			"particle semantic admission must fail before queueing a partial Rust prefix");
		assertTrue(quadState.contains("Rust whole-frame particle atlas admission changed while staging"),
			"a texture snapshot changing during staging must not masquerade as complete Rust work");
		assertTrue(quadState.contains("ensureParticleAtlasAvailable")
			&& worldRenderer.contains("public static boolean ensureParticleAtlasAvailable"),
			"all distinct particle atlases must be preflighted before any Rust quad is queued");
		assertTrue(quadState.contains("hasMaterialQuadCapacity(this.particleCount)")
			&& worldRenderer.contains("public static boolean hasMaterialQuadCapacity"),
			"particle batches must reserve bounded Rust material capacity before emission");
		assertTrue(worldRenderer.contains("texture instanceof DynamicTexture")
			&& worldRenderer.contains("registerDynamicTextureAsset(atlasLocation, textureId)"),
			"dynamic resource-pack particle textures must use the copied Rust asset uploader");
		assertTrue(worldRenderer.contains("ensureBoundedParticleViewportLocked()")
			&& worldRenderer.contains("Rust particle route requires a seeded bounded semantic viewport"),
			"particle producers must reject missing or oversized viewports before queueing semantic quads");
	}

	@Test
	void texturedBillboardsAdmitResourcePackPngsBeforeDynamicTextureFallback() throws Exception {
		String worldRenderer = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		int enqueue = worldRenderer.indexOf("private static boolean enqueueTexturedQuadForMode");
		int helper = worldRenderer.indexOf("private static boolean registerSemanticTextureAsset", enqueue);
		assertTrue(enqueue >= 0 && helper > enqueue,
			"textured billboard admission must use a dedicated semantic asset helper");
		assertTrue(worldRenderer.contains("Rust textured billboard requires a seeded bounded world primitive frame"),
			"textured billboard admission must reject oversized viewports before queueing material work");
		String enqueueBody = worldRenderer.substring(enqueue, helper);
		assertTrue(enqueueBody.contains("registerSemanticTextureAsset(textureIdentity, textureId, \"textured-billboard\")"),
			"generic billboard textures must admit resource-manager payloads before dynamic lookup");
		assertTrue(enqueueBody.indexOf("PENDING_MATERIAL_QUADS.ensureCapacityFor(1)")
			< enqueueBody.indexOf("registerSemanticTextureAsset(textureIdentity, textureId, \"textured-billboard\")"),
			"textured billboard assets must be admitted only after material capacity preflight");
		assertTrue(enqueueBody.contains("translucent ? DEPTH_POLICY_TEST_NO_WRITE : DEPTH_POLICY_TEST_WRITE"),
			"translucent billboard quads must use explicit depth-test/no-write semantics");
		String helperBody = worldRenderer.substring(helper,
			worldRenderer.indexOf("\n\t/** Reads a resource-pack payload", helper));
		assertTrue(helperBody.indexOf("readTexturePayloadForResource(identity)")
			< helperBody.indexOf("registerDynamicTextureAsset(identity, textureId)"),
			"resource-pack PNGs must precede the dynamic/atlas fallback without sharing Java handles");
		assertTrue(helperBody.contains("minecraftModelTextureAsset(textureId, resourcePayload)"),
			"resource-pack billboard bytes must be published as an explicit Rust texture asset");
	}

	@Test
	void sheepWoolInvisibleGlowUsesOutlineOnlyRustMesh() throws Exception {
		String sheepWool = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/entity/layers/SheepWoolLayer.java"));
		int glow = sheepWool.indexOf("sheepRenderState.appearsGlowing()");
		int outlineOnly = sheepWool.indexOf("enqueueStandaloneModelMeshOutlineOnly", glow);
		int normalMesh = sheepWool.indexOf("enqueueStandaloneModelMesh(", glow);
		assertTrue(glow >= 0 && outlineOnly > glow,
			"invisible glowing sheep wool must enter the explicit Rust outline-only mesh ABI");
		assertTrue(normalMesh < 0 || normalMesh > sheepWool.indexOf("} else {", glow),
			"the invisible glowing wool branch must not submit a normal Rust color mesh");
	}

	@Test
	void slimeInvisibleGlowUsesOutlineOnlyRustMesh() throws Exception {
		String slimeLayer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/entity/layers/SlimeOuterLayer.java"));
		int glow = slimeLayer.indexOf("boolean bl = slimeRenderState.appearsGlowing() && slimeRenderState.isInvisible");
		int outlineOnly = slimeLayer.indexOf("enqueueStandaloneModelMeshOutlineOnly", glow);
		assertTrue(glow >= 0 && outlineOnly > glow,
			"invisible glowing slime must enter the explicit Rust outline-only mesh ABI");
	}

	@Test
	void entityLayerBlockAndOutlineRoutesRespectRustPresenterShellOwnership() throws Exception {
		String mushroomCow = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/entity/layers/MushroomCowMushroomLayer.java"));
		String snowGolem = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/entity/layers/SnowGolemHeadLayer.java"));
		String slime = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/entity/layers/SlimeOuterLayer.java"));
		String ears = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/entity/layers/Deadmau5EarsLayer.java"));
		for (String source : List.of(mushroomCow, snowGolem, slime, ears)) {
			assertTrue(source.contains("RustGalVulkanWholeFrameMode.enabled()"),
				"entity layer must treat the Rust presenter shell as ownership");
		}
		assertTrue(mushroomCow.contains("submitBlockDisplaySemantic")
			&& snowGolem.contains("submitBlockDisplaySemantic")
			&& slime.contains("enqueueStandaloneModelMeshOutlineOnly")
			&& ears.contains("enqueueStandaloneModelMesh"),
			"each entity layer must retain its explicit Rust semantic producer");
	}

	@Test
	void rustParticleRouteDoesNotRetainJavaParticleCallbacks() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/SubmitNodeCollection.java"));
		int method = source.indexOf("void submitParticleGroup");
		int rustGate = source.indexOf("currentMaterialRoute().usesRustWholeFrameVulkan()", method);
		int callbackAppend = source.indexOf("particleGroupRenderers.add", method);
		int returnBeforeAppend = source.indexOf("return;", rustGate);
		assertTrue(method >= 0 && rustGate > method && returnBeforeAppend > rustGate && callbackAppend > returnBeforeAppend,
			"Rust particle ownership must discard Java callbacks before the legacy callback list is appended");
		assertTrue(source.contains("recordUnsupportedParticleGroup()"),
			"discarded unsupported particle callbacks must remain diagnostically visible");
		String quadState = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/state/QuadParticleRenderState.java"));
		assertTrue(quadState.contains("submitParticleGroupSemantic(") && quadState.contains("submitParticleGroup("),
			"quad particle producers must identify Rust semantic submission while preserving compatibility");
	}

	@Test
	void rustWholeFrameExtractsNonQuadParticleFamiliesThroughSemanticModels() throws Exception {
		String engine = Files.readString(Path.of("src/main/java/net/minecraft/client/particle/ParticleEngine.java"));
		String renderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
		assertTrue(engine.contains("enqueueRustGalModelParticles")
			&& engine.contains("particleGroup instanceof QuadParticleGroup")
			&& engine.contains("ItemPickupParticleGroup")
			&& engine.contains("ElderGuardianParticleGroup")
			&& engine.contains("state.submitSemantic(submitNodeStorage, cameraRenderState)"),
			"Rust whole-frame particle extraction must submit item-pickup and elder-guardian states through the semantic collector");
		assertTrue(engine.contains("MAX_RUST_MODEL_PARTICLE_GROUPS = 1_024")
			&& engine.contains("Rust whole-frame model-particle group bound exceeded"),
			"specialized model-particle extraction must be bounded before semantic submission");
		assertTrue(engine.contains("MAX_RUST_MODEL_PARTICLE_INSTANCES = 4_096")
			&& engine.contains("Rust whole-frame model-particle instance bound exceeded"),
			"each specialized model-particle group must have a bounded instance admission");
		assertTrue(engine.contains("has no semantic collector for ")
			&& engine.contains("recordUnsupportedParticleGroup()"),
			"unknown non-quad particle families must fail closed instead of being silently omitted");
		String particleState = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/state/ParticleGroupRenderState.java"));
		assertTrue(particleState.contains("submitSemantic("),
			"particle-group states must expose an explicit semantic submission contract");
		String itemPickup = Files.readString(Path.of("src/main/java/net/minecraft/client/particle/ItemPickupParticleGroup.java"));
		assertTrue(itemPickup.contains("entityRenderDispatcher.submitSemantic("),
			"item-pickup particle model submits must stay inside the semantic entity boundary");
		String elderGuardian = Files.readString(Path.of("src/main/java/net/minecraft/client/particle/ElderGuardianParticleGroup.java"));
		assertTrue(elderGuardian.contains("submitModelSemanticTexture(")
			&& elderGuardian.contains("ElderGuardianRenderer.GUARDIAN_ELDER_LOCATION"),
			"elder-guardian particles must preserve their direct texture identity for Rust mesh admission");
		String spawner = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/blockentity/SpawnerRenderer.java"));
		assertTrue(spawner.contains("EntityRenderDispatcher.isSemanticSubmission()")
			&& spawner.contains("entityRenderDispatcher.submitSemantic("),
			"spawner preview entities must preserve the semantic dispatcher boundary during Rust extraction");
		String capturedSquid = Files.readString(Path.of("src/main/java/net/alexsmobs/client/render/layer/LayerCachalotWhaleCapturedSquid.java"));
		assertTrue(capturedSquid.contains("EntityRenderDispatcher.isSemanticSubmission()")
			&& capturedSquid.contains("dispatcher.submitSemantic("),
			"nested mod entity previews must preserve the semantic dispatcher boundary during Rust extraction");
		assertTrue(renderer.contains("particleEngine.enqueueRustGalModelParticles("),
			"Rust whole-frame GameRenderer extraction must invoke the non-quad particle semantic route");
	}

	@Test
	void rustWholeFrameKeepsSpecialModelPartFamiliesOnSemanticRoutes() throws Exception {
		String shield = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/special/ShieldSpecialRenderer.java"));
		String conduit = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/special/ConduitSpecialRenderer.java"));
		String trident = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/special/TridentSpecialRenderer.java"));
		String collector = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/SubmitNodeCollection.java"));
		String primitive = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		assertTrue(shield.contains("submitNodeCollector.submitModelPartSemantic(")
			&& conduit.contains("submitNodeCollector.submitModelPartSemantic("),
			"atlas-backed shield and conduit special models must remain semantic ModelPart producers");
		assertTrue(trident.contains("submitModelSemanticTexture(")
			&& trident.contains("enqueueStandaloneGlintModelMesh("),
			"sprite-less trident geometry must use the direct-texture Rust model route and explicit glint mesh");
		assertTrue(collector.contains("enqueueModelPartMesh(")
			&& primitive.contains("extractModelPartMesh("),
			"Rust whole-frame ModelPart submissions must copy indexed semantic geometry before backend submission");
		String levelRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/LevelRenderer.java"));
		assertTrue(levelRenderer.contains("EndGatewayRenderState")
			&& levelRenderer.contains("submitSelectedWholeFrameModelBlockEntities"),
			"end-gateway block entities must remain explicitly admitted through the Rust semantic block-entity traversal");
	}

	@Test
	void directTextureEntityModelsKeepTheirTextureIdentityAtTheRustBoundary() throws Exception {
		String collector = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/SubmitNodeCollection.java"));
		int semanticTexture = collector.indexOf("submitModelSemanticTexture(");
		int entityGate = collector.indexOf("object instanceof net.minecraft.client.renderer.entity.state.EntityRenderState entityState", semanticTexture);
		int enqueue = collector.indexOf("enqueueStandaloneModelMesh(", entityGate);
		assertTrue(semanticTexture >= 0 && entityGate > semanticTexture && enqueue > entityGate,
			"direct-texture entity models must be copied through the Rust indexed mesh route before sprite-less generic admission");
		assertTrue(collector.contains("isStandaloneTranslucentModelMeshEligible(")
			&& collector.contains("enqueueStandaloneTranslucentModelMesh("),
			"translucent direct-texture entity layers must use the explicit Rust translucent material route");
		assertTrue(collector.contains("RustGalVulkanWholeFrameMode.enabled()")
			&& collector.contains("object == net.minecraft.util.Unit.INSTANCE"),
			"the Rust presenter shell must own Unit direct-texture models without requiring a separate backend-selection flag");
		assertTrue(collector.contains("rustWholeFramePresenterActive()")
			&& collector.contains("submitGuardianBeam")
			&& collector.contains("submitCrystalBeam"),
			"semantic portal, beam, and billboard families must share the Rust presenter ownership predicate");
		assertTrue(collector.contains("submitTexturedQuads")
			&& collector.contains("submitLineSegments")
			&& collector.contains("submitColoredQuads"),
			"batched billboard, fishing-line, and procedural-quad routes must remain owned by the Rust presenter shell");
		assertTrue(collector.contains("Rust whole-frame direct-texture model route selected without a copied indexed mesh request"),
			"selected direct-texture entity models must fail closed rather than reopen Java rendering");
	}

	@Test
	void rustWorldTextCollectionDoesNotCaptureIrisModelState() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/SubmitNodeCollection.java"));
		int submitText = source.indexOf("void submitText(");
		int rustGate = source.indexOf("rustWholeFrameText", submitText);
		int capture = source.indexOf("textSubmit).iris$capture()", submitText);
		assertTrue(submitText >= 0 && rustGate > submitText && capture > rustGate,
			"text capture must be explicitly gated by Rust whole-frame ownership");
		assertTrue(source.contains("boolean rustWholeFrameText = rustWholeFramePresenterActive()"),
			"Rust presenter-shell ownership must disable Iris text capture even without the backend-selection flag");
		assertTrue(source.contains("!rustWholeFrameText")
			&& source.contains("!net.vulkanic.VulkanicAPI.isVulkanBackendSelected()")
			&& source.contains("RustGalWorldPrimitiveRenderer.isFirstPersonGuiCaptureActive()"),
			"Rust semantic text must not retain Iris ModelStorage state");
		String submitStorage = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/SubmitNodeStorage.java"));
		assertTrue(submitStorage.contains("this.order(0).submitTextSemantic(")
			&& submitStorage.contains("this.order(0).submitNameTagSemantic(")
			&& submitStorage.contains("this.order(0).submitParticleGroupSemantic("),
			"semantic callbacks must remain explicit when forwarded through SubmitNodeStorage");
		String entityRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/entity/EntityRenderer.java"));
		String avatarRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/entity/player/AvatarRenderer.java"));
		assertTrue(entityRenderer.contains("submitNodeCollector.submitNameTagSemantic(")
			&& avatarRenderer.contains("submitNodeCollector.submitNameTagSemantic(")
			&& !entityRenderer.contains("submitNodeCollector.submitNameTag(")
			&& !avatarRenderer.contains("submitNodeCollector.submitNameTag("),
			"entity name tags must use explicit Rust semantic callbacks without a legacy fallback");
		assertTrue(avatarRenderer.contains("Selected Vulkan avatar name tags are unavailable before Rust whole-frame admission"),
			"selected Vulkan avatar name tags must not fall through to Java text submission before Rust admission");
		String worldText = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		assertTrue(worldText.contains("quad.blockEntityId(), quad.glyph()"),
			"first-person text projection must preserve copied block-entity identity");
		String entityDispatcher = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/entity/EntityRenderDispatcher.java"));
		assertTrue(entityRenderer.contains("submitNodeCollector.submitLeashSemantic(")
			&& entityDispatcher.contains("submitNodeCollector.submitFlameSemantic(")
			&& entityDispatcher.contains("submitNodeCollector.submitShadowSemantic(")
			&& entityDispatcher.contains("submitNodeCollector.submitHitboxSemantic("),
			"entity feature producers must enter explicit Rust semantic callbacks");
	}

	@Test
	void rustWholeFrameModelCollectionsDoNotCaptureIrisModelState() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/SubmitNodeCollection.java"));
		assertTrue(source.contains("if (!net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()\n\t\t\t&& !net.vulkanic.VulkanicAPI.isVulkanBackendSelected())")
			&& source.contains("modelSubmit).iris$capture()")
			&& source.contains("modelPartSubmit).iris$capture()")
			&& source.contains("itemSubmit).iris$capture()"),
			"Iris ModelStorage capture must remain compatibility-only for model, ModelPart, and item records");
	}

	@Test
	void rustWholeFrameGuiDoesNotQueryIrisHudVisibility() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/Gui.java"));
		int hudCheck = source.indexOf("HudHideable");
		int rustGate = source.indexOf("RustGalVulkanWholeFrameMode.enabled()", hudCheck - 400);
		assertTrue(hudCheck >= 0 && rustGate >= 0 && rustGate < hudCheck,
			"Rust whole-frame GUI must gate Iris HUD visibility checks behind compatibility ownership");
	}

	@Test
	void rustPresentationNeverQueriesIrisFromSemanticEntityAndPortalCallsites() throws Exception {
		String dragon = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/entity/EnderDragonRenderer.java"));
		String hand = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/ItemInHandRenderer.java"));
		String portal = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/blockentity/AbstractEndPortalRenderer.java"));
		String gateway = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/blockentity/TheEndGatewayRenderer.java"));
		assertTrue(dragon.contains("boolean rustPresentation = net.vulkanic.VulkanicAPI.isVulkanBackendSelected()")
			&& dragon.contains("if (!rustPresentation && !rustCrystalBeam"),
			"End Crystal semantic collection must fence Iris entity IDs with Rust presentation ownership");
		assertTrue(hand.contains("boolean rustPresentation = net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()")
			&& hand.contains("if (!rustPresentation && !rustWholeFrame"),
			"semantic first-person hand collection must fence Iris hand state with Rust presentation ownership");
		assertTrue(portal.contains("!net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()")
			&& gateway.contains("!net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()")
			&& portal.contains("!net.vulkanic.VulkanicAPI.isVulkanBackendSelected()")
			&& gateway.contains("!net.vulkanic.VulkanicAPI.isVulkanBackendSelected()"),
			"portal RenderType compatibility checks must not query Iris while Rust owns presentation or selected Vulkan is unavailable");
	}

	@Test
	void selectedVulkanEndPortalCannotFallThroughToJavaCustomGeometry() throws Exception {
		String portal = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/blockentity/AbstractEndPortalRenderer.java"));
		int rustWholeFrame = portal.indexOf("boolean rustWholeFrame");
		int shell = portal.indexOf("RustGalVulkanWholeFrameMode.enabled()", rustWholeFrame);
		assertTrue(rustWholeFrame >= 0 && shell > rustWholeFrame,
			"End Portal ownership must include the Rust presenter shell during backend handoff");
		int selectedGuard = portal.indexOf("if (net.vulkanic.VulkanicAPI.isVulkanBackendSelected() && !rustWholeFrame)");
		int javaGeometry = portal.indexOf("submitNodeCollector.submitCustomGeometry");
		assertTrue(selectedGuard >= 0 && javaGeometry > selectedGuard,
			"selected Vulkan must fail closed before the Java End Portal geometry callback");
		assertTrue(portal.contains("Selected Vulkan cannot execute Java End Portal geometry"),
			"portal ownership failure must remain explicit and diagnostic");
	}

	@Test
	void selectedVulkanDebugBlockEntitiesCannotFallThroughToJavaGeometry() throws Exception {
		String beacon = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/blockentity/BeaconRenderer.java"));
		String boxes = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/blockentity/BlockEntityWithBoundingBoxRenderer.java"));
		String testInstance = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/blockentity/TestInstanceRenderer.java"));
		assertTrue(beacon.contains("net.vulkanic.VulkanicAPI.isVulkanBackendSelected()")
			&& beacon.contains("Rust whole-frame beacon-beam route is unavailable"),
			"selected Vulkan beacon beams must fail closed before Java callbacks");
		assertTrue(boxes.contains("net.vulkanic.VulkanicAPI.isVulkanBackendSelected()\n\t\t\t\t\t\t\t|| net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()"),
			"selected Vulkan bounding-box debug lines must fail closed before Java callbacks");
		assertTrue(boxes.contains("boolean rustLines = (net.vulkanic.VulkanicAPI.isVulkanBackendSelected()\n\t\t\t\t|| net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled())"),
			"invisible-block debug lines must remain Rust-owned during presenter handoff");
		assertTrue(testInstance.contains("net.vulkanic.VulkanicAPI.isVulkanBackendSelected()\n\t\t\t\t|| net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()"),
			"selected Vulkan test-instance markers must fail closed before Java callbacks");
	}

	@Test
	void selectedVulkanUnknownBeaconTextureCannotFallThroughToJavaGeometry() throws Exception {
		String beacon = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/blockentity/BeaconRenderer.java"));
		int knownBeam = beacon.indexOf("boolean knownRustBeam");
		int route = beacon.indexOf("Route rustRoute", knownBeam);
		assertTrue(knownBeam >= 0 && route > knownBeam,
			"beacon ownership must classify the resource identity before choosing a route");
		String admission = beacon.substring(knownBeam, Math.min(beacon.length(), route + 700));
		assertTrue(admission.contains("isVulkanBackendSelected()")
			&& admission.contains("Route.DISABLED")
			&& admission.contains("Route.JAVA_COMPATIBILITY"),
			"unknown beacon textures must be unavailable on selected Vulkan and Java-compatible only on OpenGL");
	}

	@Test
	void rustWholeFrameTextureReloadDoesNotTouchIrisPbrRegistry() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/texture/TextureManager.java"));
		String pbrUtility = Files.readString(Path.of("src/main/java/net/irisshaders/iris/pbr/util/TextureManipulationUtil.java"));
		assertTrue(pbrUtility.contains("Java Iris PBR texture mutation is unavailable on the Rust Vulkan route"),
			"direct Iris PBR texture mutation must fail closed on selected Vulkan");
		int firstPbrCall = source.indexOf("PBRTextureManager.INSTANCE");
		assertTrue(firstPbrCall >= 0, "compatibility PBR lifecycle calls must remain explicit");
		int cursor = firstPbrCall;
		while (cursor >= 0) {
			int guard = source.lastIndexOf("RustGalVulkanWholeFrameMode.enabled()", cursor);
			assertTrue(guard >= 0 && cursor - guard < 240
				&& source.substring(guard, cursor).contains("!net.vulkanic.VulkanicAPI.isVulkanBackendSelected()"),
				"every TextureManager Iris PBR lifecycle call must be unavailable on selected Vulkan");
			cursor = source.indexOf("PBRTextureManager.INSTANCE", cursor + 1);
		}
	}

	@Test
	void rustWholeFrameSpriteReloadUsesCopiedVanillaMipmapsWithoutPbrRuntimeHooks() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/texture/SpriteContents.java"));
		int provider = source.indexOf("CustomMipmapGenerator.Provider provider");
		int pbrBranch = source.lastIndexOf("if (!net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()", provider);
		assertTrue(provider >= 0 && pbrBranch >= 0 && pbrBranch < provider,
			"Rust semantic sprite reload must bypass Iris custom mipmap generators");
		assertTrue(source.substring(pbrBranch, provider).contains("!net.vulkanic.VulkanicAPI.isVulkanBackendSelected()"),
			"selected Vulkan sprite reload must bypass Iris custom mipmap generators too");
		int active = source.indexOf("public void sodium$setActive");
		int activeGuard = source.indexOf("RustGalVulkanWholeFrameMode.enabled()", active);
		assertTrue(active >= 0 && activeGuard > active,
			"Rust semantic sprite activation must not publish PBR sprite state");
		assertTrue(source.contains("Java Iris PBR sprite state is unavailable on the Rust Vulkan route")
			&& source.contains("VulkanicAPI.isVulkanBackendSelected()"),
			"selected Vulkan must fail closed before creating Iris PBR sprite holders");
		String atlas = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/texture/TextureAtlas.java"));
		assertTrue(atlas.contains("Java Iris PBR atlas state is unavailable on the Rust Vulkan route")
			&& atlas.contains("VulkanicAPI.isVulkanBackendSelected()"),
			"selected Vulkan must fail closed before creating Iris PBR atlas holders");
		String loader = Files.readString(Path.of("src/main/java/net/irisshaders/iris/pbr/loader/AtlasPBRLoader.java"));
		assertTrue(loader.contains("Java Iris PBR atlas loading is unavailable on the Rust Vulkan route"),
			"selected Vulkan must not enter the Java Iris PBR atlas loader");
	}

	@Test
	void rustWholeFrameLanguageReloadDoesNotQueryIrisPackTranslations() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/resources/language/ClientLanguage.java"));
		int lookup = source.indexOf("Iris.getCurrentPack().orElse(null)");
		int guard = source.lastIndexOf("RustGalVulkanWholeFrameMode.enabled()", lookup);
		assertTrue(lookup >= 0 && guard >= 0 && lookup - guard < 240,
			"Rust whole-frame language lookup must leave shader-pack translation resolution to copied resources");
		int builtin = source.indexOf("Iris.class.getResource");
		int builtinGuard = source.lastIndexOf("RustGalVulkanWholeFrameMode.enabled()", builtin);
		assertTrue(builtin >= 0 && builtinGuard >= 0 && builtin - builtinGuard < 240,
			"Rust whole-frame language reload must not load Iris built-in language resources");
	}

	@Test
	void rustWholeFrameBufferConstructionNeverExtendsFormatsFromIrisState() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/blaze3d/vertex/BufferBuilder.java"));
		int irisCheck = source.indexOf("Iris.isPackInUseQuick()");
		int rustGuard = source.lastIndexOf("RustGalVulkanWholeFrameMode.enabled()", irisCheck);
		assertTrue(irisCheck >= 0 && rustGuard >= 0 && irisCheck - rustGuard < 180,
			"Rust semantic buffer construction must bypass Iris vertex-format extension state");
		assertTrue(source.contains("net.vulkanic.VulkanicAPI.isVulkanBackendSelected()"),
			"selected Vulkan buffer construction must bypass Iris vertex-format extension state before whole-frame admission");
	}

	@Test
	void disabledVulkanMaterialRouteCannotRetainJavaCustomGeometryCallbacks() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/SubmitNodeCollection.java"));
		int method = source.indexOf("void submitCustomGeometry");
		int backendGate = source.indexOf("isVulkanBackendSelected()", method);
		int callbackAppend = source.indexOf("customGeometrySubmits.add", method);
		assertTrue(method >= 0 && backendGate > method && callbackAppend > backendGate,
			"Vulkan custom geometry must be gated before Java callback retention");
		assertTrue(source.contains("boolean vulkanSelected"),
			"custom geometry admission must explicitly classify Vulkan selection");
		assertTrue(source.contains("if (vulkanSelected ||"),
			"all selected Vulkan routes must fail closed, including not-yet-admitted material routes");
		assertTrue(source.contains("RustGalVulkanWholeFrameMode.enabled()"),
			"active Rust presentation must fail closed even if a stale route is misclassified as Java-compatible");
	}

	@Test
	void wholeFrameCoverageAcceptsRecordedDirectTextureModelDecisions() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/LevelRenderer.java"));
		int semanticTexture = source.indexOf("submitModelSemanticTexture");
		int decisionCheck = source.indexOf("hasCurrentFrameRustModelMeshDecision(model, textureIdentity)", semanticTexture);
		assertTrue(semanticTexture >= 0 && decisionCheck > semanticTexture,
			"coverage replay must accept copied direct-texture model decisions regardless of EntityRenderState subtype");
		String decisionPrefix = source.substring(Math.max(semanticTexture, decisionCheck - 240), decisionCheck);
		assertFalse(decisionPrefix.contains("state instanceof EntityRenderState"),
			"direct-texture coverage must not discard Unit or block-entity state decisions before Rust admission");
	}

	@Test
	void rustWholeFrameCustomGeometryCannotPresentAfterDroppingACallback() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/SubmitNodeCollection.java"));
		int routeBranch = source.indexOf("if (vulkanSelected ||");
		int throwSite = source.indexOf("Java custom geometry is unavailable on Vulkan", routeBranch);
		assertTrue(routeBranch >= 0 && throwSite > routeBranch,
			"Vulkan custom geometry must abort rather than present incomplete geometry");
		assertTrue(source.contains("recordUnsupportedCustomGeometry()"),
			"the rejected callback must remain diagnostically visible");
	}

	@Test
	void rustWholeFrameParticlesCannotPresentAfterDroppingAnUnsupportedGroup() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/feature/ParticleFeatureRenderer.java"));
		String levelRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/LevelRenderer.java"));
		int unsupported = source.indexOf("recordUnsupportedParticleGroup()");
		int throwSite = source.indexOf("Rust whole-frame particle route encountered unsupported particle-group semantics", unsupported);
		assertTrue(unsupported >= 0 && throwSite > unsupported,
			"unsupported Rust particle groups must abort rather than silently omit particles");
		assertTrue(source.contains("rust-vulkan-unavailable"),
			"the unavailable particle route must remain diagnostically visible");
		assertTrue(levelRenderer.contains("Rust whole-frame material route is unavailable while Rust owns presentation"),
			"disabled material routes must not reopen Java particle rendering after Rust presentation begins");
	}

	@Test
	void rustWholeFrameFeatureDispatchSkipsJavaModelAndCustomRenderers() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/feature/FeatureRenderDispatcher.java"));
		int modelCall = source.indexOf("modelFeatureRenderer.render");
		int modelGate = source.lastIndexOf("currentMaterialRoute().usesJavaCompatibility()", modelCall);
		int customCall = source.indexOf("customFeatureRenderer.render");
		int customGate = source.lastIndexOf("currentMaterialRoute().usesJavaCompatibility()", customCall);
		assertTrue(modelGate >= 0 && modelCall > modelGate && customGate >= 0 && customCall > customGate,
			"Java model/custom feature draws must be explicitly gated away from Rust whole-frame ownership");
		assertTrue(source.contains("currentMaterialRoute().usesJavaCompatibility()"),
			"disabled Vulkan material routes must not fall through to Java feature draws");
		String block = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/feature/BlockFeatureRenderer.java"));
		assertTrue(block.contains("boolean semanticOnly")
			&& block.contains("Java block-feature rendering is unavailable while Rust owns whole-frame presentation")
			&& block.contains("Rust semantic block-feature collection requires complete Rust ownership for every block-feature family")
			&& source.contains("this.blockFeatureRenderer.render(\n\t\t\t\tsubmitNodeCollection, this.bufferSource, this.blockRenderDispatcher, this.outlineBufferSource, true"),
			"Rust block-feature collection must use the explicit semantic-only entrypoint and never Java-render directly");
	}

	@Test
	void disabledVulkanMaterialRouteCannotFallThroughToJavaBlockModelDraw() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/feature/BlockFeatureRenderer.java"));
		String submit = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/SubmitNodeCollection.java"));
		String avatar = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/entity/player/AvatarRenderer.java"));
		String trident = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/special/TridentSpecialRenderer.java"));
		assertTrue(submit.contains("object == net.minecraft.util.Unit.INSTANCE")
			&& submit.contains("enqueueStandaloneTranslucentModelMesh")
			&& avatar.contains("enqueueStandaloneTranslucentModelMesh")
			&& trident.contains("enqueueStandaloneGlintModelMesh")
			&& submit.contains("armor_entity_glint"),
			"direct-texture model parts must use the Rust semantic model/glint contracts, including the dedicated first-person mesh domain");
		int route = source.indexOf("WorldRenderRoutePolicy.currentMaterialRoute()", source.indexOf("BlockModelSubmit"));
		int gate = source.indexOf("!blockModelRoute.usesJavaCompatibility()", route);
		int javaDraw = source.indexOf("ModelBlockRenderer.renderModel", gate);
		assertTrue(route >= 0 && gate > route && javaDraw > gate,
			"disabled Vulkan block-model routes must be gated before Java model rendering");
	}

	@Test
	void selectedVulkanBlockFeatureEntryPointCannotReopenJavaGeometry() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/feature/BlockFeatureRenderer.java"));
		int entry = source.indexOf("public void render(");
		int javaGuard = source.indexOf("net.vulkanic.VulkanicAPI.isVulkanBackendSelected()", entry);
		int javaRender = source.indexOf("ModelBlockRenderer.renderModel", entry);
		assertTrue(entry >= 0 && javaGuard > entry && javaRender > javaGuard,
			"selected Vulkan block-feature rendering must be fenced before any Java model draw");
		assertTrue(source.contains("Rust whole-frame block-display route is unavailable while Rust owns presentation"),
			"disabled block-feature subroutes must remain explicitly unavailable");
	}

	@Test
	void selectedVulkanLevelRendererCannotEnterJavaWorldPasses() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/LevelRenderer.java"));
		assertTrue(source.contains("Java LevelRenderer.renderLevel is unavailable while Rust owns whole-frame Vulkan")
			&& source.contains("Java main world pass is unavailable while Rust owns presentation")
			&& source.contains("Java late-debug pass is unavailable while Rust owns presentation"),
			"LevelRenderer must retain explicit Java-pass ownership failures");
		assertTrue(source.contains("|| net.vulkanic.VulkanicAPI.isVulkanBackendSelected()"),
			"selected Vulkan must fence LevelRenderer entrypoints even before whole-frame activation");
		int renderLevel = source.indexOf("Java LevelRenderer.renderLevel is unavailable");
		int mainPass = source.indexOf("Java main world pass is unavailable");
		assertTrue(renderLevel >= 0 && mainPass > renderLevel,
			"LevelRenderer ownership gates must precede Java world-pass construction");
	}

	@Test
	void selectedVulkanLevelRendererCannotBorrowIrisForSemanticExtraction() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/LevelRenderer.java"));
		int entityMethod = source.indexOf("private void extractVisibleEntities(");
		int entityGate = source.indexOf("net.vulkanic.VulkanicAPI.isVulkanBackendSelected()", entityMethod);
		int skipAll = source.indexOf("skipAllRendering()", entityMethod);
		assertTrue(entityMethod >= 0 && entityGate > entityMethod && skipAll > entityGate,
			"selected Vulkan entity extraction must bypass Iris skip-all state before querying it");
		int terrainMethod = source.indexOf("public void iris$renderTerrainGroup(");
		int terrainGate = source.indexOf("net.vulkanic.VulkanicAPI.isVulkanBackendSelected()", terrainMethod);
		int terrainPipeline = source.indexOf("this.pipeline.setPhase", terrainMethod);
		assertTrue(terrainMethod >= 0 && terrainGate > terrainMethod && terrainPipeline > terrainGate,
			"selected Vulkan terrain hooks must return before Iris phase mutation");
	}

	@Test
	void selectedVulkanLevelPassBuildersCannotBorrowIrisCompatibilityPolicy() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/LevelRenderer.java"));
		for (String method : new String[] {"private void addParticlesPass(", "private void addCloudsPass(", "private void addWeatherPass("}) {
			int start = source.indexOf(method);
			assertTrue(start >= 0, "missing LevelRenderer pass builder: " + method);
			int next = source.indexOf("\n\tprivate void ", start + method.length());
			if (next < 0) next = source.length();
			String body = source.substring(start, next);
			int selected = body.indexOf("isVulkanBackendSelected()");
			int iris = body.indexOf("net.irisshaders.iris.Iris.getPipelineManager()");
			assertTrue(selected >= 0 && (iris < 0 || selected < iris),
				"selected Vulkan must be fenced before Iris policy in " + method);
			assertTrue(body.contains("route is unavailable while Vulkan is selected")
				|| body.contains("route.usesRustWholeFrameVulkan()"),
				"pass builder must retain explicit Rust-route admission in " + method);
			assertTrue(body.contains("RustGalVulkanWholeFrameMode.enabled()"),
				"pass builder must fail closed during the pre-selection whole-frame handoff in " + method);
		}
	}

	@Test
	void selectedVulkanLevelChangesCannotDestroyOrPrepareIrisPipelines() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/Minecraft.java"));
		int method = source.indexOf("private void updateLevelInEngines(");
		int guard = source.indexOf("!net.vulkanic.VulkanicAPI.isVulkanBackendSelected()", method);
		int destroy = source.indexOf("net.irisshaders.iris.Iris.getPipelineManager().destroyPipeline()", method);
		int prepare = source.indexOf("net.irisshaders.iris.Iris.getPipelineManager().preparePipeline", method);
		assertTrue(method >= 0 && guard > method && destroy > guard && prepare > destroy,
			"selected Vulkan level changes must fence Iris pipeline lifecycle before destroy/prepare");
	}

	@Test
	void selectedVulkanMinecraftLifecycleCannotInitializeOrToggleIrisState() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/Minecraft.java"));
		assertTrue(source.contains("!net.vulkanic.VulkanicAPI.isVulkanBackendSelected()"),
			"Minecraft lifecycle gates must include selected Vulkan ownership");
		int keybind = source.indexOf("net.irisshaders.iris.Iris.handleKeybinds(this)");
		int keybindGuard = source.lastIndexOf("!net.vulkanic.VulkanicAPI.isVulkanBackendSelected()", keybind);
		assertTrue(keybind > 0 && keybindGuard >= 0 && keybindGuard < keybind,
			"selected Vulkan ticks must not invoke Iris shader keybind handling");
		int texture = source.indexOf("new net.irisshaders.iris.targets.backed.NativeImageBackedCustomTexture");
		int textureGuard = source.lastIndexOf("!net.vulkanic.VulkanicAPI.isVulkanBackendSelected()", texture);
		assertTrue(texture > 0 && textureGuard >= 0 && textureGuard < texture,
			"selected Vulkan startup must not construct Java Iris textures");
		int handle = source.indexOf("net.vulkanic.VulkanicCoreAPI.textureId(mainColorTexture)");
		int handleGuard = source.lastIndexOf("VulkanicAPI.isVulkanBackendSelected()", handle);
		assertTrue(handle > 0 && handleGuard >= 0 && handleGuard < handle,
			"selected Vulkan diagnostics must not query Java texture handles");
	}

	@Test
	void selectedVulkanTitleScreenCannotInitializeIrisRendererLifecycle() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/screens/TitleScreen.java"));
		int hook = source.indexOf("net.irisshaders.iris.Iris.onLoadingComplete()");
		int selectedGuard = source.lastIndexOf("!net.vulkanic.VulkanicAPI.isVulkanBackendSelected()", hook);
		assertTrue(hook > 0 && selectedGuard >= 0 && selectedGuard < hook,
			"selected Vulkan main-menu setup must not initialize Iris renderer lifecycle");
	}

	@Test
	void selectedVulkanLevelSetupCannotConstructJavaTerrainStateBeforeOwnershipGate() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/LevelRenderer.java"));
		int setLevel = source.indexOf("public void setLevel(@Nullable ClientLevel clientLevel)");
		int allChanged = source.indexOf("this.allChanged();", setLevel);
		int sodium = source.indexOf("Java Sodium world setup is unavailable while Vulkan is selected", setLevel);
		assertTrue(setLevel >= 0 && allChanged > setLevel && sodium > allChanged,
			"LevelRenderer must decide Java terrain ownership before world setup can reach Sodium");
		String setup = source.substring(setLevel, allChanged);
		assertTrue(setup.contains("isVulkanBackendSelected()")
			&& setup.contains("RustGalVulkanWholeFrameMode.enabled()"),
			"selected Vulkan and Rust whole-frame setup must bypass Java allChanged terrain allocation");
		int allChangedMethod = source.indexOf("public void allChanged()");
		int returnGuard = source.indexOf("Do not recreate Java section dispatchers or ViewArea buffers", allChangedMethod);
		assertTrue(allChangedMethod >= 0 && returnGuard > allChangedMethod,
			"direct allChanged calls must also remain fail-closed for Rust-owned world state");
	}

	@Test
	void selectedVulkanLevelInvalidationCannotScheduleJavaSodiumTerrain() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/LevelRenderer.java"));
		int method = source.indexOf("public void needsUpdate()");
		int guard = source.indexOf("never reopen Sodium's Java terrain scheduler", method);
		int schedule = source.indexOf("this.renderer.scheduleTerrainUpdate()", method);
		assertTrue(method >= 0 && guard > method && schedule > guard,
			"selected Vulkan terrain invalidation must return before Java Sodium scheduling");
		String prefix = source.substring(method, guard);
		assertTrue(prefix.contains("isVulkanBackendSelected()")
			&& prefix.contains("RustGalVulkanWholeFrameMode.enabled()"),
			"the terrain scheduler gate must cover selected Vulkan and the active Rust shell");
	}

	@Test
	void selectedVulkanAutomaticScreenshotCannotQueryJavaTerrainReadiness() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/LevelRenderer.java"));
		int count = source.indexOf("public int countRenderedSections()");
		int countRedirect = source.indexOf("this.renderer.getVisibleChunkCount()", count);
		int ready = source.indexOf("public boolean hasRenderedAllSections()");
		int readyRedirect = source.indexOf("this.renderer.isTerrainRenderComplete()", ready);
		assertTrue(count >= 0 && countRedirect > count && ready > countRedirect && readyRedirect > ready,
			"automatic screenshot readiness must retain its Java compatibility queries for OpenGL only");
		assertTrue(source.substring(count, countRedirect).contains("return 0")
			&& source.substring(ready, readyRedirect).contains("return false")
			&& source.substring(count, countRedirect).contains("isVulkanBackendSelected()")
			&& source.substring(ready, readyRedirect).contains("isVulkanBackendSelected()"),
			"Rust-owned Vulkan must not consult Java Sodium terrain readiness");
	}

	@Test
	void selectedVulkanLevelRendererCannotLoadJavaTransparencyPostChain() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/LevelRenderer.java"));
		int helper = source.indexOf("private PostChain getTransparencyChain()");
		int guard = source.indexOf("Java transparency post-chain is unavailable while Vulkan is selected", helper);
		int load = source.indexOf("getPostChain(TRANSPARENCY_POST_CHAIN_ID", helper);
		assertTrue(helper >= 0 && guard > helper && load > guard,
			"selected Vulkan must reject Java transparency-chain loading before ShaderManager access");
		String body = source.substring(helper, load);
		assertTrue(body.contains("VulkanicAPI.isVulkanBackendSelected()")
			&& body.contains("RustGalVulkanWholeFrameMode.enabled()"),
			"transparency-chain loading must remain fenced during the pre-selection whole-frame handoff");
	}

	@Test
	void selectedVulkanShaderManagerCannotLoadJavaPostChains() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/ShaderManager.java"));
		int method = source.indexOf("public PostChain getPostChain");
		int guard = source.indexOf("Java post-chain loading is unavailable while Rust owns the selected Vulkan route", method);
		int cache = source.indexOf("getOrLoadPostChain", method);
		int selected = source.indexOf("VulkanicAPI.isVulkanBackendSelected()", method);
		int wholeFrame = source.indexOf("RustGalVulkanWholeFrameMode.enabled()", method);
		assertTrue(method >= 0 && selected > method && wholeFrame > selected && guard > wholeFrame && cache > guard,
			"the shared Java post-chain loader must reject selected/whole-frame Vulkan before cache or pipeline construction");
	}

	@Test
	void selectedVulkanPostChainFactoryCannotAllocateJavaResources() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/PostChain.java"));
		int factory = source.indexOf("public static PostChain load(");
		int selected = source.indexOf("VulkanicAPI.isVulkanBackendSelected()", factory);
		int wholeFrame = source.indexOf("RustGalVulkanWholeFrameMode.enabled()", factory);
		int guard = source.indexOf("Java post-chain construction is unavailable while Rust owns the selected Vulkan route", factory);
		int textureLookup = source.indexOf("textureManager.getTexture", factory);
		assertTrue(factory >= 0 && selected > factory && wholeFrame > selected && guard > wholeFrame && textureLookup > guard,
			"direct Java post-chain construction must reject selected/whole-frame Vulkan before resource-pack texture lookup");
	}

	@Test
	void selectedVulkanGuiTexturedFillCannotCarryJavaTextureViews() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/GuiGraphics.java"));
		int method = source.indexOf("public void fill(RenderPipeline renderPipeline, TextureSetup textureSetup");
		int selected = source.indexOf("VulkanicAPI.isVulkanBackendSelected()", method);
		int textureCheck = source.indexOf("textureSetup.texure0()", method);
		int submit = source.indexOf("submitColoredRectangle", method);
		assertTrue(method >= 0 && selected > method && textureCheck > selected && submit > textureCheck,
			"selected Vulkan textured GUI fills must reject Java texture views before GUI state submission");
	}

	@Test
	void wholeFrameGuiDoesNotBuildJavaItemAtlasBeforeRustSemanticItems() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/render/GuiRenderer.java"));
		int method = source.indexOf("private void prepareItemElements()");
		int selected = source.indexOf("VulkanicAPI.isVulkanBackendSelected()", method);
		int wholeFrame = source.indexOf("RustGalVulkanWholeFrameMode.enabled()", selected);
		int atlas = source.indexOf("this.createAtlasTextures", method);
		assertTrue(method >= 0 && selected > method && wholeFrame > selected && atlas > wholeFrame,
			"Rust whole-frame GUI item preparation must return before Java item-atlas allocation");
	}

	@Test
	void selectedVulkanGameRendererCannotRunJavaBlurPostChain() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
		int guard = source.indexOf("Java GUI blur post-process is unavailable");
		int postChain = source.indexOf("postChain.process", guard);
		assertTrue(guard >= 0 && postChain > guard,
			"Java blur post-processing must remain behind its ownership failure gate");
		String prefix = source.substring(Math.max(0, guard - 220), guard);
		assertTrue(prefix.contains("isVulkanBackendSelected()"),
			"selected Vulkan must fence the Java blur PostChain before processing");
		assertTrue(prefix.contains("isWholeFrameVulkanEnabled()"),
			"pre-selection Rust ownership must fence Java blur PostChain construction");
	}

	@Test
	void selectedVulkanGameRendererCannotRunJavaEntityPostEffectChain() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
		int effect = source.indexOf("this.postEffectId != null && this.effectActive");
		int reset = source.indexOf("VulkanicAPI.resetTextureMatrix()", effect);
		int guard = source.indexOf("Java post-effect processing is unavailable while Vulkan is selected", effect);
		assertTrue(effect >= 0 && guard > effect && reset > guard,
			"selected Vulkan must fence Java entity post-effect setup before touching the texture matrix");
	}

	@Test
	void rustWholeFrameBlockSubmissionsCannotPresentAfterDroppingSemanticGeometry() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/feature/BlockFeatureRenderer.java"));
		String collectorSource = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/SubmitNodeCollection.java"));
		String worldSource = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		assertTrue(worldSource.contains("BlendFunction.TRANSLUCENT.equals(blend.get())")
			&& worldSource.contains("MATERIAL_ID_TRANSLUCENT_TEXTURED, MATERIAL_MODE_TRANSLUCENT"),
			"semantic moving/block-model admission must retain translucent material semantics");
		int displayUnavailable = source.indexOf("recordSubmittedWorkIdentity(\"block-display\", \"rust-vulkan-unavailable");
		int displayAbort = source.indexOf("Rust whole-frame block-display route has no semantic mesh", displayUnavailable);
		int modelUnavailable = source.indexOf("queued ? \"rust-vulkan-whole-frame\" : \"rust-vulkan-unavailable\"");
		int modelAbort = source.indexOf("Rust whole-frame block-model route has no semantic mesh", modelUnavailable);
		assertTrue(displayUnavailable >= 0 && displayAbort > displayUnavailable,
			"unsupported block-display geometry must abort rather than silently omit the submission");
		assertTrue(modelUnavailable >= 0 && modelAbort > modelUnavailable,
			"unsupported block-model geometry must abort rather than silently omit the submission");
		assertTrue(source.contains("RustGalVulkanWholeFrameMode.enabled()")
			&& source.contains("Rust whole-frame block-model route is unavailable while Rust owns presentation"),
			"disabled block-model routes must not silently omit geometry after Rust presentation begins");
		assertTrue(source.contains("Rust whole-frame Primed TNT route is unavailable while Rust owns presentation"),
			"disabled Primed TNT routes must not silently omit geometry after Rust presentation begins");
		assertTrue(source.contains("Rust whole-frame block-display route is unavailable while Rust owns presentation"),
			"disabled block-display routes must not silently omit geometry after Rust presentation begins");
		assertTrue(source.contains("Rust whole-frame ")
			&& source.contains("is unavailable while Rust owns presentation"),
			"disabled moving-block routes must not silently omit geometry after Rust presentation begins");
		int collectorRoute = collectorSource.indexOf("enqueueBlockModelMesh(semanticSubmit)");
		int collectorAbort = collectorSource.indexOf("Rust whole-frame block-model route has no semantic mesh", collectorRoute);
		assertTrue(collectorRoute >= 0 && collectorAbort > collectorRoute,
			"collector block-model submissions must abort when semantic enqueue is rejected");
	}

	@Test
	void rustBlockModelMeshCarriesVanillaOverlayColorSemantics() throws Exception {
		String worldSource = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		int enqueue = worldSource.indexOf("public static boolean enqueueBlockModelMesh");
		int eligibility = worldSource.indexOf("isBlockModelMeshSemanticallyEligible", enqueue);
		assertTrue(enqueue >= 0 && eligibility > enqueue, "block-model semantic route must expose bounded admission");
		String enqueueBody = worldSource.substring(enqueue, eligibility);
		assertFalse(enqueueBody.contains("submit.overlayCoords() != 0"),
			"Rust block-model admission must not discard vanilla hurt-overlay coordinates");
		assertTrue(enqueueBody.contains("overlayColorArgb(submit.overlayCoords())"),
			"Rust block-model instances must carry copied overlay color semantics");
		String eligibilityBody = worldSource.substring(eligibility,
			worldSource.indexOf("\n\tprivate static MeshMaterial meshMaterialForRenderType", eligibility));
		assertFalse(eligibilityBody.contains("overlayCoords != 0"),
			"block-model eligibility must agree with the explicit overlay instance payload");
	}

	@Test
	void rustAtlasModelMeshCarriesVanillaOverlayColorSemantics() throws Exception {
		String worldSource = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		int eligibility = worldSource.indexOf("public static String modelMeshIneligibilityReason");
		int enqueue = worldSource.indexOf("public static <S> boolean enqueueModelMesh(", eligibility);
		assertTrue(eligibility >= 0 && enqueue > eligibility,
			"atlas model route must expose explicit overlay admission");
		String eligibilityBody = worldSource.substring(eligibility, enqueue);
		assertFalse(eligibilityBody.contains("overlayCoords != OverlayTexture.NO_OVERLAY"),
			"atlas model admission must not discard packed hurt-overlay coordinates");
		int instance = worldSource.indexOf("PENDING_MESH_INSTANCES.add", enqueue);
		assertTrue(instance > enqueue,
			"atlas model route must publish an explicit indexed mesh instance");
		assertTrue(worldSource.substring(enqueue).contains("overlayColorArgb(overlayCoords)"),
			"atlas model instances must carry copied overlay color semantics into Rust");
	}

	@Test
	void rustSharedModelMeshDoesNotCarryJavaEntityIdentity() throws Exception {
		String worldSource = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		int helper = worldSource.indexOf("private static <S> boolean enqueueEligibleModelMesh(");
		int extraction = worldSource.indexOf("int diagnosticEntityId =", helper);
		int instance = worldSource.indexOf("PENDING_MESH_INSTANCES.add", extraction);
		assertTrue(helper >= 0 && extraction > helper && instance > extraction,
			"shared model enqueue must construct explicit entity instances");
		String body = worldSource.substring(extraction, instance);
		assertTrue(body.contains("state instanceof EntityRenderState entityRenderState"),
			"shared model enqueue may retain the Java ID only for diagnostics");
		assertTrue(worldSource.contains("Rust VulkanicGAL model mesh requires a seeded bounded world primitive frame"));
		String instanceBody = worldSource.substring(instance, Math.min(worldSource.length(), instance + 1300));
		assertTrue(instanceBody.contains("\n\t\t\t\t\t0,"),
			"shared model instances must publish Rust-resolved identity (Java entity ID is zeroed)");
	}

	@Test
	void rustModelPartMeshCarriesVanillaOverlayColorSemantics() throws Exception {
		String worldSource = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		int eligibility = worldSource.indexOf("public static String modelPartMeshEligibilityReason");
		int enqueue = worldSource.indexOf("public static boolean enqueueModelPartMesh", eligibility);
		assertTrue(eligibility >= 0 && enqueue > eligibility, "ModelPart overlay route must remain explicitly bounded");
		String eligibilityBody = worldSource.substring(eligibility, enqueue);
		assertFalse(eligibilityBody.contains("overlayCoords != OverlayTexture.NO_OVERLAY"),
			"ModelPart admission must not discard vanilla packed hurt-overlay coordinates");
		assertTrue(eligibilityBody.contains("sheeted && hasFoil"),
			"sheeted must remain a rejection only for the foil-buffer variant; non-foil model parts have unchanged semantics");
		String enqueueBody = worldSource.substring(enqueue,
			worldSource.indexOf("\n\t/**\n\t * {@link net.minecraft.client.renderer.OrderedSubmitNodeCollector}", enqueue));
		assertTrue(enqueueBody.contains("overlayColorArgb(overlayCoords)"),
			"ModelPart instances must carry copied overlay color semantics into Rust");
		assertTrue(enqueueBody.contains("0, overlayColorArgb(overlayCoords), outlineColor"),
			"ModelPart glint instances must preserve the same overlay and outline semantics");
		assertTrue(enqueueBody.contains("Rust VulkanicGAL ModelPart requires a seeded bounded world primitive frame"));
	}

	@Test
	void rustBlockDisplayMeshCarriesVanillaOverlayColorSemantics() throws Exception {
		String worldSource = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		int enqueue = worldSource.indexOf("public static boolean enqueueBlockDisplay(");
		int body = worldSource.indexOf("PENDING_MESH_INSTANCES.add", enqueue);
		int next = worldSource.indexOf("\n\tprivate static IllegalStateException blockDisplayAdmissionFailure", body);
		assertTrue(enqueue >= 0 && body > enqueue && next > body, "block-display route must publish explicit mesh instances");
		String instanceBody = worldSource.substring(body, next);
		assertTrue(instanceBody.contains("overlayColorArgb(blockSubmit.overlayCoords())"),
			"block-display instances must carry copied overlay color semantics into Rust");
	}

	@Test
	void rustItemEntityMeshCarriesVanillaOverlayColorSemantics() throws Exception {
		String worldSource = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		int method = worldSource.indexOf("public static String itemEntityMeshIneligibility");
		int enqueue = worldSource.indexOf("public static boolean enqueueItemEntityMesh(", method);
		int instance = worldSource.indexOf("PENDING_MESH_INSTANCES.add", enqueue);
		int next = worldSource.indexOf("\n\t/**\n\t * Copies a Fabric/Sodium MeshView", instance);
		assertTrue(method >= 0 && enqueue > method && instance > enqueue && next > instance,
			"item-entity semantic route must expose bounded overlay admission");
		String admission = worldSource.substring(method, enqueue);
		assertFalse(admission.contains("overlayCoords != OverlayTexture.NO_OVERLAY"),
			"item entities must not discard vanilla packed hurt-overlay coordinates");
		String instances = worldSource.substring(instance, next);
		assertTrue(instances.contains("overlayColorArgb(overlayCoords)"),
			"item and glint instances must carry copied overlay color semantics into Rust");
		assertTrue(worldSource.contains("Rust VulkanicGAL item-entity mesh requires a seeded bounded world primitive frame"));
	}

	@Test
	void rustPrimedTntMeshCarriesVanillaOverlayColorSemantics() throws Exception {
		String worldSource = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		int eligibility = worldSource.indexOf("public static boolean isPrimedTntMeshEligible");
		assertTrue(eligibility >= 0, "primed-TNT route must expose explicit overlay admission");
		String eligibilityBody = worldSource.substring(eligibility,
			worldSource.indexOf("\n\tpublic static boolean enqueueFallingBlock", eligibility));
		assertFalse(eligibilityBody.contains("overlayCoords() != 0"),
			"primed TNT must not discard packed overlay coordinates");
		int producer = worldSource.indexOf("PENDING_MESH_PRODUCERS.add(PendingMeshProducer.PRIMED_TNT)");
		int instance = worldSource.lastIndexOf("PENDING_MESH_INSTANCES.add", producer);
		assertTrue(producer > instance && instance >= 0
			&& worldSource.substring(instance, producer).contains("overlayColorArgb(blockSubmit.overlayCoords())"),
			"primed TNT instance must carry copied overlay color semantics");
		assertTrue(worldSource.contains("Rust VulkanicGAL PrimedTnt requires a seeded bounded world primitive frame"));
		assertTrue(worldSource.contains("STRATUM_WORLD_ENTITY_MESH")
			&& worldSource.contains("WORLD_MESH_INSTANCE_FLAG_OUTLINE_ONLY")
			&& worldSource.contains("blockSubmit.outlineColor() != 0"),
			"outlined primed TNT must add a bounded Rust outline-only mesh instance");
	}

	@Test
	void primedTntCallsiteCopiesFlashingOverlayIntoSemanticSubmission() throws Exception {
		String tntSource = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/entity/TntRenderer.java"));
		assertTrue(tntSource.contains("submitPrimedTntBlockSemantic("),
			"primed TNT must use the semantic submission seam for every visual state");
		assertTrue(tntSource.contains("OverlayTexture.pack")
			&& tntSource.contains("OverlayTexture.NO_OVERLAY")
			&& tntSource.contains("tntRenderState.outlineColor"),
			"primed TNT semantic submission must carry copied flash and outline metadata");
		assertFalse(tntSource.contains("submitWhiteSolidBlock"),
			"primed TNT must not lower flashing state into the legacy white-solid helper");
	}

	@Test
	void coldChickenUsesTheSharedCopiedChickenMeshAdmission() throws Exception {
		String renderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/entity/LivingEntityRenderer.java"));
		String worldSource = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		assertTrue(renderer.contains("this.model instanceof ChickenModel")
			&& worldSource.contains("textureIdentity.getPath().startsWith(\"textures/entity/chicken/\")"),
			"cold chicken must use the same copied ChickenModel semantic family");
		assertTrue(worldSource.contains("!state.isBaby") && worldSource.contains("!translucentBody"),
			"cold chicken expansion must preserve bounded adult opaque admission");
	}

	@Test
	void rabbitBabyUsesTheSameCopiedModelMeshContract() throws Exception {
		String worldSource = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		int method = worldSource.indexOf("public static boolean isVanillaRabbitModelMeshEligible");
		int next = worldSource.indexOf("\n\tpublic static boolean isVanillaPigModelMeshEligible", method);
		assertTrue(method >= 0 && next > method, "rabbit admission method must remain explicit");
		String body = worldSource.substring(method, next);
		assertFalse(body.contains("!state.isBaby"),
			"rabbit baby geometry is already represented by the copied RabbitModel mesh");
		assertTrue(body.contains("vanillaRabbitTextureIdentity(state)")
			&& body.contains("isStandaloneModelMeshEligible"),
			"both rabbit ages must retain copied texture and mesh eligibility checks");
	}

	@Test
	void chickenBabyUsesTheSharedCopiedChickenMeshContract() throws Exception {
		String worldSource = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		int method = worldSource.indexOf("public static boolean isVanillaChickenModelMeshEligible");
		int next = worldSource.indexOf("\n\t/** Resolves the direct texture identity", method);
		assertTrue(method >= 0 && next > method, "chicken admission method must remain explicit");
		String body = worldSource.substring(method, next);
		assertFalse(body.contains("!state.isBaby"),
			"baked temperate and cold baby layers must use the copied ChickenModel route");
		assertTrue(body.contains("model instanceof ChickenModel")
			&& body.contains("textures/entity/chicken/"),
			"chicken age variants must retain bounded model and texture-family admission");
	}

	@Test
	void donkeyBabyUsesTheCopiedDonkeyMeshWithChestState() throws Exception {
		String worldSource = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		int method = worldSource.indexOf("public static boolean isVanillaDonkeyModelMeshEligible");
		int next = worldSource.indexOf("\n\t/** Llama bodies", method);
		assertTrue(method >= 0 && next > method, "donkey admission method must remain explicit");
		String body = worldSource.substring(method, next);
		assertFalse(body.contains("!state.isBaby"),
			"donkey and mule baby layers must use the copied DonkeyModel route");
		assertTrue(body.contains("horse/donkey.png") && body.contains("horse/mule.png")
			&& body.contains("isStandaloneModelMeshEligible"),
			"donkey age variants must retain bounded texture and mesh admission");
	}

	@Test
	void horseBabyUsesCopiedHorseBodyWhileEquipmentStaysSeparate() throws Exception {
		String worldSource = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		int method = worldSource.indexOf("public static boolean isVanillaHorseModelMeshEligible");
		int next = worldSource.indexOf("\n\t/** Donkey/mule", method);
		assertTrue(method >= 0 && next > method, "horse admission method must remain explicit");
		String body = worldSource.substring(method, next);
		assertFalse(body.contains("!state.isBaby"),
			"baby horse body layers must use the copied HorseModel route");
		assertTrue(body.contains("textures/entity/horse/horse_")
			&& body.contains("isStandaloneModelMeshEligible"),
			"horse age variants must retain bounded variant texture and mesh admission");
	}

	@Test
	void llamaBabyUsesCopiedLlamaBodyWhileDecorStaysSeparate() throws Exception {
		String worldSource = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		int method = worldSource.indexOf("public static boolean isVanillaLlamaModelMeshEligible");
		int next = worldSource.indexOf("\n\t/** Strider bodies", method);
		assertTrue(method >= 0 && next > method, "llama admission method must remain explicit");
		String body = worldSource.substring(method, next);
		assertFalse(body.contains("!state.isBaby"),
			"baby llama body layers must use the copied LlamaModel route");
		assertTrue(body.contains("textures/entity/llama/creamy.png")
			&& body.contains("textures/entity/llama/gray.png")
			&& body.contains("isStandaloneModelMeshEligible"),
			"llama age variants must retain bounded variant texture and mesh admission");
	}

	@Test
	void striderBabyUsesCopiedBodyWhileSaddleStaysSeparate() throws Exception {
		String worldSource = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		int method = worldSource.indexOf("public static boolean isVanillaStriderModelMeshEligible");
		int next = worldSource.indexOf("\n\t/** Hoglin and zoglin", method);
		assertTrue(method >= 0 && next > method, "strider admission method must remain explicit");
		String body = worldSource.substring(method, next);
		assertFalse(body.contains("!state.isBaby"),
			"baby strider body layers must use the copied StriderModel route");
		assertTrue(body.contains("textures/entity/strider/strider.png")
			&& body.contains("textures/entity/strider/strider_cold.png")
			&& body.contains("isStandaloneModelMeshEligible"),
			"strider age variants must retain bounded warm/cold texture admission");
	}

	@Test
	void hoglinBabyUsesCopiedHoglinBodyForHoglinAndZoglinTextures() throws Exception {
		String worldSource = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		int method = worldSource.indexOf("public static boolean isVanillaHoglinModelMeshEligible");
		int next = worldSource.indexOf("\n\t/** Camel bodies", method);
		assertTrue(method >= 0 && next > method, "hoglin admission method must remain explicit");
		String body = worldSource.substring(method, next);
		assertFalse(body.contains("!state.isBaby"),
			"baby hoglin and zoglin body layers must use the copied HoglinModel route");
		assertTrue(body.contains("textures/entity/hoglin/hoglin.png")
			&& body.contains("textures/entity/hoglin/zoglin.png")
			&& body.contains("isStandaloneModelMeshEligible"),
			"hoglin age variants must retain bounded body texture and mesh admission");
	}

	@Test
	void camelBabyUsesCopiedAnimatedBodyWhileSaddleStaysSeparate() throws Exception {
		String worldSource = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		int method = worldSource.indexOf("public static boolean isVanillaCamelModelMeshEligible");
		int next = worldSource.indexOf("\n\t/** Plain piglins", method);
		assertTrue(method >= 0 && next > method, "camel admission method must remain explicit");
		String body = worldSource.substring(method, next);
		assertFalse(body.contains("!state.isBaby"),
			"baby camel body layers must use the copied CamelModel animation route");
		assertTrue(body.contains("textures/entity/camel/camel.png")
			&& body.contains("isStandaloneModelMeshEligible"),
			"camel age variants must retain bounded texture and mesh admission");
	}

	@Test
	void plainBabyPiglinUsesCopiedBodyWhileEquipmentRemainsRequiredToBeEmpty() throws Exception {
		String worldSource = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		int method = worldSource.indexOf("public static boolean isVanillaPiglinModelMeshEligible");
		int next = worldSource.indexOf("\n\t/** Plain adult skeletons", method);
		assertTrue(method >= 0 && next > method, "piglin admission method must remain explicit");
		String body = worldSource.substring(method, next);
		assertFalse(body.contains("!state.isBaby"),
			"plain baby piglins must use the copied PiglinModel body route");
		assertTrue(body.contains("state.rightHandItem.isEmpty()")
			&& body.contains("state.headItem.isEmpty()")
			&& body.contains("state.wornHeadType == null"),
			"piglin admission must continue to require copied empty equipment state");
	}

	@Test
	void babyZombieUsesCopiedBodyWhileSpecialStatesRemainRejected() throws Exception {
		String worldSource = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		int method = worldSource.indexOf("public static boolean isVanillaZombieModelMeshEligible");
		int next = worldSource.indexOf("\n\t/**\n\t * Marks the real ItemEntity", method);
		assertTrue(method >= 0 && next > method, "zombie admission method must remain explicit");
		String body = worldSource.substring(method, next);
		assertFalse(body.contains("!state.isBaby"),
			"baby zombies must use the copied ZombieModel body route");
		assertTrue(body.contains("!state.isUsingItem")
			&& body.contains("!state.isConverting")
			&& body.contains("!state.displayFireAnimation"),
			"zombie admission must retain copied special-state exclusions");
	}

	@Test
	void babyWolfUsesCopiedBodyWhileCollarAndArmorStaySeparate() throws Exception {
		String worldSource = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		int method = worldSource.indexOf("public static boolean isVanillaWolfModelMeshEligible");
		int next = worldSource.indexOf("\n\t/** Parrot has", method);
		assertTrue(method >= 0 && next > method, "wolf admission method must remain explicit");
		String body = worldSource.substring(method, next);
		assertFalse(body.contains("!state.isBaby"),
			"baby wolves must use the copied WolfModel body route");
		assertTrue(body.contains("textures/entity/wolf/")
			&& body.contains("isStandaloneModelMeshEligible"),
			"wolf age variants must retain resource-pack texture and mesh admission");
	}

	@Test
	void babyGoatUsesCopiedBodyWithHornAndRammingState() throws Exception {
		String worldSource = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		int method = worldSource.indexOf("public static boolean isVanillaGoatModelMeshEligible");
		int next = worldSource.indexOf("\n\t/** Base tropical-fish", method);
		assertTrue(method >= 0 && next > method, "goat admission method must remain explicit");
		String body = worldSource.substring(method, next);
		assertFalse(body.contains("!state.isBaby"),
			"baby goats must use the copied GoatModel body route");
		assertTrue(body.contains("textures/entity/goat/goat.png")
			&& body.contains("isStandaloneModelMeshEligible"),
			"goat age variants must retain bounded texture and mesh admission");
	}

	@Test
	void babyPolarBearUsesCopiedBodyWithStandAnimationState() throws Exception {
		String worldSource = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		int method = worldSource.indexOf("public static boolean isVanillaPolarBearModelMeshEligible");
		int next = worldSource.indexOf("\n\t/** Dolphins use", method);
		assertTrue(method >= 0 && next > method, "polar-bear admission method must remain explicit");
		String body = worldSource.substring(method, next);
		assertFalse(body.contains("!state.isBaby"),
			"baby polar bears must use the copied PolarBearModel body route");
		assertTrue(body.contains("textures/entity/bear/polarbear.png")
			&& body.contains("isStandaloneModelMeshEligible"),
			"polar-bear age variants must retain bounded texture and mesh admission");
	}

	@Test
	void babyTurtleUsesCopiedBodyWhileEggStateRemainsSemantic() throws Exception {
		String worldSource = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		int method = worldSource.indexOf("public static boolean isVanillaTurtleModelMeshEligible");
		int next = worldSource.indexOf("\n\t/** Pandas copy", method);
		assertTrue(method >= 0 && next > method, "turtle admission method must remain explicit");
		String body = worldSource.substring(method, next);
		assertFalse(body.contains("!state.isBaby"),
			"baby turtles must use the copied TurtleModel body route");
		assertTrue(body.contains("textures/entity/turtle/big_sea_turtle.png")
			&& body.contains("isStandaloneModelMeshEligible"),
			"turtle age variants must retain bounded texture and mesh admission");
	}

	@Test
	void babyPandaUsesCopiedGeneTextureWhileHeldItemsStaySeparate() throws Exception {
		String worldSource = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		int method = worldSource.indexOf("public static boolean isVanillaPandaModelMeshEligible");
		int next = worldSource.indexOf("\n\tpublic static ResourceLocation vanillaPandaTextureIdentity", method);
		assertTrue(method >= 0 && next > method, "panda admission method must remain explicit");
		String body = worldSource.substring(method, next);
		assertFalse(body.contains("!state.isBaby"),
			"baby pandas must use the copied PandaModel body route");
		assertTrue(body.contains("vanillaPandaTextureIdentity(state)")
			&& body.contains("isStandaloneModelMeshEligible"),
			"panda age variants must retain gene texture and mesh admission");
	}

	@Test
	void babyBeeUsesCopiedAngerAndNectarTextureState() throws Exception {
		String worldSource = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		int method = worldSource.indexOf("public static boolean isVanillaBeeModelMeshEligible");
		int next = worldSource.indexOf("\n\tpublic static ResourceLocation vanillaBeeTextureIdentity", method);
		assertTrue(method >= 0 && next > method, "bee admission method must remain explicit");
		String body = worldSource.substring(method, next);
		assertFalse(body.contains("!state.isBaby"),
			"baby bees must use the copied BeeModel body route");
		assertTrue(body.contains("vanillaBeeTextureIdentity(state)")
			&& body.contains("isStandaloneModelMeshEligible"),
			"bee age variants must retain anger/nectar texture and mesh admission");
	}

	@Test
	void babyAxolotlUsesCopiedVariantTextureAndAnimationState() throws Exception {
		String worldSource = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		int method = worldSource.indexOf("public static boolean isVanillaAxolotlModelMeshEligible");
		int next = worldSource.indexOf("\n\tpublic static ResourceLocation vanillaAxolotlTextureIdentity", method);
		assertTrue(method >= 0 && next > method, "axolotl admission method must remain explicit");
		String body = worldSource.substring(method, next);
		assertFalse(body.contains("!state.isBaby"),
			"baby axolotls must use the copied AxolotlModel body route");
		assertTrue(body.contains("vanillaAxolotlTextureIdentity(state)")
			&& body.contains("isStandaloneModelMeshEligible"),
			"axolotl age variants must retain exact variant texture and mesh admission");
	}

	@Test
	void babyFoxUsesCopiedSleepVariantTextureWhileHeldItemsStaySeparate() throws Exception {
		String worldSource = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		int method = worldSource.indexOf("public static boolean isVanillaFoxModelMeshEligible");
		int next = worldSource.indexOf("\n\t/** Vindicator/Pillager", method);
		assertTrue(method >= 0 && next > method, "fox admission method must remain explicit");
		String body = worldSource.substring(method, next);
		assertFalse(body.contains("!state.isBaby"),
			"baby foxes must use the copied FoxModel body route");
		assertTrue(body.contains("textures/entity/fox/fox_sleep.png")
			&& body.contains("textures/entity/fox/snow_fox_sleep.png")
			&& body.contains("isStandaloneModelMeshEligible"),
			"fox age variants must retain sleep/variant texture and mesh admission");
	}

	@Test
	void babySquidUsesCopiedModelForSquidAndGlowSquidTextures() throws Exception {
		String worldSource = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		int method = worldSource.indexOf("public static boolean isVanillaSquidModelMeshEligible");
		int next = worldSource.indexOf("\n\t/** Guardian bodies", method);
		assertTrue(method >= 0 && next > method, "squid admission method must remain explicit");
		String body = worldSource.substring(method, next);
		assertFalse(body.contains("!state.isBaby"),
			"baby squid variants must use the copied SquidModel body route");
		assertTrue(body.contains("textures/entity/squid/squid.png")
			&& body.contains("textures/entity/squid/glow_squid.png")
			&& body.contains("isStandaloneModelMeshEligible"),
			"squid age variants must retain exact texture and mesh admission");
	}

	@Test
	void babyArmadilloUsesCopiedShellAndRollBodyState() throws Exception {
		String worldSource = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		int method = worldSource.indexOf("public static boolean isVanillaArmadilloModelMeshEligible");
		int next = worldSource.indexOf("\n\t/** Sniffers have", method);
		assertTrue(method >= 0 && next > method, "armadillo admission method must remain explicit");
		String body = worldSource.substring(method, next);
		assertFalse(body.contains("!state.isBaby"),
			"baby armadillos must use the copied ArmadilloModel body route");
		assertTrue(body.contains("textures/entity/armadillo.png")
			&& body.contains("isStandaloneModelMeshEligible"),
			"armadillo age variants must retain bounded texture and mesh admission");
	}

	@Test
	void babySnifferUsesCopiedDiggingAndScentingBodyState() throws Exception {
		String worldSource = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		int method = worldSource.indexOf("public static boolean isVanillaSnifferModelMeshEligible");
		int next = worldSource.indexOf("\n\t/** Normal-model Nautiluses", method);
		assertTrue(method >= 0 && next > method, "sniffer admission method must remain explicit");
		String body = worldSource.substring(method, next);
		assertFalse(body.contains("!state.isBaby"),
			"baby sniffers must use the copied SnifferModel body route");
		assertTrue(body.contains("textures/entity/sniffer/sniffer.png")
			&& body.contains("isStandaloneModelMeshEligible"),
			"sniffer age variants must retain bounded texture and mesh admission");
	}

	@Test
	void babyNautilusUsesCopiedAgeSpecificBodyWhileEquipmentStaysSeparate() throws Exception {
		String worldSource = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		int method = worldSource.indexOf("public static boolean isVanillaNautilusModelMeshEligible");
		int next = worldSource.indexOf("\n\t/** Phantom body", method);
		assertTrue(method >= 0 && next > method, "nautilus admission method must remain explicit");
		String body = worldSource.substring(method, next);
		assertFalse(body.contains("!state.isBaby"),
			"baby nautiluses must use the copied NautilusModel body route");
		assertTrue(body.contains("textures/entity/nautilus/")
			&& body.contains("isStandaloneModelMeshEligible"),
			"nautilus age variants must retain age-specific texture and mesh admission");
	}

	@Test
	void zombieNautilusCoralSubclassUsesTheCopiedNautilusBodyRoute() throws Exception {
		String worldSource = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		String livingEntityRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/entity/LivingEntityRenderer.java"));
		int method = worldSource.indexOf("public static boolean isVanillaNautilusModelMeshEligible");
		int next = worldSource.indexOf("\n\t/** Phantom body", method);
		assertTrue(method >= 0 && next > method, "nautilus admission method must remain explicit");
		String body = worldSource.substring(method, next);
		assertTrue(body.contains("model instanceof net.minecraft.client.model.animal.nautilus.NautilusModel"),
			"zombie nautilus coral must use the copied NautilusModel subclass route");
		assertTrue(livingEntityRenderer.contains("this.model instanceof net.minecraft.client.model.animal.nautilus.NautilusModel"),
			"living entity dispatch must admit NautilusModel subclasses for zombie variants");
	}

	@Test
	void plainBabyDrownedUsesCopiedBodyWhileEquipmentAndSpecialStatesStayRejected() throws Exception {
		String worldSource = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		int method = worldSource.indexOf("public static boolean isVanillaDrownedModelMeshEligible");
		int next = worldSource.indexOf("\n\t/**\n\t * Armor stands", method);
		assertTrue(method >= 0 && next > method, "drowned admission method must remain explicit");
		String body = worldSource.substring(method, next);
		assertFalse(body.contains("!state.isBaby"),
			"plain baby drowned must use the copied DrownedModel body route");
		assertTrue(body.contains("state.rightHandItem.isEmpty()")
			&& body.contains("!state.isUsingItem")
			&& body.contains("!state.displayFireAnimation"),
			"drowned admission must retain copied empty-equipment and special-state fences");
	}

	@Test
	void rustArrowMeshUsesNeutralShaderIdentity() throws Exception {
		String worldSource = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		int enqueue = worldSource.indexOf("public static boolean enqueueArrowModel(");
		int instance = worldSource.indexOf("PENDING_MESH_INSTANCES.add", enqueue);
		int producer = worldSource.indexOf("PENDING_MESH_PRODUCERS.add(PendingMeshProducer.ARROW)", instance);
		assertTrue(enqueue >= 0 && instance > enqueue && producer > instance,
			"arrow route must publish an explicit indexed mesh instance");
		String body = worldSource.substring(instance, producer);
		assertTrue(body.contains("neutral identity") && body.contains("\n\t\t\t\t\t0,"),
			"arrow instances must use a Rust-owned neutral shader identity");
		assertFalse(body.contains("arrowRenderState.entityId"),
			"arrow instances must not pass Java runtime entity IDs into Rust semantics");
	}

	@Test
	void rustWholeFrameMovingAndItemRoutesCannotSilentlyDropSemanticSubmissions() throws Exception {
		String blockSource = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/feature/BlockFeatureRenderer.java"));
		String itemSource = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/item/ItemStackRenderState.java"));
		assertTrue(blockSource.contains("Rust whole-frame moving-block route has no semantic source"));
		assertTrue(blockSource.contains("Rust whole-frame Primed TNT route has no semantic mesh"));
		assertTrue(itemSource.contains("Rust whole-frame TACZ item route produced no semantic mesh"));
		assertTrue(itemSource.contains("Rust whole-frame special-item route produced no semantic mesh"));
		assertTrue(itemSource.contains("Rust whole-frame item route has no semantic mesh"));
		assertTrue(itemSource.contains("submitNodeCollector.submitItemSemantic("),
			"vanilla item state must enter the explicit semantic item callback");
	}

	@Test
	void rustWholeFrameItemHandoffCannotReachJavaSpecialRendererBeforeSelection() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/item/ItemStackRenderState.java"));
		int submit = source.indexOf("void submit(PoseStack poseStack");
		int gate = source.indexOf("if (!submitNodeCollector.isSemanticCoverageOnly()", submit);
		int ownershipInputs = source.indexOf("&& (selectedVulkan || rustWholeFrameHandoff || indexedItemScope)", gate);
		int handoff = source.indexOf("Rust whole-frame item route requires Vulkan selection", gate);
		int special = source.indexOf("this.specialRenderer.submit(", handoff);
		assertTrue(submit >= 0 && gate > submit && handoff > gate && special > handoff,
			"item submission must reject the pre-selection handoff before any Java special renderer call");
		assertTrue(ownershipInputs > gate && ownershipInputs < handoff,
			"only non-drawing coverage replay may bypass native item ownership");
	}

	@Test
	void rustWholeFrameFirstPersonOwnershipCannotResolveToJavaDuringHandoff() throws Exception {
		String policy = Files.readString(Path.of("src/main/java/net/vulkanic/world/WorldRenderRoutePolicy.java"));
		int method = policy.indexOf("public static Route currentFirstPersonItemOwnershipRoute()");
		int disabled = policy.indexOf("rustGalFirstPersonItem.disabled", method);
		int wholeFrame = policy.indexOf("RustGalVulkanWholeFrameMode.enabled()", disabled);
		int legacy = policy.indexOf("rustGalFirstPersonItem.legacyControl", wholeFrame);
		assertTrue(method >= 0 && wholeFrame > disabled && legacy > wholeFrame,
			"first-person ownership must prefer the Rust handoff route before legacy Java compatibility");
		assertTrue(policy.substring(wholeFrame, legacy).contains("Route.RUST_VULKAN_WHOLE_FRAME"),
			"pre-selection Rust ownership must not resolve first-person items to Java");
	}

	@Test
	void rustWholeFrameExperienceOrbAndItemRoutesPrecedeLegacyDiagnostics() throws Exception {
		String policy = Files.readString(Path.of("src/main/java/net/vulkanic/world/WorldRenderRoutePolicy.java"));
		int orb = policy.indexOf("public static Route currentExperienceOrbRoute()");
		int orbDisabled = policy.indexOf("rustGalWorldExperienceOrb.disabled", orb);
		int orbShell = policy.indexOf("RustGalVulkanWholeFrameMode.enabled()", orbDisabled);
		int orbLegacy = policy.indexOf("rustGalWorldExperienceOrb.legacyControl", orbShell);
		assertTrue(orb >= 0 && orbShell > orbDisabled && orbLegacy > orbShell,
			"experience-orb ownership must prefer the Rust handoff before legacy compatibility");

		int item = policy.indexOf("public static Route currentItemEntityMeshRoute(boolean eligible)");
		int itemDisabled = policy.indexOf("rustGalWorldItemEntity.disabled", item);
		int itemShell = policy.indexOf("RustGalVulkanWholeFrameMode.enabled()", itemDisabled);
		int itemLegacy = policy.indexOf("rustGalWorldItemEntity.legacyControl", itemShell);
		assertTrue(item >= 0 && itemShell > itemDisabled && itemLegacy > itemShell,
			"item-entity ownership must prefer the Rust handoff before legacy compatibility");
	}

	@Test
	void selectedVulkanMovingBlockCollectorsRemainBoundedBeforeShellActivation() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/SubmitNodeCollection.java"));
		assertTrue(source.contains("(net.vulkanic.VulkanicAPI.isVulkanBackendSelected()\n\t\t\t|| net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled())")
			&& source.contains("MAX_RUST_SEMANTIC_MODEL_SUBMITS")
			&& source.contains("this.modelSubmits.totalSubmitCount() >= MAX_RUST_SEMANTIC_MODEL_SUBMITS")
			&& source.contains("this.movingBlockSubmits.size() >= MAX_RUST_SEMANTIC_BLOCK_SUBMITS")
			&& source.contains("this.blockSubmits.size() >= MAX_RUST_SEMANTIC_BLOCK_SUBMITS")
			&& source.contains("this.hitboxSubmits.size() >= MAX_RUST_SEMANTIC_FEATURE_SUBMITS")
			&& source.contains("this.shadowSubmits.size() >= MAX_RUST_SEMANTIC_FEATURE_SUBMITS")
			&& source.contains("this.flameSubmits.size() >= MAX_RUST_SEMANTIC_FEATURE_SUBMITS")
			&& source.contains("this.leashSubmits.size() >= MAX_RUST_SEMANTIC_FEATURE_SUBMITS")
			&& source.contains("if (net.vulkanic.VulkanicAPI.isVulkanBackendSelected()\n\t\t\t|| net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled())")
			&& source.contains("enqueueFabricMeshItem("),
			"selected Vulkan must enforce bounded moving/block/entity-feature collection before the diagnostic shell flag is raised");
	}

	@Test
	void unknownMovingBlockCannotFallThroughToJavaVulkanTessellation() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/feature/BlockFeatureRenderer.java"));
		int guard = source.indexOf("!fallingBlock && !piston");
		int rustRoute = source.indexOf("enqueueUnknownMovingBlock", guard);
		int tessellate = source.indexOf("tesselateBlock", guard);
		assertTrue(guard >= 0 && rustRoute > guard && tessellate > rustRoute,
			"unknown moving blocks must try the explicit Rust semantic route before Java tessellation");
	}

	@Test
	void rustWholeFrameDoesNotSilentlyDropVoxelMapThreeDWaypoints() throws Exception {
		String gameRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
		String voxelConstants = Files.readString(Path.of("src/main/java/net/voxelmap/VoxelConstants.java"));
		String waypointManager = Files.readString(Path.of("src/main/java/net/voxelmap/WaypointManager.java"));
		assertTrue(gameRenderer.contains("assertRustWholeFrameWaypointsSupported()"),
			"Rust whole-frame extraction must account for VoxelMap 3D waypoint ownership");
		assertTrue(voxelConstants.contains("submitRustWaypointSemantics"),
			"VoxelMap waypoint signs/icons/labels must enter the Rust semantic world-text route");
		assertTrue(waypointManager.contains("hasRenderableWaypoints()"),
			"waypoint activity must be derived from semantic waypoint state");
		assertTrue(voxelConstants.contains("enqueueVoxelMapBeaconSegments")
			&& voxelConstants.contains("showBeacons && !manager.options.showWaypoints"),
			"beacon-only VoxelMap overlays must use the explicit Rust line route when waypoint labels are hidden");
		String routePolicy = Files.readString(Path.of("src/main/java/net/vulkanic/world/WorldRenderRoutePolicy.java"));
		int voxelBeaconRoute = routePolicy.indexOf("public static Route currentVoxelMapBeaconRoute()");
		assertTrue(voxelBeaconRoute >= 0);
		String voxelBeaconBody = routePolicy.substring(voxelBeaconRoute, routePolicy.indexOf("\n\t}\n", voxelBeaconRoute));
		assertTrue(voxelBeaconBody.contains("RustGalVulkanWholeFrameMode.enabled()"),
			"VoxelMap beacon ownership must stay Rust-owned during the pre-selection whole-frame handoff");
	}

	@Test
	void rustWholeFrameOwnsVanillaThreeDDebugCrosshairSemantics() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
		int debugCheck = source.indexOf("DebugScreenEntries.THREE_DIMENSIONAL_CROSSHAIR", source.indexOf("renderRustVulkanWholeFrameShell"));
		int enqueue = source.indexOf("enqueueThreeDimensionalDebugCrosshair", debugCheck);
		String primitive = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		assertTrue(debugCheck >= 0 && enqueue > debugCheck && primitive.contains("enqueueThreeDimensionalDebugCrosshair"),
			"Rust whole-frame must submit the 3D debug crosshair as explicit line semantics");
		String overlay = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/components/DebugScreenOverlay.java"));
		int overlayMethod = overlay.indexOf("public void render3dCrosshair(Camera camera)");
		int overlayEnqueue = overlay.indexOf("enqueueThreeDimensionalDebugCrosshair", overlayMethod);
		assertTrue(overlayMethod >= 0 && overlayEnqueue > overlayMethod,
			"the debug overlay callsite must invoke the Rust semantic crosshair producer instead of throwing before extraction");
	}

	@Test
	void disabledVulkanMaterialRouteSkipsJavaHitboxAndItemFeatureRenderers() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/feature/FeatureRenderDispatcher.java"));
		int hitbox = source.indexOf("hitboxFeatureRenderer.render");
		int hitboxGate = source.lastIndexOf("isVulkanBackendSelected()", hitbox);
		int item = source.indexOf("itemFeatureRenderer.render");
		int itemGate = source.lastIndexOf("isVulkanBackendSelected()", item);
		assertTrue(hitboxGate >= 0 && hitbox > hitboxGate && itemGate >= 0 && item > itemGate,
			"disabled Vulkan routes must not invoke Java hitbox or item feature draws");
	}

	@Test
	void rustBridgeIsOnlyRoutedFromSubsystemBenchmarkControls() throws Exception {
		String subsystem = Files.readString(Path.of("src/main/java/net/minecraft/client/dev/GraphicsSubsystemBenchmark.java"));
		String bridge = Files.readString(Path.of("src/main/java/net/minecraft/client/dev/RustGraphicsSubsystemBenchmark.java"));

		assertTrue(subsystem.contains("rust-vulkan") && subsystem.contains("rust-opengl"));
		assertTrue(subsystem.contains("RustGalVulkanWholeFrameMode.enabled()")
			&& subsystem.contains("RustGraphicsSubsystemBenchmark.run(minecraft, STATUS_PATH, ITERATIONS, \"rust-vulkan\")"),
			"whole-frame Vulkan must redirect the developer benchmark away from Java render passes");
		assertTrue(bridge.contains("Rust VulkanicGAL bridge"));
		assertTrue(bridge.contains("awaitTracyCaptureGrace()"));
		assertTrue(bridge.contains("Boolean.getBoolean(\"mattmc.dev.tracyCapture\")"));
		assertTrue(bridge.contains("mattmc.dev.graphicsSubsystemBenchmark.tracyGraceMillis"));
	}

	@Test
	void bridgePollsCompletionBeforeReadbackAndReportsFinalFfiMetrics() throws Exception {
		String bridge = Files.readString(Path.of("src/main/java/net/vulkanic/bridge/VulkanicGalBridge.java"));
		String benchmark = Files.readString(Path.of("src/main/java/net/minecraft/client/dev/RustGraphicsSubsystemBenchmark.java"));

		assertTrue(bridge.contains("mattmc_vulkanic_gal_completion_query"));
		assertTrue(benchmark.indexOf("pollCompletion(bridge, submission)") < benchmark.indexOf("bridge.readback(submission"));
		assertTrue(benchmark.contains("VulkanicGalBridge.Status retireStatus = bridge.retire(submission)"));
		assertTrue(benchmark.contains("\\\"completionPolls\\\""));
	}

	@Test
	void subsystemBridgeBarriersUseSemanticUsageStatesOnly() throws Exception {
		String bridge = Files.readString(Path.of("src/main/java/net/vulkanic/bridge/VulkanicGalBridge.java"));
		String benchmark = Files.readString(Path.of("src/main/java/net/minecraft/client/dev/RustGraphicsSubsystemBenchmark.java"));
		String rustFfi = readRustFfiModules();

		assertTrue(benchmark.contains(".barrier(handles.sampledTexture, VulkanicGalBridge.USAGE_TRANSFER_DST, VulkanicGalBridge.USAGE_SHADER_READ, true)"));
		assertTrue(benchmark.contains("layout(std430, binding = 3) readonly buffer Storage3"));
		assertTrue(benchmark.contains("layout(set = 0, binding = 3, std430) readonly buffer Storage3"));
		assertTrue(bridge.contains("Struct.BARRIER.setInt(barrier, 4, before);"));
		assertTrue(bridge.contains("Struct.BARRIER.setInt(barrier, 5, after);"));
		assertTrue(bridge.contains("Struct.BARRIER.setInt(barrier, 6, 0);"));
		assertTrue(bridge.contains("Struct.BARRIER.setInt(barrier, 7, 0);"));
		assertFalse(bridge.contains("Struct.BARRIER.setInt(barrier, 6, STAGE"));
		assertFalse(bridge.contains("Struct.BARRIER.setInt(barrier, 7, ACCESS"));
		assertTrue(rustFfi.contains("resource barrier stage/access bits are deprecated"));
	}

	@Test
	void rustOpenGlContextFallbackIsExplicitAndDoesNotUseProductionCallsites() throws Exception {
		String context = Files.readString(Path.of("src/main/rust/render/vulkanic/backends/opengl/context.rs"));
		String subsystem = Files.readString(Path.of("src/main/java/net/minecraft/client/dev/GraphicsSubsystemBenchmark.java"));

		assertTrue(context.contains("EGL:") && context.contains("GLX:"));
		assertTrue(context.contains("glXCreatePbuffer"));
		assertTrue(subsystem.contains("RustGraphicsSubsystemBenchmark.run"));
		assertTrue(subsystem.contains("minecraft.stop()"));
	}

	@Test
	void guiSpriteBatchingPreservesIncompatibleStratumAndStateBoundaries() throws Exception {
		String rustGuiFrontend = Files.readString(Path.of("src/main/rust/render/vulkanic/gui_frontend.rs"));
		assertEquals(ArmorIconState.EMPTY, RustGalGuiRenderer.armorIconStateForTests(0, 0));
		assertEquals(ArmorIconState.HALF, RustGalGuiRenderer.armorIconStateForTests(1, 0));
		assertEquals(ArmorIconState.FULL, RustGalGuiRenderer.armorIconStateForTests(2, 0));
		assertEquals(ArmorIconState.EMPTY, RustGalGuiRenderer.armorIconStateForTests(18, 9));
		assertEquals(ArmorIconState.HALF, RustGalGuiRenderer.armorIconStateForTests(19, 9));
		assertEquals(ArmorIconState.FULL, RustGalGuiRenderer.armorIconStateForTests(20, 9));
		assertTrue(rustGuiFrontend.contains("texture(Sampler0, v_uv)"));
		assertTrue(rustGuiFrontend.contains("fn packed_uniform_bytes"));
		assertTrue(rustGuiFrontend.contains("MAX_CUSTOM_POST_EFFECT_UNIFORM_BYTES: usize = 1024 * 1024")
			&& rustGuiFrontend.contains("MAX_CUSTOM_POST_EFFECT_UNIFORM_GRAPH_BYTES: usize = 2 * 1024 * 1024")
			&& rustGuiFrontend.contains("custom post-effect graph uniform bytes exceed bounded limit"),
			"Rust custom post effects must bound both per-pass and aggregate uniform payloads");
		assertTrue(rustGuiFrontend.contains("SpriteBatch"));
		for (int armor = 0; armor <= 20; armor++) {
			for (int icon = 0; icon < 10; icon++) {
				int threshold = icon * 2 + 1;
				ArmorIconState expected = threshold < armor
					? ArmorIconState.FULL
					: threshold == armor ? ArmorIconState.HALF : ArmorIconState.EMPTY;
				assertEquals(expected, RustGalGuiRenderer.armorIconStateForTests(armor, icon), "armor=" + armor + " icon=" + icon);
			}
		}
		assertThrows(IllegalArgumentException.class, () -> new PlayerHeartRequest(
			PlayerHeartVariant.NORMAL,
			GuiHeartState.CONTAINER,
			false,
			false,
			0,
			0,
			0
		));
		assertThrows(IllegalArgumentException.class, () -> new PlayerHeartRequest(
			PlayerHeartVariant.CONTAINER,
			GuiHeartState.FULL,
			false,
			false,
			0,
			0,
			0
		));
		assertThrows(IllegalArgumentException.class, () -> new PlayerHeartRequest(
			PlayerHeartVariant.NORMAL,
			GuiHeartState.FULL,
			false,
			false,
			-1,
			0,
			0
		));
		for (PlayerHeartVariant variant : List.of(
			PlayerHeartVariant.NORMAL,
			PlayerHeartVariant.POISONED,
			PlayerHeartVariant.WITHERED,
			PlayerHeartVariant.FROZEN
		)) {
			for (GuiHeartState state : List.of(
				GuiHeartState.HALF,
				GuiHeartState.FULL
			)) {
				for (boolean hardcore : List.of(false, true)) {
					for (boolean flashing : List.of(false, true)) {
						new PlayerHeartRequest(variant, state, hardcore, flashing, 1, 2, 3);
					}
				}
			}
		}
		new PlayerHeartRequest(
			PlayerHeartVariant.CONTAINER,
			GuiHeartState.CONTAINER,
			true,
			true,
			0,
			2,
			3
		);
		assertThrows(IllegalArgumentException.class, () -> new AbsorptionHeartRequest(
			AbsorptionHeartVariant.ABSORBING,
			GuiHeartState.CONTAINER,
			false,
			false,
			0,
			0,
			0
		));
		assertThrows(IllegalArgumentException.class, () -> new AbsorptionHeartRequest(
			AbsorptionHeartVariant.CONTAINER,
			GuiHeartState.FULL,
			false,
			false,
			0,
			0,
			0
		));
		for (AbsorptionHeartVariant variant : List.of(AbsorptionHeartVariant.ABSORBING, AbsorptionHeartVariant.WITHERED)) {
			for (GuiHeartState state : List.of(GuiHeartState.HALF, GuiHeartState.FULL)) {
				new AbsorptionHeartRequest(variant, state, true, false, 1, 2, 3);
			}
		}
		new AbsorptionHeartRequest(AbsorptionHeartVariant.CONTAINER, GuiHeartState.CONTAINER, true, true, 0, 2, 3);
		assertThrows(IllegalArgumentException.class, () -> new HungerIconRequest(
			HungerIconVariant.NORMAL,
			HungerIconState.FULL,
			false,
			2,
			0,
			0,
			0
		));
		assertThrows(IllegalArgumentException.class, () -> new HungerIconRequest(
			HungerIconVariant.NORMAL,
			HungerIconState.FULL,
			false,
			0,
			-1,
			0,
			0
		));
		for (HungerIconVariant variant : HungerIconVariant.values()) {
			for (HungerIconState state : HungerIconState.values()) {
				new HungerIconRequest(variant, state, true, -1, 1, 2, 3);
				new HungerIconRequest(variant, state, false, 0, 1, 2, 3);
				new HungerIconRequest(variant, state, true, 1, 1, 2, 3);
			}
		}
		assertThrows(IllegalArgumentException.class, () -> new AirBubbleRequest(
			AirBubbleState.FULL,
			true,
			true,
			0,
			0,
			0
		));
		assertThrows(IllegalArgumentException.class, () -> new AirBubbleRequest(
			AirBubbleState.EMPTY,
			false,
			true,
			-1,
			0,
			0
		));
		new AirBubbleRequest(AirBubbleState.FULL, false, true, 0, 1, 2);
		new AirBubbleRequest(AirBubbleState.PARTIAL, true, true, 1, 1, 2);
		new AirBubbleRequest(AirBubbleState.EMPTY, false, true, 2, 1, 2);
		assertThrows(IllegalArgumentException.class, () -> new MountHeartRequest(
			MountHeartVariant.VEHICLE,
			MountHeartState.FULL,
			true,
			-1,
			0,
			0,
			0
		));
		assertThrows(IllegalArgumentException.class, () -> new MountHeartRequest(
			MountHeartVariant.VEHICLE,
			MountHeartState.FULL,
			true,
			0,
			-1,
			0,
			0
		));
		for (MountHeartState state : MountHeartState.values()) {
			new MountHeartRequest(MountHeartVariant.VEHICLE, state, true, 1, 2, 3, 4);
		}
		new AirBubbleRequest(AirBubbleState.EMPTY, false, false, 3, 1, 2);
		assertTrue(rustGuiFrontend.contains("GUI_MAX_PACKED_SPRITES"));
		assertTrue(rustGuiFrontend.contains("CommandOp::CopyBufferToTexture"));
	}

	@Test
	void frameAbiV29PreservesFogProjectionAndAddsTypedGuiTiles() throws Exception {
		String bridge = Files.readString(Path.of("src/main/java/net/vulkanic/bridge/VulkanicGalBridge.java"));
		String world = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		String queue = Files.readString(Path.of("src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"));
		String rustGuiFrontend = Files.readString(Path.of("src/main/rust/render/vulkanic/gui_frontend.rs"));
		String ffiContext = Files.readString(Path.of("src/main/rust/render/vulkanic/ffi/context.rs"));
		String ffiGui = Files.readString(Path.of("src/main/rust/render/vulkanic/ffi/gui.rs"));
		String lodFrontend = Files.readString(Path.of("src/main/rust/render/vulkanic/world_primitive_frontend.rs"));
		String ffiWorld = Files.readString(Path.of("src/main/rust/render/vulkanic/ffi/world.rs"));
		String ffiMaterial = Files.readString(Path.of("src/main/rust/render/vulkanic/ffi/material.rs"));
		String gameRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
		String gui = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/Gui.java"));
		String guiRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/render/GuiRenderer.java"));
		String experienceBar = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/contextualbar/ExperienceBarRenderer.java"));
		String bossOverlay = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/components/BossHealthOverlay.java"));

		assertEquals(54, VulkanicGalBridge.ABI_VERSION);
		assertTrue(bridge.contains("GUI_TILED_QUAD_REQUEST(101)"));
		assertTrue(bridge.contains("Struct.WHOLE_FRAME_SUBMIT.setFloat(request, 34, guiProjection.width())"));
		assertTrue(bridge.contains("Struct.GUI_FRAME_SUBMIT.setFloat(request, 10, guiProjection.width())"));
		assertTrue(bridge.contains("GUI_AFFINE_QUAD_REQUEST(92)"));
		assertTrue(bridge.contains("WORLD_TEXT_QUAD_REQUEST(93)"));
		assertTrue(bridge.contains("WORLD_TEXT_IMAGE_ASSET_PAYLOAD(94)"));
		assertTrue(bridge.contains("record WorldTextImageAssetRecord"));
		assertTrue(bridge.contains("record WorldTextQuadRecord"));
		assertTrue(bridge.contains("record GuiAffineQuadRecord"));
		assertTrue(bridge.contains("int midBlockPacked"));
		assertTrue(bridge.contains("record WorldVoxelVolumeFrameRecord"));
		assertTrue(bridge.contains("voxel-volume camera coordinates must be finite"));
		assertTrue(bridge.contains("record WorldShaderEnvironmentFrameRecord"));
		assertTrue(bridge.contains("record WorldLodRenderFrameRecord"));
		assertTrue(bridge.contains("float[] modelViewMatrix"));
		assertTrue(bridge.contains("float[] projectionMatrix"));
		assertTrue(bridge.contains("float[] projectionInverseMatrix"));
		assertTrue(bridge.contains("WORLD_VOXEL_VOLUME_FRAME(72)"));
		assertTrue(bridge.contains("WORLD_SHADER_ENVIRONMENT_FRAME(73)"));
		assertTrue(bridge.contains("WORLD_FIRST_PERSON_FRAME(98)"));
		assertTrue(bridge.contains("whole-frame view matrix must contain finite values")
			&& bridge.contains("whole-frame projection matrix must contain finite values"));
		assertTrue(bridge.contains("record WorldFirstPersonFrameRecord"));
		assertTrue(bridge.contains("float[] modelViewMatrix"));
		assertTrue(bridge.contains("first-person projection matrix must contain finite values")
			&& bridge.contains("first-person model-view matrix must contain finite values"));
		assertTrue(bridge.contains("firstPersonMeshInstances"));
		assertTrue(bridge.contains("voxelVolumeFrame.worldGeneration()"));
		assertTrue(bridge.contains("voxelVolumeFrame.cameraX()"));
		assertTrue(bridge.contains("shaderEnvironmentFrame.timeOfDay()"));
		assertTrue(bridge.contains("shaderEnvironmentFrame.eyeSubmersion()"));
		assertTrue(bridge.contains("shaderEnvironmentFrame.farPlane()"));
		assertTrue(bridge.contains("shaderEnvironmentFrame.skyColorRed()"));
		assertTrue(bridge.contains("shaderEnvironmentFrame.darknessLightFactor()"));
		assertTrue(bridge.contains("shaderEnvironmentFrame.blindness()"));
		assertTrue(bridge.contains("shaderEnvironmentFrame.darknessFactor()"));
		assertTrue(bridge.contains("shaderEnvironmentFrame.eyeBrightnessBlock()"));
		assertTrue(bridge.contains("shaderEnvironmentFrame.eyeBrightnessSky()"));
		assertTrue(bridge.contains("shaderEnvironmentFrame.fogParameterColorRed()"));
		assertTrue(bridge.contains("shaderEnvironmentFrame.fogEnvironmentalStart()"));
		assertTrue(bridge.contains("shaderEnvironmentFrame.fogRenderDistanceEnd()"));
		assertTrue(bridge.contains("shaderEnvironmentFrame.nightVision()"));
		assertTrue(bridge.contains("shaderEnvironmentFrame.fogColorRed()"));
		assertTrue(bridge.contains("shaderEnvironmentFrame.biomePrecipitation()"));
		assertTrue(bridge.contains("shaderEnvironmentFrame.biomeResourceLocation()"));
		assertTrue(bridge.contains("shaderEnvironmentFrame.mainHandItemModelResourceLocation()"));
		assertTrue(bridge.contains("shaderEnvironmentFrame.offHandItemModelResourceLocation()"));
		assertTrue(bridge.contains("shaderEnvironmentFrame.mainHandItemLightEmission()"));
		assertTrue(bridge.contains("shaderEnvironmentFrame.offHandItemLightEmission()"));
		assertFalse(bridge.contains("IrisShaderEnvironmentFrame"));
		assertTrue(world.contains("level.getSkyColor(camera.getPosition(), shaderPackFramePartialTick)"));
		assertTrue(world.contains("shaderPackFogColor(level, camera)"));
		assertTrue(world.contains("shaderPackFogParameters(level, camera)"));
		assertTrue(world.contains("collectFogParametersForRust("));
		assertFalse(world.contains("fogRenderer.sodium$getFogParameters()"));
		assertFalse(world.contains("FogStorage"));
		assertFalse(world.contains("FogParameters.NONE"));
		assertTrue(world.contains("shaderPackBlindness()"));
		assertTrue(world.contains("shaderPackDarknessFactor()"));
		assertTrue(world.contains("shaderPackEyeBrightness()"));
		assertTrue(world.contains("fogRenderer.computeFogColorSemantic("),
			"Rust shader-environment fog extraction must use the Iris-free semantic color path");
		assertTrue(world.contains("shaderPackBiomePrecipitation(level, camera)"));
		assertTrue(world.contains("shaderPackBiomeResourceLocation(level, camera)"));
		assertTrue(world.contains("shaderPackHeldItemModelResourceLocation("));
		assertTrue(world.contains("shaderPackHeldItemLightEmission("));
		assertFalse(world.contains("IrisItemLightProvider"),
			"Rust held-item light semantics must use vanilla item data rather than an Iris API");
		assertTrue(world.contains("DataComponents.ITEM_MODEL"));
		assertTrue(world.contains("BuiltInRegistries.ITEM.getKey"));
		assertTrue(world.contains("Biome.Precipitation precipitation"));
		assertFalse(world.contains("CapturedRenderingState.INSTANCE.getFogColor"));
		assertTrue(world.contains("camera.getFluidInCamera()"));
		assertTrue(world.contains("shaderPackDarknessLightFactor()"));
		assertTrue(world.contains("shaderPackNightVision()"));
		assertFalse(world.contains("CapturedRenderingState"));
		assertTrue(bridge.contains("Struct.WORLD_MESH_VERTEX.setInt(vertexItem, 13, vertex.terrainMaterialBits())")
			&& bridge.contains("Struct.WORLD_MESH_VERTEX.setInt(vertexItem, 14, vertex.midBlockPacked())"),
			"ABI v27 must preserve both explicit terrain material bits and packed mid-block identity");
		assertTrue(bridge.contains("mattmc_vulkanic_gal_context_create_borrowed_opengl"));
		assertTrue(bridge.contains("mattmc_vulkanic_gal_frame_acquire"));
		assertTrue(bridge.contains("mattmc_vulkanic_gal_frame_present"));
		assertTrue(bridge.contains("mattmc_vulkanic_gal_gui_submit_frame"));
		assertTrue(ffiContext.contains("MAX_BRIDGE_CONTEXTS: usize = 16"));
		assertTrue(ffiContext.contains("registry.contexts.len() >= MAX_BRIDGE_CONTEXTS"));
		assertTrue(rustGuiFrontend.contains("GUI_MAX_RAW_IMAGES: usize = 4_096"));
		assertTrue(rustGuiFrontend.contains("GUI_MAX_MESH_RASTER_RESOURCES: usize = 4_096")
			&& rustGuiFrontend.contains("GUI_MAX_MESH_COMPOSITE_RESOURCES: usize = 256")
			&& rustGuiFrontend.contains("GUI mesh raster resource cache exceeds bounded limit")
			&& rustGuiFrontend.contains("GUI mesh composite resource cache exceeds bounded limit"));
		assertTrue(ffiGui.contains("raw GUI image payload count"));
		assertTrue(lodFrontend.contains("WORLD_LOD_MAX_COLUMNS: usize = 512"));
		assertTrue(lodFrontend.contains("projected_columns > WORLD_LOD_MAX_COLUMNS"));
		assertTrue(lodFrontend.contains("WORLD_LOD_MAX_VISIBLE_SEGMENTS: usize = 16_384"));
		assertTrue(lodFrontend.contains("frame.lod_instances.len() > WORLD_LOD_MAX_VISIBLE_SEGMENTS"));
		assertTrue(lodFrontend.contains("WORLD_LOD_MAX_SOURCE_MESH_CACHE: usize = 16_384"));
		assertTrue(lodFrontend.contains("self.lod_voxel_source_meshes.len() >= WORLD_LOD_MAX_SOURCE_MESH_CACHE"));
		assertTrue(ffiWorld.contains("world LOD column asset count"));
		assertTrue(ffiWorld.contains("WORLD_LOD_MAX_VISIBLE_SEGMENTS"));
		assertTrue(lodFrontend.contains("WORLD_MESH_ASSET_RESIDENCY: usize = 16_384"));
		assertTrue(lodFrontend.contains("WORLD_MESH_TEXTURE_RESIDENCY: usize = 8_192"));
		assertTrue(lodFrontend.contains("WORLD_MESH_RESOURCE_RESIDENCY: usize = 65_536"));
		assertTrue(lodFrontend.contains("WORLD_MESH_GEOMETRY_RESIDENCY: usize = 16_384"));
		assertTrue(lodFrontend.contains("WORLD_SOURCE_MESH_RESOURCE_RESIDENCY: usize = 16_384"));
		assertTrue(lodFrontend.contains("WORLD_MESH_PIPELINE_RESIDENCY: usize = 4_096"));
		assertTrue(ffiMaterial.contains("world mesh asset payload count"));
		assertTrue(ffiMaterial.contains("world mesh texture payload count"));
		assertTrue(bridge.contains("record GuiSpriteRecord"));
		assertEquals(1, VulkanicGalBridge.HANDLE_BUFFER);
		assertEquals(2, VulkanicGalBridge.HANDLE_TEXTURE);
		assertEquals(3, VulkanicGalBridge.HANDLE_TEXTURE_VIEW);
		assertEquals(4, VulkanicGalBridge.HANDLE_SAMPLER);
		assertEquals(5, VulkanicGalBridge.HANDLE_SHADER_MODULE);
		assertEquals(6, VulkanicGalBridge.HANDLE_RESOURCE_LAYOUT);
		assertEquals(7, VulkanicGalBridge.HANDLE_RESOURCE_SET);
		assertEquals(8, VulkanicGalBridge.HANDLE_PIPELINE_LAYOUT);
		assertEquals(9, VulkanicGalBridge.HANDLE_GRAPHICS_PIPELINE);
		assertEquals(13, VulkanicGalBridge.HANDLE_FRAME_TARGET);
		assertTrue(queue.contains("GUI_CROSSHAIR"));
		assertTrue(queue.contains("GUI_HOTBAR_BASE"));
		assertTrue(queue.contains("GUI_HOTBAR_SELECTION"));
		assertTrue(queue.contains("GUI_ARMOR"));
		assertTrue(queue.contains("GUI_EXPERIENCE_BAR_BACKGROUND"));
		assertTrue(queue.contains("GUI_EXPERIENCE_BAR_PROGRESS"));
		assertTrue(queue.contains("GUI_BOSS_BAR_BACKGROUND"));
		assertTrue(queue.contains("GUI_BOSS_BAR_PROGRESS"));
		assertTrue(queue.contains("GUI_PLAYER_HEALTH"));
		assertTrue(queue.contains("HOTBAR_BASE"));
		assertTrue(queue.contains("HOTBAR_SELECTION"));
		assertTrue(queue.contains("EXPERIENCE_BAR_BACKGROUND"));
		assertTrue(queue.contains("EXPERIENCE_BAR_PROGRESS"));
		assertTrue(queue.contains("enqueuePlayerHearts"));
		assertTrue(queue.contains("enqueueHotbarBase"));
		assertTrue(queue.contains("enqueueHotbarSelection"));
		assertTrue(queue.contains("enqueueArmorIcons"));
		assertTrue(queue.contains("enqueueExperienceBar"));
		assertTrue(queue.contains("enqueueBossBar"));
		assertFalse(bridge.contains("guiAlphaPipeline"));
		assertFalse(bridge.contains("guiInvertPipeline"));
		assertTrue(rustGuiFrontend.contains("TextureGroup::Alpha"));
		assertFalse(queue.contains("DeferredBatchScheduler"));
		String frameCoordinator = Files.readString(Path.of("src/main/java/net/vulkanic/gui/RustGalFrameCoordinator.java"));
		assertFalse(queue.contains("RustGalFrameScheduler<VulkanicGalBridge.GuiSpriteRecord>"));
		assertTrue(frameCoordinator.contains("RustGalFrameScheduler<QueuedGuiRequest>"));
		assertTrue(frameCoordinator.contains("enqueueGuiAffineQuadRequest"));
		assertFalse(queue.contains("record CachedResources"));
		assertFalse(queue.contains("record CacheKey"));
		assertFalse(Files.exists(Path.of("src/main/java/net/vulkanic/gui/GuiBatchBuilder.java")));
		assertFalse(Files.exists(Path.of("src/main/java/net/vulkanic/gui/GuiResourceCache.java")));
		assertFalse(Files.exists(Path.of("src/main/java/net/vulkanic/gui/GuiSpriteAtlas.java")));
		assertFalse(Files.exists(Path.of("src/main/java/net/vulkanic/gui/GuiPipelineLibrary.java")));
		assertTrue(frameCoordinator.contains("cacheHits"));
		assertTrue(frameCoordinator.contains("completionTimeouts"));
		assertFalse(frameCoordinator.contains("RETIRE_INTERVAL_FRAMES"));
		assertTrue(frameCoordinator.contains("if (!force)"));
		assertTrue(frameCoordinator.contains("rust_gal_frames_executed"));
		assertFalse(queue.contains("destroyHandles(created)"));
		assertTrue(rustGuiFrontend.contains("pub struct GuiFrontend"));
		assertTrue(rustGuiFrontend.contains("fn create_resources"));
		assertTrue(rustGuiFrontend.contains("fn build_atlas"));
		assertTrue(rustGuiFrontend.contains("fn packed_uniform_bytes"));
		assertTrue(rustGuiFrontend.contains("const VERTEX_SHADER"));
		assertTrue(rustGuiFrontend.contains("const FRAGMENT_SHADER"));
		assertTrue(frameCoordinator.contains("VulkanicGalBridge.isBorrowedOpenGlContextCurrent"));
		assertTrue(bridge.contains("GLFW.glfwGetCurrentContext()"));
		assertFalse(queue.contains("beginFramePass(frameResources.pass(), frameResources.target())"));
		assertFalse(queue.contains("frameResourcesFor("));
		assertFalse(queue.contains("GuiBatchBuilder.packCompatibleSpriteBatches"));
		assertTrue(frameCoordinator.contains("bridge.submitGuiFrame"));
		assertTrue(rustGuiFrontend.contains("CommandOp::DrawIndexed"));
		assertTrue(rustGuiFrontend.contains("TextureGroup::Alpha"));
		assertTrue(frameCoordinator.contains("rust_gal_sprite_batches_executed"));
		assertTrue(queue.contains("GuiExecutionRoute.JAVA_COMPATIBILITY"));
		assertTrue(queue.contains("Rust VulkanicGAL GUI enqueue requested while route is"));
		assertTrue(gameRenderer.contains("RustGalFrameCoordinator.resize"));
		assertTrue(gameRenderer.contains("RustGalFrameCoordinator.shutdown"));
		assertTrue(gui.contains("RustGalGuiRenderer.enqueueCrosshair"));
		assertTrue(gui.contains("RustGalGuiRenderer.enqueueHotbarBase"));
		assertTrue(gui.contains("RustGalGuiRenderer.enqueueArmorIcons"));
		assertTrue(experienceBar.contains("RustGalGuiRenderer.enqueueExperienceBar"));
		assertTrue(bossOverlay.contains("RustGalGuiRenderer.enqueueBossBar"));
		assertTrue(bossOverlay.contains("drawString(this.minecraft.font"));
		assertTrue(guiRenderer.contains("RustGalGuiElementRenderState"));
		assertTrue(guiRenderer.contains("RustGalFrameCoordinator.executeGuiFrame"));
		assertFalse(guiRenderer.contains("try (RenderPass ignored = VulkanicAPI.createRenderPass("),
			"Rust GUI execution must not be wrapped in a second Java Vulkan render pass"
		);
		assertTrue(
			guiRenderer.indexOf("RustGalFrameCoordinator.executeGuiFrame") < guiRenderer.indexOf("rustGalFrameExecuted.setTrue()"),
			"the combined Rust GUI frame should be marked executed only after the scoped render-pass submission"
		);
	}

	@Test
	void lodBridgeRejectsValuesOutsideRustLayerAndVertexDomains() {
		assertThrows(IllegalArgumentException.class, () -> new VulkanicGalBridge.WorldLodVertexRecord(
			0, 0, 0, 0, 0xffffffff, 16, 0));
		assertThrows(IllegalArgumentException.class, () -> new VulkanicGalBridge.WorldLodVertexRecord(
			0, 0, 0, 0, 0xffffffff, 0, 6));
		assertThrows(IllegalArgumentException.class, () -> new VulkanicGalBridge.WorldLodColumnInstanceRecord(
			1L, 1L, 5, 0, 0));
		assertThrows(IllegalArgumentException.class, () -> new VulkanicGalBridge.WorldLodSegmentMaterialProvenanceRecord(
			5, 0, new int[] {1}));
	}

	@Test
	void semanticGuiTextTransportStaysCopiedAndBackendNeutral() throws Exception {
		String guiRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/render/GuiRenderer.java"));
		String rustGui = Files.readString(Path.of("src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"));
		String coordinator = Files.readString(Path.of("src/main/java/net/vulkanic/gui/RustGalFrameCoordinator.java"));
		String bridge = Files.readString(Path.of("src/main/java/net/vulkanic/bridge/VulkanicGalBridge.java"));

		assertTrue(guiRenderer.contains("RustGalGuiRenderer.tryEnqueueText"));
		assertTrue(guiRenderer.contains("!RustGalGuiRenderer.isWholeFrameVulkanEnabled()"));
		int itemPreparation = guiRenderer.indexOf("private void prepareItemElements()");
		int itemPreparationEnd = guiRenderer.indexOf("private void prepareItemsViaPictureInPicture", itemPreparation);
		assertTrue(itemPreparation >= 0 && itemPreparationEnd > itemPreparation);
		assertFalse(
			guiRenderer.substring(itemPreparation, itemPreparationEnd).contains("this.prepareItemsViaPictureInPicture(i)"),
			"selected Vulkan GUI item collection must not prepare Java off-screen item/PIP renderers"
		);
		assertTrue(guiRenderer.contains("RustGalGuiRenderer.isWholeFrameVulkanEnabled()"));
		assertTrue(guiRenderer.contains("RustGalGuiRenderer.recordUnsupportedElement(\"text\")"));
		assertTrue(rustGui.contains("public static void recordUnsupportedElement(String elementKind)")
			&& rustGui.contains("public static void recordUnsupportedElementDetail(String detail)")
			&& rustGui.contains("if (isWholeFrameVulkanEnabled())"),
			"GUI unsupported-element diagnostics must be retained during pre-selection Rust handoff");
		assertTrue(
			guiRenderer.indexOf("RustGalGuiRenderer.isWholeFrameVulkanEnabled()")
				< guiRenderer.indexOf("RustGalGuiRenderer.recordUnsupportedElement(\"text\")"),
			"whole-frame GUI text extraction misses must be admitted as unsupported semantics before any Java glyph state is submitted"
		);
		assertTrue(rustGui.contains("FontTexture.semanticAtlasSnapshot"));
		assertTrue(rustGui.contains("GuiAffineQuadRecord"));
		assertTrue(rustGui.contains("List<TextAtlasRequest>"));
		assertTrue(rustGui.contains("request.withClip"));
		assertFalse(rustGui.contains("textState.scissor != null"));
		assertFalse(rustGui.contains("semantic-text-extraction-failed"));
		assertFalse(rustGui.contains("getTextureView("));
		assertFalse(rustGui.contains("RenderSystem."));
		assertTrue(coordinator.contains("enqueueGuiAffineQuadRequest"));
		assertTrue(coordinator.contains("SCHEDULER.takeAllItems"));
		assertTrue(coordinator.contains("withSequence(sequence)"));
		assertTrue(coordinator.contains("updateGuiRawImages"));
		assertTrue(bridge.contains("GUI_AFFINE_QUAD_REQUEST(92)"));
	}

	@Test
	void rejectedGuiPictureInPictureFamiliesRemainExplicitlyDiagnosed() throws Exception {
		String guiRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/render/GuiRenderer.java"));
		String semanticRenderer = Files.readString(Path.of("src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"));
		String modelCollector = Files.readString(Path.of("src/main/java/net/vulkanic/gui/GuiModelPipSemanticCollector.java"));
		assertTrue(guiRenderer.contains("picture-in-picture:entity"));
		assertTrue(guiRenderer.contains("picture-in-picture:skin"));
		assertTrue(guiRenderer.contains("picture-in-picture:book"));
		assertTrue(guiRenderer.contains("picture-in-picture:sign"));
		assertTrue(guiRenderer.contains("picture-in-picture:banner"));
		assertTrue(guiRenderer.contains("picture-in-picture:oversized-item"));
		assertTrue(guiRenderer.contains("MAX_RUST_PICTURE_IN_PICTURE_STATES = 1_024")
			&& guiRenderer.contains("Rust whole-frame GUI picture-in-picture bound exceeded"),
			"Rust GUI PIP extraction must bound copied inputs before model/atlas expansion");
		assertTrue(semanticRenderer.contains("EntityPipLayerCapture")
			&& semanticRenderer.contains("living.layers")
			&& semanticRenderer.contains("EntityPipLayerModel::layerOrder")
			&& semanticRenderer.contains("EntityPipLayerItem")
			&& semanticRenderer.contains("MAX_ENTITY_PIP_ITEM_QUADS = 1_024")
			&& semanticRenderer.contains("layerItem.foilType()")
			&& semanticRenderer.contains("collectBakedQuads")
			&& semanticRenderer.contains("layerModel.materialMode()")
			&& semanticRenderer.contains("if (name.contains(\"cutout\")) return 2")
			&& semanticRenderer.contains("BlendFunction.TRANSLUCENT")
			&& semanticRenderer.contains("sprite.atlasLocation()")
			&& semanticRenderer.contains("contains(\"glint\")")
			&& semanticRenderer.contains("new Model.Simple(modelPart")
			&& modelCollector.contains("FoilType.SPECIAL")
			&& modelCollector.contains("SPECIAL_FOIL_TEXTURE_SCALE"),
			"admitted GUI entity previews must preserve direct semantic renderer layers and their tint state");
	}

	@Test
	void unsupportedRustGuiSpecialItemsCannotFallThroughToFlatItemRendering() throws Exception {
		String guiRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/render/GuiRenderer.java"));
		int method = guiRenderer.indexOf("public void collectRustGalItemSemantics()");
		int next = guiRenderer.indexOf("public void collectRustGalPictureInPictureSemantics()", method);
		assertTrue(method >= 0 && next > method);
		String body = guiRenderer.substring(method, next);
		assertTrue(body.contains("boolean hasSpecialRenderer"));
		assertTrue(body.contains("item:special-renderer"));
		assertTrue(body.contains("if (hasSpecialRenderer)"));
		assertTrue(body.indexOf("if (hasSpecialRenderer)") < body.indexOf("boolean standard3dCandidate"),
			"unadmitted special items must not reach the generic flat/3D item routes");
	}

	@Test
	void taczFirstPersonUsesItsSemanticBedrockCollectorOnRustWholeFrame() throws Exception {
		String handRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/ItemInHandRenderer.java"));
		String taczRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/special/TaczGlock17SpecialRenderer.java"));
		assertTrue(taczRenderer.contains("submitSemanticBedrockRoots")
			&& taczRenderer.contains("submitTexturedQuads"),
			"TACZ must provide copied semantic Bedrock roots rather than a Java Vulkan draw");
		assertFalse(handRenderer.contains("These hand families have no Rust semantic collector yet"),
			"the first-person TACZ callsite must not discard an already implemented semantic collector");
		assertTrue(handRenderer.contains("renderTaczGlockFirstPerson"),
			"Rust whole-frame TACZ ownership must reach the semantic first-person producer");
		assertTrue(handRenderer.contains("if (itemStack.getItem() instanceof TaczMvpGunItem)"),
			"TACZ first-person ownership must invoke the semantic producer instead of recording an unavailable item");
	}

	@Test
	void taczItemEntitySpecialRendererUsesSemanticDispatchBeforeGenericRejection() throws Exception {
		String itemState = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/item/ItemStackRenderState.java"));
		assertTrue(itemState.contains("instanceof net.minecraft.client.renderer.special.TaczGlock17SpecialRenderer taczRenderer"),
			"TACZ item entities must reach their copied semantic producer");
		assertTrue(itemState.contains("taczRenderer.submit(") && itemState.contains("special-renderer"),
			"special item renderers must retain an explicit unsupported diagnostic boundary");
		assertTrue(itemState.contains("pendingSemanticItemSubmissionCount()")
			&& itemState.contains("special-renderer-semantic-receipt"),
			"vanilla special renderers must be admitted only after a copied Rust mesh receipt");
		String screenEffects = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/ScreenEffectRenderer.java"));
		assertTrue(screenEffects.contains("itemStackRenderState.submitSemantic("),
			"Rust item activation must use the explicit semantic item submission callsite");
		assertTrue(itemState.contains("public void submitSemantic("),
			"item state must expose an explicit semantic submission contract");
	}

	@Test
	void taczMuzzleFlashCannotFallBackToJavaGeometryWhenRustOwnsPresentation() throws Exception {
		String renderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/special/TaczGlock17SpecialRenderer.java"));
		int semantic = renderer.indexOf("submitTranslucentTexturedQuadSemantic");
		int unavailable = renderer.indexOf("Rust whole-frame TACZ muzzle flash route is unavailable", semantic);
		int javaGeometry = renderer.indexOf("submitNodeCollector.submitCustomGeometrySemantic", unavailable);
		assertTrue(semantic >= 0 && unavailable > semantic && javaGeometry > unavailable,
			"TACZ muzzle flashes must fail closed after semantic Rust admission is unavailable");
		int guard = renderer.lastIndexOf("VulkanicAPI.isVulkanBackendSelected()", unavailable);
		assertTrue(guard > semantic && renderer.substring(guard, unavailable).contains("RustGalVulkanWholeFrameMode.enabled()"),
			"muzzle-flash Java geometry must be fenced by both selected Vulkan and Rust presenter ownership");
	}

	@Test
	void taczSpecialRendererTreatsTheRustPresenterShellAsVulkanOwnership() throws Exception {
		String renderer = Files.readString(Path.of(
			"src/main/java/net/minecraft/client/renderer/special/TaczGlock17SpecialRenderer.java"));
		assertTrue(renderer.contains("boolean rustPresentation = VulkanicAPI.isVulkanBackendSelected()")
			&& renderer.contains("RustGalVulkanWholeFrameMode.enabled()"),
			"TACZ must fence Java geometry during the Rust presenter handoff, before backend selection is finalized");
		assertTrue(renderer.contains("&& rustPresentation\n\t\t\t&& !rustWholeFrame"),
			"TACZ must fail closed when the presenter shell owns the frame but semantic admission is unavailable");
		assertTrue(renderer.contains("Rust whole-frame TACZ attachment route is unavailable"),
			"TACZ attachments must not reopen Java custom geometry under the Rust presenter");
	}

	@Test
	void itemFrameBackingModelTreatsTheRustPresenterShellAsVulkanOwnership() throws Exception {
		String renderer = Files.readString(Path.of(
			"src/main/java/net/minecraft/client/renderer/entity/ItemFrameRenderer.java"));
		assertTrue(renderer.contains("submitBlockDisplaySemantic("),
			"item-frame backing geometry must have an explicit Rust semantic producer");
		assertTrue(renderer.contains("VulkanicAPI.isVulkanBackendSelected()")
			&& renderer.contains("RustGalVulkanWholeFrameMode.enabled()"),
			"item-frame ownership must include the Rust presenter handoff window");
	}

	@Test
	void ominousItemSpawnerUsesTheIndexedSemanticScopeDuringPresenterHandoff() throws Exception {
		String renderer = Files.readString(Path.of(
			"src/main/java/net/minecraft/client/renderer/entity/OminousItemSpawnerRenderer.java"));
		int submit = renderer.indexOf("ItemEntityRenderer.submitMultipleFromCount(");
		assertTrue(submit > 0, "ominous item spawner must retain its item-cluster producer");
		int begin = renderer.lastIndexOf("beginItemEntitySubmission()", submit);
		int end = renderer.indexOf("endItemEntitySubmission()", submit);
		assertTrue(begin >= 0 && end > submit,
			"ominous item clusters must stay inside the indexed semantic scope during presenter handoff");
	}

	@Test
	void fireworkItemSubmissionUsesTheIndexedSemanticScopeDuringPresenterHandoff() throws Exception {
		String renderer = Files.readString(Path.of(
			"src/main/java/net/minecraft/client/renderer/entity/FireworkEntityRenderer.java"));
		int submit = renderer.indexOf("fireworkRocketRenderState.item.submit(");
		assertTrue(submit > 0, "firework renderer must retain its item-state producer");
		int begin = renderer.lastIndexOf("beginItemEntitySubmission()", submit);
		int end = renderer.indexOf("endItemEntitySubmission()", submit);
		assertTrue(begin >= 0 && end > submit,
			"firework item submissions must stay inside the indexed semantic scope during presenter handoff");
	}

	@Test
	void thrownItemSubmissionUsesTheIndexedSemanticScopeDuringPresenterHandoff() throws Exception {
		String renderer = Files.readString(Path.of(
			"src/main/java/net/minecraft/client/renderer/entity/ThrownItemRenderer.java"));
		int submit = renderer.indexOf("thrownItemRenderState.item.submit(");
		assertTrue(submit > 0, "thrown-item renderer must retain its item-state producer");
		int begin = renderer.lastIndexOf("beginItemEntitySubmission()", submit);
		int end = renderer.indexOf("endItemEntitySubmission()", submit);
		assertTrue(begin >= 0 && end > submit,
			"thrown item submissions must stay inside the indexed semantic scope during presenter handoff");
	}

	@Test
	void itemDisplaySubmissionUsesTheIndexedSemanticScopeDuringPresenterHandoff() throws Exception {
		String renderer = Files.readString(Path.of(
			"src/main/java/net/minecraft/client/renderer/entity/DisplayRenderer.java"));
		int submit = renderer.indexOf("itemDisplayEntityRenderState.item.submit(");
		assertTrue(submit > 0, "item-display renderer must retain its item-state producer");
		int begin = renderer.lastIndexOf("beginItemEntitySubmission()", submit);
		int end = renderer.indexOf("endItemEntitySubmission()", submit);
		assertTrue(begin >= 0 && end > submit,
			"item-display submissions must stay inside the indexed semantic scope during presenter handoff");
	}

	@Test
	void itemFrameItemSubmissionUsesTheIndexedSemanticScopeDuringPresenterHandoff() throws Exception {
		String renderer = Files.readString(Path.of(
			"src/main/java/net/minecraft/client/renderer/entity/ItemFrameRenderer.java"));
		int submit = renderer.indexOf("itemFrameRenderState.item.submit(");
		assertTrue(submit > 0, "item-frame renderer must retain its item-state producer");
		int begin = renderer.lastIndexOf("beginItemEntitySubmission()", submit);
		int end = renderer.indexOf("endItemEntitySubmission()", submit);
		assertTrue(begin >= 0 && end > submit,
			"item-frame item submissions must stay inside the indexed semantic scope during presenter handoff");
	}

	@Test
	void livingEntityLayersUseTheIndexedSemanticScopeDuringPresenterHandoff() throws Exception {
		String renderer = Files.readString(Path.of(
			"src/main/java/net/minecraft/client/renderer/entity/LivingEntityRenderer.java"));
		int layers = renderer.indexOf("for (RenderLayer<S, M> renderLayer : this.layers)");
		assertTrue(layers > 0, "living-entity renderer must retain its feature-layer loop");
		int begin = renderer.lastIndexOf("beginItemEntitySubmission()", layers);
		int end = renderer.indexOf("endItemEntitySubmission()", layers);
		assertTrue(begin >= 0 && end > layers,
			"living-entity feature layers must stay inside the indexed semantic scope during presenter handoff");
		assertTrue(renderer.contains("VulkanicAPI.isVulkanBackendSelected()")
			&& renderer.contains("RustGalVulkanWholeFrameMode.enabled()"),
			"the layer scope must be limited to Rust presenter ownership");
	}

	@Test
	void guiBlurTransportRetainsTheSemanticSourceStratum() throws Exception {
		String state = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/render/state/GuiRenderState.java"));
		String renderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
		assertTrue(state.contains("blurBeforeStratumIndex()"));
		assertTrue(state.contains("return this.firstStratumAfterBlur == Integer.MAX_VALUE ? -1 : this.firstStratumAfterBlur;"));
		assertTrue(renderer.contains("blur boundary is semantic frame data"));
		assertFalse(renderer.contains("gui-blur-post-process"));
	}

	@Test
	void productionCrosshairSliceHasLifecycleInvalidationAndNoJavaFallback() throws Exception {
		String minecraft = Files.readString(Path.of("src/main/java/net/minecraft/client/Minecraft.java"));
		String gui = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/Gui.java"));
		String queue = Files.readString(Path.of("src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"));
		String frameCoordinator = Files.readString(Path.of("src/main/java/net/vulkanic/gui/RustGalFrameCoordinator.java"));
		String stratum = Files.readString(Path.of("src/main/java/net/vulkanic/gui/GuiRenderStratum.java"));
		String experienceBar = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/contextualbar/ExperienceBarRenderer.java"));
		String bossOverlay = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/components/BossHealthOverlay.java"));
		String context = Files.readString(Path.of("src/main/rust/render/vulkanic/backends/opengl/context.rs"));
		String openGlResources = Files.readString(Path.of("src/main/rust/render/vulkanic/backends/opengl/resources.rs"));
		String rustGuiFrontend = Files.readString(Path.of("src/main/rust/render/vulkanic/gui_frontend.rs"));

		assertTrue(minecraft.contains("ResourceManagerReloadListener") && minecraft.contains("RustGalFrameCoordinator.reload(resourceManager)"));
		assertTrue(minecraft.contains("RustGalFrameCoordinator.cancelPending(\"world-disconnect\")"));
		assertTrue(minecraft.contains("RustGalFrameCoordinator.cancelPending(\"world-unload\")"));
		assertTrue(frameCoordinator.contains("SCHEDULER.cancelAll(\"resource-reload\")"));
		assertTrue(frameCoordinator.contains("SCHEDULER.cancelAll(\"resize\")"));
		assertTrue(frameCoordinator.contains("SCHEDULER.cancelAll(\"shutdown\")"));
		assertTrue(queue.contains("mattmc.rustGal.gui.enabled"));
		assertTrue(queue.contains("mattmc.rustGal.guiCrosshair.enabled"));
		assertTrue(frameCoordinator.contains("rust_gal_ffi_resource_batch_calls"));
		assertTrue(queue.contains("collectResolvedAssets(ResourceManager resourceManager)"));
		assertTrue(frameCoordinator.contains("bridge.updateGuiAssets(assetGeneration, pendingAssets)"));
		assertTrue(frameCoordinator.contains("rust_gal_ffi_completion_query_calls"));
		assertTrue(frameCoordinator.contains("rust_gal_queue_depth"));
		assertTrue(frameCoordinator.contains("rust_gal_batches_executed"));
		assertTrue(queue.contains("mattmc.dev.guiCrosshair.disabled"));
		assertTrue(queue.contains("mattmc.dev.guiCrosshair.legacyControl"));
		assertTrue(queue.contains("mattmc.dev.rustGalGui.disabled"));
		assertTrue(queue.contains("mattmc.dev.rustGalGui.legacyControl"));
		assertTrue(queue.contains("mattmc.dev.rustGalGui.armor.disabled"));
		assertTrue(queue.contains("mattmc.dev.rustGalGui.armor.legacyControl"));
		assertTrue(queue.contains("mattmc.dev.rustGalGui.playerHealth.disabled"));
		assertTrue(queue.contains("mattmc.dev.rustGalGui.playerHealth.legacyControl"));
		assertTrue(queue.contains("mattmc.dev.rustGalGui.absorption.disabled"));
		assertTrue(queue.contains("mattmc.dev.rustGalGui.absorption.legacyControl"));
		assertTrue(queue.contains("mattmc.dev.rustGalGui.hunger.disabled"));
		assertTrue(queue.contains("mattmc.dev.rustGalGui.hunger.legacyControl"));
		assertTrue(queue.contains("HOTBAR_SELECTION_PRODUCER"));
		assertTrue(queue.contains("EXPERIENCE_BACKGROUND_PRODUCER"));
		assertTrue(queue.contains("EXPERIENCE_PROGRESS_PRODUCER"));
		assertTrue(queue.contains("ATTACK_CROSSHAIR_BACKGROUND_PRODUCER"));
		assertTrue(queue.contains("ATTACK_CROSSHAIR_PROGRESS_PRODUCER"));
		assertTrue(queue.contains("ATTACK_HOTBAR_BACKGROUND_PRODUCER"));
		assertTrue(queue.contains("ATTACK_HOTBAR_PROGRESS_PRODUCER"));
		assertTrue(queue.contains("BOSS_BAR_BACKGROUND_PRODUCER"));
		assertTrue(queue.contains("BOSS_BAR_PROGRESS_PRODUCER"));
		assertTrue(queue.contains("ARMOR_ICON_PRODUCER"));
		assertTrue(queue.contains("PLAYER_HEART_PRODUCER"));
		assertTrue(queue.contains("HUNGER_ICON_PRODUCER"));
		assertTrue(queue.contains("ABSORPTION_HEART_PRODUCER"));
		assertTrue(queue.contains("selected hotbar slot must be in 0..8"));
		assertTrue(queue.contains("armor value must be in 0..20"));
		assertTrue(queue.contains("PlayerHeartRequest"));
		assertTrue(queue.contains("AbsorptionHeartRequest"));
		assertFalse(queue.contains("getHealth()"));
		assertFalse(queue.contains("getMaxHealth()"));
		assertFalse(queue.contains("getAbsorptionAmount()"));
		assertTrue(queue.contains("experience progress fraction must be finite"));
		assertTrue(queue.contains("experience bar filled width is outside the vanilla range"));
		assertTrue(queue.contains("crosshair attack indicator filled width must be in 0..16"));
		assertTrue(queue.contains("hotbar attack indicator filled height must be in 0..18"));
		assertTrue(queue.contains("boss bar progress fraction must be finite"));
		assertTrue(queue.contains("boss bar filled width must be in 0.."));
			assertTrue(stratum.indexOf("GUI_HOTBAR_BASE(\"gui.hotbar.base\", 300)") < stratum.indexOf("GUI_HOTBAR_SELECTION(\"gui.hotbar.selection\", 310)"));
			assertTrue(stratum.indexOf("GUI_HOTBAR_SELECTION(\"gui.hotbar.selection\", 310)") < stratum.indexOf("GUI_ARMOR(\"gui.armor\", 350)"));
			assertTrue(stratum.indexOf("GUI_ARMOR(\"gui.armor\", 350)") < stratum.indexOf("GUI_PLAYER_HEALTH(\"gui.player-health\", 360)"));
			assertTrue(stratum.indexOf("GUI_PLAYER_HEALTH(\"gui.player-health\", 360)") < stratum.indexOf("GUI_EXPERIENCE_BAR_BACKGROUND(\"gui.experience.background\", 400)"));
			assertTrue(stratum.indexOf("GUI_EXPERIENCE_BAR_BACKGROUND(\"gui.experience.background\", 400)") < stratum.indexOf("GUI_EXPERIENCE_BAR_PROGRESS(\"gui.experience.progress\", 410)"));
		assertTrue(stratum.indexOf("GUI_EXPERIENCE_BAR_PROGRESS(\"gui.experience.progress\", 410)") < stratum.indexOf("GUI_ATTACK_CROSSHAIR_BACKGROUND(\"gui.attack.crosshair.background\", 500)"));
		assertTrue(stratum.indexOf("GUI_ATTACK_CROSSHAIR_BACKGROUND(\"gui.attack.crosshair.background\", 500)") < stratum.indexOf("GUI_ATTACK_CROSSHAIR_PROGRESS(\"gui.attack.crosshair.progress\", 510)"));
		assertTrue(stratum.indexOf("GUI_ATTACK_CROSSHAIR_PROGRESS(\"gui.attack.crosshair.progress\", 510)") < stratum.indexOf("GUI_ATTACK_HOTBAR_BACKGROUND(\"gui.attack.hotbar.background\", 520)"));
		assertTrue(stratum.indexOf("GUI_ATTACK_HOTBAR_BACKGROUND(\"gui.attack.hotbar.background\", 520)") < stratum.indexOf("GUI_ATTACK_HOTBAR_PROGRESS(\"gui.attack.hotbar.progress\", 530)"));
		assertTrue(stratum.indexOf("GUI_ATTACK_HOTBAR_PROGRESS(\"gui.attack.hotbar.progress\", 530)") < stratum.indexOf("GUI_BOSS_BAR_BACKGROUND(\"gui.boss.background\", 600)"));
		assertTrue(stratum.indexOf("GUI_BOSS_BAR_BACKGROUND(\"gui.boss.background\", 600)") < stratum.indexOf("GUI_BOSS_BAR_PROGRESS(\"gui.boss.progress\", 610)"));
			assertTrue(queue.contains("GuiSprite.HOTBAR_SELECTION"));
			assertTrue(queue.contains("GuiSprite.ARMOR_FULL"));
			assertTrue(queue.contains("GuiSprite.ARMOR_HALF"));
			assertTrue(queue.contains("GuiSprite.ARMOR_EMPTY"));
			assertTrue(queue.contains("GuiSprite.HEART_NORMAL_FULL"));
			assertTrue(queue.contains("GuiSprite.HEART_NORMAL_HALF"));
			assertTrue(queue.contains("GuiSprite.HEART_POISONED_FULL"));
			assertTrue(queue.contains("GuiSprite.HEART_WITHERED_FULL"));
			assertTrue(queue.contains("GuiSprite.HEART_FROZEN_FULL"));
			assertTrue(queue.contains("GuiSprite.HEART_ABSORBING_FULL"));
			assertTrue(queue.contains("GuiSprite.EXPERIENCE_BAR_BACKGROUND"));
		assertTrue(queue.contains("GuiSprite.EXPERIENCE_BAR_PROGRESS"));
		assertTrue(queue.contains("GuiSprite.CROSSHAIR_ATTACK_BACKGROUND"));
		assertTrue(queue.contains("GuiSprite.CROSSHAIR_ATTACK_PROGRESS"));
		assertTrue(queue.contains("GuiSprite.HOTBAR_ATTACK_BACKGROUND"));
		assertTrue(queue.contains("GuiSprite.HOTBAR_ATTACK_PROGRESS"));
		assertTrue(queue.contains("BOSS_BAR_PINK_BACKGROUND"));
		assertTrue(queue.contains("BOSS_BAR_WHITE_PROGRESS"));
		assertTrue(queue.contains("BOSS_BAR_NOTCHED_20_BACKGROUND"));
		assertTrue(queue.contains("BOSS_BAR_NOTCHED_20_PROGRESS"));
		assertTrue(queue.contains("GuiFillDirection.HORIZONTAL_LEFT_TO_RIGHT"));
		assertTrue(queue.contains("GuiFillDirection.VERTICAL_BOTTOM_TO_TOP"));
		assertTrue(queue.contains("GuiFillDirection.HORIZONTAL_LEFT_TO_RIGHT"));
		assertTrue(rustGuiFrontend.contains("uv_region"));
		assertTrue(queue.contains("selectedSlot"));
		assertTrue(queue.contains("progressFraction"));
		assertTrue(gui.indexOf("RustGalGuiRenderer.shouldDrawJavaCompatibilityGui()") < gui.indexOf("blitSprite(RenderPipelines.CROSSHAIR, CROSSHAIR_SPRITE"));
		assertTrue(gui.indexOf("blitSprite(RenderPipelines.CROSSHAIR, CROSSHAIR_SPRITE") < gui.indexOf("RustGalGuiRenderer.enqueueCrosshair"));
		assertTrue(gui.indexOf("RustGalGuiRenderer.shouldDrawJavaCompatibilityGui()", gui.indexOf("renderItemHotbar")) < gui.indexOf("blitSprite(RenderPipelines.GUI_TEXTURED, HOTBAR_SPRITE"));
			int hotbarMethod = gui.indexOf("renderItemHotbar");
			assertTrue(gui.indexOf("RustGalGuiRenderer.enqueueHotbarBase", hotbarMethod) < gui.indexOf("HOTBAR_SELECTION_SPRITE", hotbarMethod));
		assertTrue(gui.indexOf("HOTBAR_SELECTION_SPRITE", hotbarMethod) < gui.indexOf("RustGalGuiRenderer.enqueueHotbarSelection", hotbarMethod));
		assertTrue(gui.indexOf("RustGalGuiRenderer.enqueueHotbarSelection", hotbarMethod) < gui.indexOf("HOTBAR_OFFHAND_LEFT_SPRITE", hotbarMethod));
			assertTrue(gui.contains("selectedHotbarHighlightX"));
			assertTrue(gui.contains("selectedHotbarHighlightY"));
			int armorMethod = gui.indexOf("renderArmor");
			assertTrue(gui.indexOf("RustGalGuiRenderer.shouldDrawJavaCompatibilityGui()", armorMethod)
				< gui.indexOf("guiGraphics.blitSprite(RenderPipelines.GUI_TEXTURED, ARMOR_FULL_SPRITE", armorMethod));
			assertTrue(gui.indexOf("guiGraphics.blitSprite(RenderPipelines.GUI_TEXTURED, ARMOR_EMPTY_SPRITE", armorMethod)
				< gui.indexOf("RustGalGuiRenderer.enqueueArmorIcons", armorMethod));
			int healthMethod = gui.indexOf("private void renderHearts");
			assertTrue(gui.indexOf("rustAbsorptionHearts.add(new AbsorptionHeartRequest", healthMethod)
				< gui.indexOf("RustGalGuiRenderer.enqueueAbsorptionHearts", healthMethod));
			assertTrue(gui.indexOf("rustPlayerHearts.add(new PlayerHeartRequest", healthMethod)
				< gui.indexOf("RustGalGuiRenderer.enqueuePlayerHearts", healthMethod));
			assertTrue(gui.indexOf("RustGalGuiRenderer.enqueueAbsorptionHearts", healthMethod)
				< gui.indexOf("RustGalGuiRenderer.enqueuePlayerHearts", healthMethod));
			assertTrue(gui.indexOf("rustHeartVariant(heartType)", healthMethod)
				< gui.indexOf("RustGalGuiRenderer.enqueuePlayerHearts", healthMethod));
			assertTrue(gui.indexOf("rustAbsorptionHeartVariant(heartType)", healthMethod)
				< gui.indexOf("RustGalGuiRenderer.enqueueAbsorptionHearts", healthMethod));
			int foodMethod = gui.indexOf("private void renderFood");
			assertTrue(gui.indexOf("RustGalGuiRenderer.isHungerLegacyControl()", foodMethod)
				< gui.indexOf("guiGraphics.blitSprite(RenderPipelines.GUI_TEXTURED, resourceLocation", foodMethod));
			assertTrue(gui.indexOf("rustHungerIcons.add(new HungerIconRequest", foodMethod)
				< gui.indexOf("RustGalGuiRenderer.enqueueHungerIcons", foodMethod));
			assertTrue(gui.indexOf("diagnosticFoodLevel(foodData.getFoodLevel())", foodMethod)
				< gui.indexOf("RustGalGuiRenderer.enqueueHungerIcons", foodMethod));
			int airMethod = gui.indexOf("private void renderAirBubbles");
			assertTrue(gui.indexOf("RustGalGuiRenderer.isAirLegacyControl()", airMethod)
				< gui.indexOf("guiGraphics.blitSprite(RenderPipelines.GUI_TEXTURED, AIR_SPRITE", airMethod));
			assertTrue(gui.indexOf("rustAirBubbles.add(new AirBubbleRequest", airMethod)
				< gui.indexOf("RustGalGuiRenderer.enqueueAirBubbles", airMethod));
			assertTrue(gui.indexOf("diagnosticAirSupply(player.getAirSupply(), l)", airMethod)
				< gui.indexOf("RustGalGuiRenderer.enqueueAirBubbles", airMethod));
			int experienceMethod = experienceBar.indexOf("renderBackground");
		assertTrue(experienceBar.indexOf("RustGalGuiRenderer.shouldDrawJavaCompatibilityGui()", experienceMethod)
			< experienceBar.indexOf("guiGraphics.blitSprite(RenderPipelines.GUI_TEXTURED, EXPERIENCE_BAR_BACKGROUND_SPRITE", experienceMethod));
		assertTrue(experienceBar.indexOf("guiGraphics.blitSprite(RenderPipelines.GUI_TEXTURED, EXPERIENCE_BAR_BACKGROUND_SPRITE", experienceMethod)
			< experienceBar.indexOf("RustGalGuiRenderer.enqueueExperienceBar", experienceMethod));
		assertTrue(experienceBar.indexOf("guiGraphics.blitSprite(RenderPipelines.GUI_TEXTURED, EXPERIENCE_BAR_PROGRESS_SPRITE", experienceMethod)
			< experienceBar.indexOf("RustGalGuiRenderer.enqueueExperienceBar", experienceMethod));
		int crosshairAttack = gui.indexOf("CROSSHAIR_ATTACK_INDICATOR_BACKGROUND_SPRITE");
		assertTrue(gui.indexOf("RustGalGuiRenderer.shouldDrawJavaCompatibilityGui()", gui.indexOf("renderCrosshair"))
			< gui.indexOf("guiGraphics.blitSprite(RenderPipelines.CROSSHAIR, CROSSHAIR_ATTACK_INDICATOR_BACKGROUND_SPRITE", crosshairAttack));
		int crosshairProgressBlit = gui.indexOf("guiGraphics.blitSprite(RenderPipelines.CROSSHAIR, CROSSHAIR_ATTACK_INDICATOR_PROGRESS_SPRITE", crosshairAttack);
		assertTrue(crosshairProgressBlit < gui.indexOf("RustGalGuiRenderer.enqueueCrosshairAttackIndicator", crosshairProgressBlit));
		int hotbarAttack = gui.indexOf("HOTBAR_ATTACK_INDICATOR_BACKGROUND_SPRITE");
		assertTrue(gui.indexOf("RustGalGuiRenderer.shouldDrawJavaCompatibilityGui()", gui.indexOf("AttackIndicatorStatus.HOTBAR"))
			< gui.indexOf("guiGraphics.blitSprite(RenderPipelines.GUI_TEXTURED, HOTBAR_ATTACK_INDICATOR_BACKGROUND_SPRITE", hotbarAttack));
		assertTrue(gui.indexOf("guiGraphics.blitSprite(RenderPipelines.GUI_TEXTURED, HOTBAR_ATTACK_INDICATOR_PROGRESS_SPRITE", hotbarAttack)
			< gui.indexOf("RustGalGuiRenderer.enqueueHotbarAttackIndicator", hotbarAttack));
		assertTrue(bossOverlay.contains("DeterministicCameraCapture.applyBossBarOverridesForDiagnostics"));
		assertTrue(bossOverlay.contains("RustGalGuiRenderer.enqueueBossBar"));
		assertTrue(bossOverlay.indexOf("RustGalGuiRenderer.shouldDrawJavaCompatibilityGui()") < bossOverlay.indexOf("guiGraphics.blitSprite(RenderPipelines.GUI_TEXTURED, resourceLocations"));
		assertTrue(bossOverlay.indexOf("RustGalGuiRenderer.shouldDrawJavaCompatibilityGui()") < bossOverlay.indexOf("RustGalGuiRenderer.enqueueBossBar"));
		assertTrue(bossOverlay.indexOf("RustGalGuiRenderer.enqueueBossBar")
			< bossOverlay.indexOf("private void drawBar(\n\t\tGuiGraphics guiGraphics"));
		assertTrue(bossOverlay.contains("drawString(this.minecraft.font"));
		assertTrue(context.contains("MAX_COMBINED_TEXTURE_IMAGE_UNITS"));
		assertTrue(openGlResources.contains("current_frame_target_framebuffer"),
			"persistent borrowed frame-target handles must refresh the native OpenGL framebuffer after screen transitions");
		assertTrue(openGlResources.contains("borrowed_frame_targets_follow_latest_acquired_framebuffer"));
	}

	@Test
	void wholeFrameHudVignetteDoesNotQueryIrisPipelineState() throws Exception {
		String gui = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/Gui.java"));
		int method = gui.indexOf("private void renderVignette");
		int iris = gui.indexOf("Iris.getPipelineManager().getPipelineNullable()", method);
		assertTrue(method >= 0 && iris > method);
		String prefix = gui.substring(method, iris);
		assertTrue(prefix.contains("RustGalVulkanWholeFrameMode.enabled()"));
		assertTrue(prefix.contains("? null"));
	}

	@Test
	void selectedVulkanHudDoesNotEnterIrisVisibilityOrDebugState() throws Exception {
		String gui = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/Gui.java"));
		assertTrue(gui.contains("!net.vulkanic.VulkanicAPI.isVulkanBackendSelected()")
			&& gui.contains("legacyIrisDebugGroup"),
			"selected Vulkan HUD extraction must not enter Iris visibility or debug-group state");
		assertTrue(gui.contains("net.vulkanic.VulkanicAPI.isVulkanBackendSelected()\n\t\t\t\t? null"),
			"selected Vulkan vignette selection must not query the Iris pipeline");
	}

	@Test
	void legacySkyDrawMethodsFailClosedDuringRustWholeFrame() throws Exception {
		String sky = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/SkyRenderer.java"));
		assertEquals(12, occurrences(sky, "ensureJavaSkyRenderingAvailable();"));
		assertTrue(sky.contains("Java sky rendering is unavailable while Rust owns whole-frame presentation"));
		assertTrue(sky.contains("net.vulkanic.VulkanicAPI.isVulkanBackendSelected()"));
	}

	@Test
	void legacyDebugDispatcherFailsClosedDuringRustWholeFrame() throws Exception {
		String debug = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/debug/DebugRenderer.java"));
		int method = debug.indexOf("public void render(");
		int minecraft = debug.indexOf("Minecraft minecraft", method);
		assertTrue(method >= 0 && minecraft > method);
		String body = debug.substring(method, minecraft);
		assertTrue(body.contains("RustGalVulkanWholeFrameMode.enabled()"));
		assertTrue(body.contains("Java debug rendering is unavailable"));
	}

	@Test
	void gameTestBlockHighlightsCannotReopenJavaBuffersDuringRustWholeFrame() throws Exception {
		String highlights = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/debug/GameTestBlockHighlightRenderer.java"));
		int method = highlights.indexOf("public void render(");
		int clock = highlights.indexOf("long l = Util.getMillis();", method);
		assertTrue(method >= 0 && clock > method);
		String body = highlights.substring(method, clock);
		assertTrue(body.contains("RustGalVulkanWholeFrameMode.enabled()"));
		assertTrue(body.contains("Java game-test block highlights are unavailable"));
	}

	@Test
	void wholeFrameResourceTextureUploadsStayOutOfJavaEncoders() throws Exception {
		String reloadable = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/texture/ReloadableTexture.java"));
		String sprite = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/texture/SpriteContents.java"));
		String cubeMap = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/texture/CubeMapTexture.java"));
		assertTrue(reloadable.contains("!net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()"));
		assertTrue(sprite.contains("net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()")
			&& sprite.contains("Rust consumes the copied atlas/resource-pack pixels"));
		assertTrue(cubeMap.contains("!net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()"));
	}

	@Test
	void wholeFrameFontAtlasBakingKeepsGlyphPixelsCpuOwned() throws Exception {
		String truetype = Files.readString(Path.of("src/main/java/net/blaze3d/font/TrueTypeGlyphProvider.java"));
		String bitmap = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/font/providers/BitmapProvider.java"));
		String unihex = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/font/providers/UnihexProvider.java"));
		String special = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/font/glyphs/SpecialGlyphs.java"));
		assertTrue(truetype.contains("&& !net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()")
			&& truetype.contains("&& !net.vulkanic.VulkanicAPI.isVulkanBackendSelected()"));
		assertTrue(bitmap.contains("!net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()")
			&& bitmap.contains("!net.vulkanic.VulkanicAPI.isVulkanBackendSelected()"));
		assertTrue(unihex.contains("if (!net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()")
			&& unihex.contains("&& !net.vulkanic.VulkanicAPI.isVulkanBackendSelected()"));
		assertTrue(special.contains("!net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()")
			&& special.contains("!net.vulkanic.VulkanicAPI.isVulkanBackendSelected()"));
		assertTrue(truetype.contains("copyTo(NativeImage"));
		assertTrue(bitmap.contains("copyTo(NativeImage"));
		assertTrue(unihex.contains("copyTo(NativeImage"));
		assertTrue(special.contains("copyTo(NativeImage"));
	}

	@Test
	void wholeFrameChunkUploadsDoNotCreateJavaSectionBuffers() throws Exception {
		String dispatcher = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/chunk/SectionRenderDispatcher.java"));
		int meshUpload = dispatcher.indexOf("public CompletableFuture<Void> upload(");
		int indexUpload = dispatcher.indexOf("public CompletableFuture<Void> uploadSectionIndexBuffer(");
		assertTrue(meshUpload >= 0 && indexUpload > meshUpload);
		String meshBody = dispatcher.substring(meshUpload, indexUpload);
		String indexBody = dispatcher.substring(indexUpload);
		assertTrue(meshBody.contains("RustGalVulkanWholeFrameMode.enabled()"));
		assertTrue(meshBody.contains("map.values().forEach(MeshData::close)"));
		assertTrue(indexBody.contains("RustGalVulkanWholeFrameMode.enabled()"));
		assertTrue(indexBody.contains("result.close()"));
	}

	@Test
	void wholeFramePanoramaDoesNotAllocateJavaProjectionUbo() throws Exception {
		String cubeMap = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/CubeMap.java"));
		int constructor = cubeMap.indexOf("public CubeMap(ResourceLocation resourceLocation)");
		int textureRegistration = cubeMap.indexOf("public void registerTextures", constructor);
		assertTrue(constructor >= 0 && textureRegistration > constructor);
		String body = cubeMap.substring(constructor, textureRegistration);
		assertTrue(body.contains("RustGalVulkanWholeFrameMode.enabled()"));
		assertTrue(body.contains("? null"));
		assertTrue(cubeMap.contains("if (this.projectionMatrixUbo != null)"));
	}

	@Test
	void wholeFrameGuiRendererDoesNotAllocateJavaProjectionUbos() throws Exception {
		String renderer = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/render/GuiRenderer.java"));
		int constructor = renderer.indexOf("public GuiRenderer(");
		int builder = renderer.indexOf("Builder<Class<? extends PictureInPictureRenderState>", constructor);
		assertTrue(constructor >= 0 && builder > constructor);
		String body = renderer.substring(constructor, builder);
		assertTrue(body.contains("RustGalGuiRenderer.isWholeFrameVulkanEnabled()"));
		assertTrue(body.contains("this.guiProjectionMatrixBuffer = null"));
		assertTrue(body.contains("this.itemsProjectionMatrixBuffer = null"));
		assertTrue(renderer.contains("if (this.guiProjectionMatrixBuffer != null)"));
		assertTrue(renderer.contains("if (this.itemsProjectionMatrixBuffer != null)"));
	}

	@Test
	void selectedVulkanGuiCannotAllocateProjectionOrPictureInPictureResources() throws Exception {
		String gui = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/render/GuiRenderer.java"));
		String pip = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/render/pip/PictureInPictureRenderer.java"));
		assertTrue(gui.contains("VulkanicAPI.isVulkanBackendSelected()")
			&& pip.contains("VulkanicAPI.isVulkanBackendSelected()"),
			"selected Vulkan GUI construction must not allocate Java projection resources");
		assertTrue(pip.contains("Java GUI picture-in-picture rendering is unavailable on selected Vulkan"),
			"selected Vulkan PIP rendering must fail closed before offscreen Java textures");
	}

	@Test
	void wholeFramePipRenderersDoNotAllocateJavaProjectionUbos() throws Exception {
		String pip = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/render/pip/PictureInPictureRenderer.java"));
		int constructor = pip.indexOf("protected PictureInPictureRenderer(");
		int prepare = pip.indexOf("public void prepare(", constructor);
		assertTrue(constructor >= 0 && prepare > constructor);
		String body = pip.substring(constructor, prepare);
		assertTrue(body.contains("RustGalGuiRenderer.isWholeFrameVulkanEnabled()"));
		assertTrue(body.contains("? null"));
		assertTrue(pip.contains("if (this.projectionMatrixBuffer != null)"));
		assertTrue(pip.contains("Java GUI picture-in-picture rendering is unavailable while Rust owns whole-frame presentation")
			|| pip.contains("Java GUI picture-in-picture rendering is unavailable on selected Vulkan"),
			"Java PIP preparation must fail closed even if called outside the normal GUI dispatcher");
	}

	@Test
	void wholeFrameGameRendererDoesNotAllocateJavaHudProjectionUbo() throws Exception {
		String renderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
		int constructor = renderer.indexOf("public GameRenderer(");
		int lightTexture = renderer.indexOf("this.lightTexture =", constructor);
		assertTrue(constructor >= 0 && lightTexture > constructor);
		String body = renderer.substring(constructor, lightTexture);
		assertTrue(body.contains("RustGalVulkanWholeFrameMode.enabled()"));
		assertTrue(body.contains("? null"));
		assertTrue(renderer.contains("if (this.hud3dProjectionMatrixBuffer != null)"));
	}

	@Test
	void selectedVulkanGameRendererCannotAllocateJavaProjectionOrHardwareState() throws Exception {
		String renderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
		int constructor = renderer.indexOf("public GameRenderer(");
		int levelProjection = renderer.indexOf("new PerspectiveProjectionMatrixBuffer(\"level\")", constructor);
		assertTrue(constructor >= 0 && levelProjection > constructor
			&& renderer.substring(constructor, levelProjection).contains("isVulkanBackendSelected()"),
			"selected Vulkan GameRenderer construction must leave Java projection buffers absent");
		assertTrue(renderer.contains("!net.vulkanic.VulkanicAPI.isVulkanBackendSelected()")
			&& renderer.contains("Java GameRenderer.renderLevel is unavailable"),
			"selected Vulkan GameRenderer must avoid Java hardware state and level rendering");
	}

	@Test
	void wholeFrameLightingDoesNotAllocateOrSubmitJavaUbo() throws Exception {
		String lighting = Files.readString(Path.of("src/main/java/net/blaze3d/platform/Lighting.java"));
		int constructor = lighting.indexOf("public Lighting()");
		int updateBuffer = lighting.indexOf("private void updateBuffer", constructor);
		assertTrue(constructor >= 0 && updateBuffer > constructor);
		String body = lighting.substring(constructor, updateBuffer);
		assertTrue(body.contains("RustGalVulkanWholeFrameMode.enabled()"));
		assertTrue(body.contains("this.buffer = null"));
		assertTrue(lighting.contains("if (this.buffer == null)"));
		assertTrue(lighting.contains("if (this.buffer != null)"));
	}

	@Test
	void wholeFrameGameRendererDoesNotAllocateLegacyWorldHandOrSettingsUbos() throws Exception {
		String renderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
		int constructor = renderer.indexOf("public GameRenderer(");
		int lightTexture = renderer.indexOf("this.lightTexture =", constructor);
		assertTrue(constructor >= 0 && lightTexture > constructor);
		String body = renderer.substring(constructor, lightTexture);
		assertTrue(body.contains("RustGalVulkanWholeFrameMode.enabled()"));
		assertTrue(body.contains("this.globalSettingsUniform = null"));
		assertTrue(body.contains("this.levelProjectionMatrixBuffer = null"));
		assertTrue(body.contains("this.handProjectionMatrixBuffer = null"));
		assertTrue(renderer.contains("if (this.globalSettingsUniform != null)"));
		assertTrue(renderer.contains("if (this.levelProjectionMatrixBuffer != null)"));
		assertTrue(renderer.contains("if (this.handProjectionMatrixBuffer != null)"));
	}

	@Test
	void wholeFrameGameRendererDoesNotTouchIrisHardwareDiagnostics() throws Exception {
		String renderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
		int diagnostics = renderer.indexOf("net.irisshaders.iris.Iris.logger.info(\"Hardware information:\")");
		int screenEffects = renderer.indexOf("this.screenEffectRenderer =", diagnostics);
		assertTrue(diagnostics >= 0 && screenEffects > diagnostics);
		assertTrue(renderer.contains("!net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()")
			&& renderer.contains("!net.vulkanic.VulkanicAPI.isVulkanBackendSelected()"));
	}

	@Test
	void wholeFrameGameRendererDoesNotFinalizeThroughIrisPipeline() throws Exception {
		String renderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
		int finalize = renderer.indexOf("finalizeGameRendering");
		assertTrue(finalize >= 0);
		String prefix = renderer.substring(Math.max(0, finalize - 300), finalize);
		assertTrue(prefix.contains("!net.vulkanic.VulkanicAPI.isVulkanBackendSelected()")
			&& prefix.contains("!net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()"),
			"Iris finalization must be fenced to Java OpenGL ownership");
	}

	@Test
	void firstPersonGameRendererIrisQueryIsJavaRouteOnly() throws Exception {
		String renderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
		int helper = renderer.indexOf("private static boolean javaIrisShaderPackActive()");
		assertTrue(helper >= 0);
		int query = renderer.indexOf("net.irisshaders.iris.Iris.isPackInUseQuick()", helper);
		assertTrue(query > helper);
		String helperBody = renderer.substring(helper, renderer.indexOf("\n\t}", query) + 3);
		assertTrue(helperBody.contains("!net.vulkanic.VulkanicAPI.isVulkanBackendSelected()"));
		assertTrue(helperBody.contains("!net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()"));
		int handCall = renderer.indexOf("if (!net.vulkanic.VulkanicAPI.isVulkanBackendSelected()", renderer.indexOf("private void renderItemInHand"));
		assertTrue(handCall >= 0, "selected Vulkan must not reopen Java first-person hand rendering");
		String hand = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/ItemInHandRenderer.java"));
		int route = hand.indexOf("WorldRenderRoutePolicy.Route firstPersonRoute");
		assertTrue(route >= 0 && hand.indexOf("RustGalVulkanWholeFrameMode.enabled()", route) > route,
			"first-person item ownership must use whole-frame handoff enablement before presenter selection");
	}

	@Test
	void levelCoverageCollectorHonorsRustPresenterShellBeforeFinalizedSelection() throws Exception {
		String level = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/LevelRenderer.java"));
		int extraction = level.indexOf("boolean rustWholeFrameEntityExtraction");
		assertTrue(extraction >= 0);
		String extractionBody = level.substring(extraction, level.indexOf("boolean entitySectionReady", extraction));
		assertTrue(extractionBody.contains("VulkanicAPI.isVulkanBackendSelected()")
			&& extractionBody.contains("RustGalVulkanWholeFrameMode.enabled()")
			&& !extractionBody.contains("&& net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()"),
			"entity extraction must remain Rust-owned during the presenter shell");
		int blockModel = level.lastIndexOf("public void submitBlockModel(PoseStack poseStack");
		assertTrue(blockModel >= 0);
		String blockModelBody = level.substring(blockModel, level.indexOf("public void submitItem(", blockModel));
		assertTrue(blockModelBody.contains("RustGalVulkanWholeFrameMode.enabled()"),
			"block-model coverage must honor the Rust presenter shell");
		int item = level.indexOf("boolean rustWholeFrame =", blockModel);
		assertTrue(item >= 0);
		String itemBody = level.substring(item, level.indexOf("boolean firstPersonSemantic", item));
		assertTrue(itemBody.contains("RustGalVulkanWholeFrameMode.enabled()")
			&& !itemBody.contains("&&\n\t\t\t\tnet.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()"),
			"item coverage must honor the Rust presenter shell");
	}

	@Test
	void mapRendererCannotReopenJavaGeometryDuringPresenterHandoff() throws Exception {
		String map = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/MapRenderer.java"));
		assertTrue(map.contains("!mapAccepted && (net.vulkanic.VulkanicAPI.isVulkanBackendSelected()")
			&& map.contains("!decorationAccepted && (net.vulkanic.VulkanicAPI.isVulkanBackendSelected()")
			&& map.contains("net.vulkanic.VulkanicAPI.isVulkanBackendSelected()\n\t\t\t\t\t\t\t|| net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()"),
			"map quads, decorations, and labels must fail closed under the Rust presenter shell");
	}

	@Test
	void guiItemAtlasCannotEnterJavaRenderingDuringPresenterHandoff() throws Exception {
		String gui = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/render/GuiRenderer.java"));
		int method = gui.indexOf("private void renderItemToAtlas(");
		int body = gui.indexOf("poseStack.pushPose();", method);
		assertTrue(method >= 0 && body > method);
		String prefix = gui.substring(method, body);
		assertTrue(prefix.contains("RustGalVulkanWholeFrameMode.enabled()")
			&& prefix.contains("Java GUI item-atlas rendering is unavailable"),
			"GUI item-atlas rendering must fail closed under Rust presenter ownership");
		assertTrue(gui.substring(method).contains("boolean bl2 = !VulkanicAPI.isVulkanBackendSelected();"),
			"item-atlas scissor state must explicitly bypass selected Vulkan routing");
	}

	@Test
	void wholeFrameSkyDoesNotAllocateLegacyVertexBuffers() throws Exception {
		String sky = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/SkyRenderer.java"));
		int constructor = sky.indexOf("public SkyRenderer()");
		int buildStars = sky.indexOf("this.starBuffer = this.buildStars()", constructor);
		assertTrue(constructor >= 0 && buildStars > constructor);
		String body = sky.substring(constructor, buildStars);
		assertTrue(body.contains("RustGalVulkanWholeFrameMode.enabled()"));
		assertTrue(body.contains("this.starBuffer = null"));
		assertTrue(body.contains("this.endFlashBuffer = null"));
		assertTrue(sky.contains("if (this.sunBuffer != null)"));
		String world = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		assertTrue(world.contains("Rust VulkanicGAL owned sky texture assets registered")
			&& world.contains("+ \" route=admitted\""),
			"successful Rust sky asset admission must be reported as admitted for parity diagnostics");
	}

	@Test
	void wholeFrameFogDoesNotAllocateOrPublishJavaUbos() throws Exception {
		String fog = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/fog/FogRenderer.java"));
		int constructor = fog.indexOf("public FogRenderer()");
		int close = fog.indexOf("public void close()", constructor);
		assertTrue(constructor >= 0 && close > constructor);
		String body = fog.substring(constructor, close);
		assertTrue(body.contains("RustGalVulkanWholeFrameMode.enabled()"));
		assertTrue(body.contains("this.regularBuffer = null"));
		assertTrue(body.contains("this.emptyBuffer = null"));
		assertTrue(fog.contains("Java fog UBO rendering is unavailable"));
		assertTrue(fog.contains("public Vector4f setupFog(")
			&& fog.substring(fog.indexOf("public Vector4f setupFog("))
				.contains("RustGalVulkanWholeFrameMode.enabled()"),
			"the Java fog setup entrypoint must fail closed before touching the absent UBO");
		int color = fog.indexOf("public Vector4f computeFogColor(");
		int legacyFlag = fog.indexOf("!net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()", color);
		assertTrue(color >= 0 && legacyFlag > color,
			"public fog color queries must not publish Iris state during Rust handoff");
		int endFrame = fog.indexOf("public void endFrame()");
		assertTrue(endFrame >= 0 && fog.indexOf("RustGalVulkanWholeFrameMode.enabled()", endFrame) > endFrame,
			"Rust handoff must not rotate the Java fog ring buffer");
	}

	@Test
	void selectedVulkanFogStaysSemanticAndCannotEnterJavaUboSetup() throws Exception {
		String fog = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/fog/FogRenderer.java"));
		int constructor = fog.indexOf("public FogRenderer()");
		int setup = fog.indexOf("public Vector4f setupFog(");
		assertTrue(fog.contains("net.vulkanic.VulkanicAPI.isVulkanBackendSelected()")
			&& constructor >= 0 && setup > constructor,
			"selected Vulkan fog must be admitted as semantic state before Java UBO setup");
		assertTrue(fog.substring(setup).contains("net.vulkanic.VulkanicAPI.isVulkanBackendSelected()")
			&& fog.contains("computeFogColorSemantic"),
			"selected Vulkan fog must fail closed and retain an Iris-free semantic color path");
	}

	@Test
	void wholeFrameCloudsDoNotAllocateLegacyUbo() throws Exception {
		String clouds = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/CloudRenderer.java"));
		int constructor = clouds.indexOf("public CloudRenderer()");
		int prepare = clouds.indexOf("protected Optional<CloudRenderer.TextureData> prepare", constructor);
		assertTrue(constructor >= 0 && prepare > constructor);
		String body = clouds.substring(constructor, prepare);
		assertTrue(body.contains("RustGalVulkanWholeFrameMode.enabled()"));
		assertTrue(body.contains("? null"));
		assertTrue(clouds.contains("if (this.ubo != null)"));
	}

	@Test
	void wholeFrameWorldBorderDoesNotAllocateLegacyVertexBuffer() throws Exception {
		String border = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/WorldBorderRenderer.java"));
		int constructor = border.indexOf("public WorldBorderRenderer()");
		int render = border.indexOf("public void render(", constructor);
		assertTrue(constructor >= 0 && render > constructor);
		String body = border.substring(constructor, render);
		assertTrue(body.contains("RustGalVulkanWholeFrameMode.enabled()"));
		assertTrue(body.contains("? null"));
		assertTrue(body.contains("this.indices") && body.contains("getSequentialBuffer(VertexFormat.Mode.QUADS)"),
			"world-border sequential index storage must be constructed inside the same Rust ownership fence");
		assertTrue(border.contains("Java world-border rendering is unavailable"));
	}

	@Test
	void selectedVulkanAtmosphericRenderersCannotUseJavaBuffers() throws Exception {
		String clouds = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/CloudRenderer.java"));
		String sky = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/SkyRenderer.java"));
		String border = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/WorldBorderRenderer.java"));
		assertTrue(clouds.contains("net.vulkanic.VulkanicAPI.isVulkanBackendSelected()")
			&& clouds.contains("Java cloud rendering is unavailable"));
		assertTrue(sky.contains("net.vulkanic.VulkanicAPI.isVulkanBackendSelected()")
			&& sky.contains("Java sky rendering is unavailable"));
		assertTrue(border.contains("VulkanicAPI.isVulkanBackendSelected()")
			&& border.contains("Java Vulkan world-border rendering is unavailable"));
	}

	@Test
	void wholeFrameLightTextureKeepsOnlySemanticInputsAndNoJavaUbo() throws Exception {
		String light = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/LightTexture.java"));
		int constructor = light.indexOf("public LightTexture(");
		int textureView = light.indexOf("public GpuTextureView getTextureView", constructor);
		assertTrue(constructor >= 0 && textureView > constructor);
		String accessor = light.substring(textureView, light.indexOf("@Nullable", textureView));
		assertTrue(accessor.contains("VulkanicAPI.isVulkanBackendSelected()")
			&& accessor.contains("RustGalVulkanWholeFrameMode.enabled()")
			&& accessor.contains("use semantic lightmap inputs"),
			"Java lightmap texture access must fail closed for selected/whole-frame Rust Vulkan");
		String body = light.substring(constructor, textureView);
		assertTrue(body.contains("RustGalVulkanWholeFrameMode.enabled()"));
		assertTrue(body.contains("? null"));
		assertTrue(light.contains("if (this.ubo != null)"));
		assertTrue(light.contains("ensureRustSemanticLightmapInputs"));
		int darkness = light.indexOf("setDarknessLightFactor");
		int darknessGuard = light.lastIndexOf("RustGalVulkanWholeFrameMode.enabled()", darkness);
		assertTrue(darkness >= 0 && darknessGuard >= 0 && darknessGuard < darkness,
			"whole-frame lightmap calculation must not publish Iris darkness state");
	}

	@Test
	void wholeFramePostPassDoesNotAllocateJavaUniformBuffers() throws Exception {
		String post = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/PostPass.java"));
		int constructor = post.indexOf("public PostPass(");
		int uniforms = post.indexOf("for (Entry<String, List<UniformValue>> entry", constructor);
		assertTrue(constructor >= 0 && uniforms > constructor);
		String body = post.substring(constructor, uniforms);
		assertTrue(body.contains("RustGalVulkanWholeFrameMode.enabled()"));
		assertTrue(body.contains("this.infoUbo = null"));
		assertTrue(post.contains("Java post-pass rendering is unavailable"));
	}

	@Test
	void wholeFrameEndFrameDoesNotRotateJavaCloudOrSodiumBuffers() throws Exception {
		String level = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/LevelRenderer.java"));
		String clouds = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/CloudRenderer.java"));
		int levelMethod = level.indexOf("public void endFrame()");
		int cloudCall = level.indexOf("this.cloudRenderer.endFrame()", levelMethod);
		assertTrue(levelMethod >= 0 && cloudCall > levelMethod);
		assertTrue(level.substring(levelMethod, cloudCall).contains("RustGalVulkanWholeFrameMode.enabled()"));
		assertTrue(clouds.contains("RustGalVulkanWholeFrameMode.enabled()"));
	}

	@Test
	void wholeFrameRenderTargetCompatibilityHelpersFailClosed() throws Exception {
		String target = Files.readString(Path.of("src/main/java/net/blaze3d/pipeline/RenderTarget.java"));
		assertTrue(target.contains("rejectRustWholeFrameOperation(\"depth-copy\")"));
		assertTrue(target.contains("rejectRustWholeFrameOperation(\"render-target blend\")"));
		assertTrue(target.contains("Java RenderTarget " ));
	}

	@Test
	void rustScreenEffectExtractionRequiresActiveWholeFrameShell() throws Exception {
		String effects = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/ScreenEffectRenderer.java"));
		int activation = effects.indexOf("public void renderRustVulkanItemActivation");
		int activationGuard = effects.indexOf("Rust Vulkan item-activation extraction requires an active whole-frame shell", activation);
		int screen = effects.indexOf("public void renderRustVulkanScreenEffects");
		int screenGuard = effects.indexOf("Rust Vulkan screen-effect extraction requires an active whole-frame shell", screen);
		assertTrue(activation >= 0 && activationGuard > activation
			&& screen >= 0 && screenGuard > screen,
			"Rust screen-effect semantic entrypoints must fail closed before shell admission");
	}

	@Test
	void selectedVulkanLevelReloadCannotAllocateJavaEntityOutlineTarget() throws Exception {
		String levelRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/LevelRenderer.java"));
		int initOutline = levelRenderer.indexOf("public void initOutline()");
		int selectedGuard = levelRenderer.indexOf("net.vulkanic.VulkanicAPI.isVulkanBackendSelected()", initOutline);
		int allocation = levelRenderer.indexOf("new TextureTarget(\"Entity Outline\"", initOutline);
		assertTrue(initOutline >= 0 && selectedGuard > initOutline && selectedGuard < allocation,
			"selected Vulkan resource reload must not allocate the Java entity-outline target");
		int doOutline = levelRenderer.indexOf("public void doEntityOutline()");
		int doGuard = levelRenderer.indexOf("!net.vulkanic.VulkanicAPI.isVulkanBackendSelected()", doOutline);
		assertTrue(doOutline >= 0 && doGuard > doOutline,
			"selected Vulkan must not blit the Java entity-outline target");
	}

	@Test
	void devOnlyRustVulkanWholeFrameShellOwnsPresentationWithoutJavaVulkanExecution() throws Exception {
		String bridge = Files.readString(Path.of("src/main/java/net/vulkanic/bridge/VulkanicGalBridge.java"));
		String mode = Files.readString(Path.of("src/main/java/net/vulkanic/bridge/RustGalVulkanWholeFrameMode.java"));
		String queue = Files.readString(Path.of("src/main/java/net/vulkanic/gui/RustGalFrameCoordinator.java"));
		String minecraft = Files.readString(Path.of("src/main/java/net/minecraft/client/Minecraft.java"));
		String gameRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
		String guiRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/render/GuiRenderer.java"));
		String hud = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/Gui.java"));
		String guiState = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/render/state/GuiRenderState.java"));
		String blitState = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/render/state/BlitRenderState.java"));
		String guiGraphics = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/GuiGraphics.java"));
		String guiSemanticRenderer = Files.readString(Path.of("src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"));
		String guiStratum = Files.readString(Path.of("src/main/java/net/vulkanic/gui/GuiRenderStratum.java"));
		String levelRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/LevelRenderer.java"));
		String featureRenderDispatcher = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/feature/FeatureRenderDispatcher.java"));
		String modelFeatureRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/feature/ModelFeatureRenderer.java"));
		String itemFeatureRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/feature/ItemFeatureRenderer.java"));
		String textFeatureRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/feature/TextFeatureRenderer.java"));
		String worldPrimitiveRenderer = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		String rustWorldPrimitiveFrontend = Files.readString(Path.of("src/main/rust/render/vulkanic/world_primitive_frontend.rs"));
		String rustWorldTextFrontend = Files.readString(Path.of("src/main/rust/render/vulkanic/world_primitive_frontend/world_text.rs"));
		assertTrue(worldPrimitiveRenderer.contains("MAX_WORLD_MESH_TEXTURE_PNG_BYTES = 4 * 1024 * 1024")
			&& worldPrimitiveRenderer.contains("MAX_RUST_WORLD_MESH_INSTANCES = 4_096")
			&& worldPrimitiveRenderer.contains("new BoundedSemanticQueue<>(MAX_RUST_WORLD_MESH_INSTANCES, \"mesh-producer\")")
			&& worldPrimitiveRenderer.contains("MAX_WORLD_BORDER_ASSET_BYTES = 2 * 1024 * 1024")
			&& worldPrimitiveRenderer.contains("MAX_WORLD_AUXILIARY_ASSET_BYTES = 4 * 1024 * 1024")
			&& worldPrimitiveRenderer.contains("MAX_WORLD_MESH_ASSET_RESIDENCY = 16_384")
			&& worldPrimitiveRenderer.contains("MAX_WORLD_MESH_TEXTURE_RESIDENCY = 4_096")
			&& worldPrimitiveRenderer.contains("MAX_WORLD_MESH_TEXTURE_PNG_BYTES_TOTAL = 256L * 1024L * 1024L")
			&& rustWorldPrimitiveFrontend.contains("WORLD_MAX_MESH_TEXTURE_ASSETS: usize = 4_096")
			&& rustWorldPrimitiveFrontend.contains("WORLD_MAX_MESH_TEXTURE_DECODED_BYTES: usize = 256 * 1024 * 1024")
			&& worldPrimitiveRenderer.contains("MAX_PARTICLE_ATLAS_IDENTITIES = 1_024")
			&& worldPrimitiveRenderer.contains("MAX_DYNAMIC_WORLD_ASSET_FINGERPRINTS = 4_096")
			&& worldPrimitiveRenderer.contains("MAX_WORLD_TEXT_QUADS_PER_FRAME = 65_536")
			&& worldPrimitiveRenderer.contains("MAX_WORLD_TEXT_IMAGE_RESIDENCY = 4_096")
			&& rustWorldTextFrontend.contains("MAX_WORLD_TEXT_RESOURCES: usize = 8_192")
			&& rustWorldTextFrontend.contains("world text resource cache exceeds bounded limit")
			&& worldPrimitiveRenderer.contains("MAX_WORLD_LINE_SEGMENTS = 65_536")
			&& worldPrimitiveRenderer.contains("MAX_WORLD_CRACK_QUADS = 512")
			&& worldPrimitiveRenderer.contains("MAX_WORLD_BORDER_QUADS = 64")
			&& worldPrimitiveRenderer.contains("MAX_SEMANTIC_VIEWPORT_AXIS = 16_384")
			&& worldPrimitiveRenderer.contains("new BoundedSemanticQueue<>(MAX_RUST_WORLD_MATERIAL_QUADS, \"material-quad\")")
			&& worldPrimitiveRenderer.contains("class BoundedSemanticQueue<E> extends ArrayList<E>")
			&& worldPrimitiveRenderer.contains("ensureWorldQueueCapacityLocked")
			&& worldPrimitiveRenderer.contains("ensureWorldMeshRegistryCapacityLocked")
			&& worldPrimitiveRenderer.contains("readBoundedResourceBytes(input")
			&& worldPrimitiveRenderer.contains("semantic sky texture")
			&& worldPrimitiveRenderer.contains("exceeds the \" + maximumBytes + \" byte bound"),
			"world semantic resource copies must enforce the Rust-owned bounded payload contracts before FFI");
		String renderSystem = Files.readString(Path.of("src/main/java/net/blaze3d/systems/RenderSystem.java"));
		String renderStateShard = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/RenderStateShard.java"));
		String window = Files.readString(Path.of("src/main/java/net/blaze3d/platform/Window.java"));
		String vulkanicApi = Files.readString(Path.of("src/main/java/net/vulkanic/VulkanicAPI.java"));
		String vulkanBackend = Files.readString(Path.of("src/main/java/net/vulkanic/backends/vulkan/VulkanBackend.java"));
		String rustBackends = Files.readString(Path.of("src/main/rust/render/vulkanic/backends/mod.rs"));
		String rustVulkan = Files.readString(Path.of("src/main/rust/render/vulkanic/backends/vulkan/mod.rs"));
		String rustGuiFrontend = Files.readString(Path.of("src/main/rust/render/vulkanic/gui_frontend.rs"));
		String rustFfi = readRustFfiModules();
		String cubeMap = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/CubeMap.java"));
		String panoramaRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/PanoramaRenderer.java"));
		String semanticPanorama = Files.readString(Path.of("src/main/java/net/vulkanic/gui/RustGalPanoramaRenderer.java"));
		String wholeFrameTextureAtlas = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/texture/TextureAtlas.java"));
		String wholeFrameTiming = Files.readString(Path.of("src/main/java/net/vulkanic/bridge/RustGalDeterministicTiming.java"));

		assertTrue(mode.contains("mattmc.dev.rustGalVulkanWholeFrame"));
		assertTrue(mode.contains("isRustPresentationActive"));
		assertTrue(mode.contains("activateRustPresentation"));
		assertTrue(bridge.contains("mattmc_vulkanic_gal_context_create_windowed_vulkan"));
		assertTrue(bridge.contains("WINDOWED_VULKAN_CONTEXT_CREATE(49)"));
		assertTrue(queue.contains("createWindowedVulkan"));
		assertTrue(queue.contains("executeWholeFrameVulkan"));
		assertTrue(queue.contains("enum BridgeMode"));
		assertTrue(queue.contains("WINDOWED_VULKAN"));
		assertTrue(queue.contains("RustGalVulkanWholeFrameMode.activateRustPresentation"));
		assertTrue(queue.contains("whole-frame execution cannot reuse a"),
			"whole-frame Vulkan must fail closed instead of reusing a borrowed OpenGL bridge");
		assertTrue(queue.contains("borrowed OpenGL execution cannot reuse a"),
			"the partial-frame route must likewise reject a native Vulkan bridge");
		assertFalse(queue.contains("GLFWNativeX11.glfwGetX11Window"));
		assertFalse(queue.contains("GLFWNativeWayland.glfwGetWaylandWindow"));
		assertTrue(bridge.contains("GLFWNativeX11.glfwGetX11Window"));
		assertTrue(bridge.contains("GLFWNativeWayland.glfwGetWaylandWindow"));
		assertTrue(minecraft.contains("renderRustVulkanWholeFrameShell"));
		assertTrue(minecraft.contains("game.rendering.rust-vulkan-whole-frame"));
		int fpsHandleLookup = minecraft.indexOf("net.vulkanic.VulkanicCoreAPI.textureId(mainColorTexture)");
		int fpsWholeFrameGuard = minecraft.lastIndexOf("RustGalVulkanWholeFrameMode.enabled()", fpsHandleLookup);
		assertTrue(fpsHandleLookup > 0 && fpsWholeFrameGuard >= 0 && fpsWholeFrameGuard < fpsHandleLookup
			&& minecraft.contains("rust-semantic-frame-target"),
			"whole-frame diagnostics must not query a Java main-target native texture handle");
		int irisKeybinds = minecraft.indexOf("net.irisshaders.iris.Iris.handleKeybinds(this)");
		String irisKeybindContext = irisKeybinds < 0 ? ""
			: minecraft.substring(Math.max(0, irisKeybinds - 320), irisKeybinds);
		assertTrue(irisKeybinds > 0
			&& irisKeybindContext.contains("RustGalVulkanWholeFrameMode.enabled()")
			&& irisKeybindContext.contains("!net.vulkanic.VulkanicAPI.isVulkanBackendSelected()"),
			"whole-frame Vulkan must not invoke Iris's Java shader-toggle runtime");
		int setLevelIrisDimension = minecraft.indexOf("net.irisshaders.iris.Iris.lastDimension =");
		String setLevelIrisContext = setLevelIrisDimension < 0 ? ""
			: minecraft.substring(Math.max(0, setLevelIrisDimension - 320), setLevelIrisDimension);
		assertTrue(setLevelIrisDimension > 0
			&& setLevelIrisContext.contains("RustGalVulkanWholeFrameMode.enabled()")
			&& setLevelIrisContext.contains("!net.vulkanic.VulkanicAPI.isVulkanBackendSelected()"),
			"whole-frame level changes must not publish Java Iris dimension state");
		int pipelineDimension = minecraft.indexOf("net.irisshaders.iris.Iris.getPipelineManager().destroyPipeline()");
		String pipelineDimensionContext = pipelineDimension < 0 ? ""
			: minecraft.substring(Math.max(0, pipelineDimension - 800), pipelineDimension);
		assertTrue(pipelineDimension > 0
			&& pipelineDimensionContext.contains("RustGalVulkanWholeFrameMode.enabled()")
			&& pipelineDimensionContext.contains("!net.vulkanic.VulkanicAPI.isVulkanBackendSelected()"),
			"whole-frame level changes must not destroy or prepare Java Iris pipelines");
		assertTrue(minecraft.contains("!net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()")
			&& minecraft.contains("preloadUiShader"),
			"Rust whole-frame startup must not precompile Java GUI pipelines");
		assertTrue(minecraft.contains("!net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()")
			&& minecraft.contains("!net.vulkanic.VulkanicAPI.isVulkanBackendSelected()")
			&& minecraft.contains("!iris$initialized")
			&& minecraft.contains("Whole-frame Vulkan does not borrow Iris's renderer lifecycle"),
			"Rust whole-frame startup must not initialize Iris before presentation ownership transfers");
		int voxelMapBootstrap = minecraft.indexOf("VoxelMap still owns Java offscreen textures and render passes");
		int voxelMapInitialize = minecraft.indexOf("VoxelMapInitializer.initialize();", voxelMapBootstrap);
		String voxelMapContext = voxelMapInitialize < 0 ? ""
			: minecraft.substring(Math.max(voxelMapBootstrap, voxelMapInitialize - 360), voxelMapInitialize);
		assertTrue(voxelMapBootstrap >= 0 && voxelMapInitialize > voxelMapBootstrap
			&& voxelMapContext.contains("RustGalVulkanWholeFrameMode.enabled()")
			&& voxelMapContext.contains("!net.vulkanic.VulkanicAPI.isVulkanBackendSelected()"),
			"whole-frame startup must keep VoxelMap's Java renderer unavailable");
		assertTrue(gameRenderer.contains("rustVulkanWholeFrameGuiExtraction"));
		int legacyIrisFrameState = gameRenderer.indexOf("net.irisshaders.iris.uniforms.CapturedRenderingState.INSTANCE.setRealTickDelta");
		assertTrue(gameRenderer.contains("RustGalDeterministicTiming.partialTick(deltaTracker)")
			&& legacyIrisFrameState > gameRenderer.indexOf("if (!rustWholeFrame) {"),
			"the whole-frame route must use renderer-neutral timing before Iris frame state is touched");
		assertTrue(wholeFrameTiming.contains("mattmc.vulkan.deterministicTemporalParity.partialTick")
			&& !wholeFrameTiming.contains("net.irisshaders"),
			"whole-frame deterministic timing must preserve parity fixture settings without Iris runtime internals");
		assertTrue(gameRenderer.contains("gui.screen-semantic-extraction"),
			"the Rust whole-frame shell must extract screen semantics rather than leaving the main menu unrendered");
		assertTrue(gameRenderer.contains("gui.loading-overlay-semantic-extraction"),
			"startup overlay primitives must be submitted to Rust rather than skipped before the title screen");
		int rustShellStart = gameRenderer.indexOf("public boolean renderRustVulkanWholeFrameShell");
		int overlayExtraction = gameRenderer.indexOf("gui.loading-overlay-semantic-extraction", rustShellStart);
		String overlayExtractionContext = gameRenderer.substring(Math.max(rustShellStart, overlayExtraction - 360), overlayExtraction);
		assertTrue(rustShellStart >= 0 && overlayExtraction > rustShellStart
			&& overlayExtractionContext.contains("if (this.minecraft.getOverlay() != null)"),
			"the Rust loading-overlay semantic producer must match Frozen and run while the normal in-game GUI predicate is false during reload");
		int screenExtraction = gameRenderer.indexOf("gui.screen-semantic-extraction", rustShellStart);
		String screenExtractionContext = gameRenderer.substring(Math.max(rustShellStart, screenExtraction - 360), screenExtraction);
		assertTrue(screenExtraction > rustShellStart
			&& screenExtractionContext.contains("else if (gameLoadFinished && this.minecraft.screen != null)"),
			"the Rust screen semantic producer must match Frozen during the overlay-to-title handoff instead of clearing an empty frame when the normal in-game predicate is false");
		assertTrue(Files.readString(Path.of("src/main/java/net/minecraft/client/gui/screens/LoadingOverlay.java"))
			.contains("renderCompatibleScreen"),
			"loading-overlay screen delegation must remain on the semantic GUI extraction path");
		String titleScreen = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/screens/TitleScreen.java"));
		assertTrue(titleScreen.contains("!net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()")
			&& titleScreen.contains("!net.vulkanic.VulkanicAPI.isVulkanBackendSelected()")
			&& titleScreen.contains("!iris$hasFirstInit"),
			"the admitted title route must not initialize Iris renderer runtime state");
		assertTrue(hud.contains("boolean legacyIrisDebugGroup = !net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()")
			&& hud.contains("if (legacyIrisDebugGroup) {"),
			"whole-frame HUD extraction must not enter Iris GL debug state");
		assertTrue(hud.contains("VoxelMap") && hud.contains("semantic producer owns fail-closed admission"),
			"the Rust Vulkan HUD path must stay semantic and fail closed without reopening Java GPU rendering");
		assertTrue(gameRenderer.contains("renderWithTooltipAndSubtitles(guiGraphics"));
		assertFalse(gameRenderer.contains("instanceof net.minecraft.client.gui.screens.TitleScreen"),
			"the Rust whole-frame shell must not restrict semantic extraction to the title screen");
		assertFalse(gameRenderer.contains("instanceof net.minecraft.client.gui.screens.LevelLoadingScreen"),
			"world-load progress must use the broad semantic GUI extraction path rather than a screen-specific Java renderer");
		String loadingOverlay = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/screens/LoadingOverlay.java"));
		assertTrue(loadingOverlay.contains("RustGalFrameCoordinator rejects any element family")
			&& !loadingOverlay.contains("instanceof LevelLoadingScreen"),
			"loading overlay must use broad semantic screen extraction rather than a Java screen whitelist");
		assertTrue(cubeMap.contains("requires PanoramaRenderer to attach its semantic command")
			&& cubeMap.contains("Java cube-map rendering is not a fallback"),
			"direct CubeMap calls cannot enqueue detached work; selected Vulkan must fail closed without Java rendering");
		assertTrue(panoramaRenderer.contains("RustGalPanoramaRenderer.enqueue"));
		assertTrue(semanticPanorama.contains("resolveCubeMap") && semanticPanorama.contains("enqueueGuiMeshItemRequest"),
			"the title panorama must cross the boundary as copied semantic image data and Rust-owned mesh work");
		assertTrue(gameRenderer.contains("gui.rectangle-semantic-enqueue"));
		assertTrue(guiRenderer.contains("collectRustGalRectangleSemantics()"));
		assertTrue(guiRenderer.contains("ColoredRectangleRenderState rectangle"));
		assertTrue(guiRenderer.contains("FourColoredRectangleRenderState rectangle")
			&& guiRenderer.contains("tryEnqueueFourColoredRectangle"),
			"VoxelMap four-corner gradients must enter the explicit Rust GUI mesh path");
		String voxelMapGuiGraphics = Files.readString(Path.of("src/main/java/net/voxelmap/util/VoxelMapGuiGraphics.java"));
		assertTrue(voxelMapGuiGraphics.contains("requires a semantic ResourceLocation")
			&& voxelMapGuiGraphics.contains("rejectRustTextureView")
			&& voxelMapGuiGraphics.contains("VulkanicAPI.isVulkanBackendSelected()"),
			"VoxelMap texture-view GUI helpers must fail closed on Rust Vulkan instead of crossing Java GPU views");
		assertTrue(guiState.contains("this.current = currentNode;"),
			"semantic GUI extraction must retain the source stratum when it appends explicit commands");
		assertTrue(guiState.contains("currentSemanticLayerOrder(GuiRenderState.SemanticPhase phase)"));
		assertTrue(guiState.contains("ELEMENTS(0)") && guiState.contains("ITEMS(1)") && guiState.contains("TEXT(2)"),
			"semantic GUI ordering must mirror GuiRenderer's element/item/text preparation phases");
		assertTrue(guiRenderer.contains("currentSemanticLayerOrder(GuiRenderState.SemanticPhase.TEXT)"));
		assertTrue(guiRenderer.contains("currentSemanticLayerOrder(GuiRenderState.SemanticPhase.ELEMENTS)"));
		assertTrue(queue.contains("String semanticLayerId") && queue.contains("int semanticLayerOrder"),
			"whole-frame GUI requests must carry an explicit source-layer order rather than a fixed HUD-only stratum");
		assertTrue(blitState.contains("@Nullable ResourceLocation semanticTexture"),
			"generic GUI blits must retain a resource identity rather than requiring a GPU-view lookup");
		assertTrue(guiGraphics.contains("submitBlit(renderPipeline, gpuTextureView, resourceLocation"),
			"GuiGraphics must attach the original resource location at the semantic callsite");
		assertTrue(guiGraphics.contains("RustGalGuiRenderer.isWholeFrameVulkanEnabled()")
			&& guiGraphics.contains("VulkanicAPI.isVulkanBackendSelected()")
			&& guiGraphics.contains("TextureSetup.noTexture()"),
			"selected Vulkan semantic blits must not materialize Java texture views before Rust copies their resource bytes");
		assertTrue(guiGraphics.contains("ResourceLocation mapTexture = mapRenderState.texture")
			&& guiGraphics.contains("atlasTexture = textureAtlasSprite.atlasLocation()")
			&& guiGraphics.contains("decoration.atlasSprite"),
			"map GUI must use copied semantic blits for the map texture and decorations rather than Java texture views");
		assertTrue(guiSemanticRenderer.contains("semanticSingleTexture"),
			"the Rust semantic GUI frontend must accept explicit resource identities without Java texture views");
		assertTrue(wholeFrameTextureAtlas.contains("Semantic GUI consumers retain only the stitched CPU source")
			&& wholeFrameTextureAtlas.contains("if (!rustWholeFrame) {")
			&& wholeFrameTextureAtlas.contains("this.createTexture(preparations.width(), preparations.height(), preparations.mipLevel());"),
			"whole-frame GUI atlases must not allocate Java GPU textures before Rust stages the snapshot");
		assertTrue(guiSemanticRenderer.contains("tryEnqueueUniformRectangle("));
		assertTrue(guiSemanticRenderer.contains("currentExecutionRoute() != GuiExecutionRoute.RUST_VULKAN_WHOLE_FRAME"),
			"rectangle semantics must not leak into the borrowed OpenGL GUI route");
		assertTrue(guiSemanticRenderer.contains("tryEnqueueVerticalGradientRectangle"),
			"gradient rectangles must use Rust-owned mesh interpolation rather than a Java rendering fallback");
		assertTrue(guiSemanticRenderer.contains("color00()") && guiSemanticRenderer.contains("color11()")
			&& guiSemanticRenderer.contains("voxelmap.gui.four-colored-rectangle"),
			"four-corner GUI gradients must preserve all semantic vertex colors in Rust-owned mesh data");
		assertTrue(guiSemanticRenderer.contains("clippedLeft")
			&& guiSemanticRenderer.contains("right <= 0 || bottom <= 0"),
			"off-screen four-corner GUI gradients must use viewport-clipped semantic metadata instead of being dropped");
		assertTrue(guiSemanticRenderer.contains("RECTANGLE_PRODUCER + \".gradient\""));
		assertTrue(guiSemanticRenderer.contains("SOLID_WHITE_ASSET_ID"));
		String tiledProducer = guiSemanticRenderer.substring(
			guiSemanticRenderer.indexOf("public static List<RustGalGuiElementRenderState> tryEnqueueTiledCopiedBlit("),
			guiSemanticRenderer.indexOf("static List<float[]> wrappedUnitIntervalSegments("));
		assertTrue(tiledProducer.contains("new VulkanicGalBridge.GuiTiledQuadRecord(")
			&& tiledProducer.contains("enqueueGuiTiledQuadRequest("),
			"tiled GUI must send one semantic parent for Rust's bounded whole-frame preflight");
		assertFalse(tiledProducer.contains("rustGuiTiledCommands"), "normal tiled transport must not require a diagnostic flag");
		assertFalse(tiledProducer.contains("for ("), "Java must not expand semantic tile geometry");
		assertFalse(tiledProducer.contains("GuiAffineQuadRecord"), "Java must not substitute expanded affine children");
		assertTrue(guiStratum.contains("GUI_RECTANGLES(\"gui.rectangles\", 100)"));
		assertTrue(gameRenderer.contains("enqueueRustGalIndexedMeshFeaturesForWholeFrame"));
		assertTrue(levelRenderer.contains("enqueueRustGalIndexedMeshFeaturesForWholeFrame"));
		assertTrue(levelRenderer.contains("this.entityRenderDispatcher"));
		assertTrue(levelRenderer.contains(".submit("));
		assertTrue(levelRenderer.contains("rustWorldTextSubmitNodeStorage"));
		assertTrue(levelRenderer.contains("submitWholeFrameWorldText"));
		assertTrue(levelRenderer.contains("WorldTextSubmitSemanticCollector"));
		assertTrue(levelRenderer.contains("this.entityRenderDispatcher.submitSemantic("));
		assertTrue(levelRenderer.contains("this.blockEntityRenderDispatcher.submitSemantic("));
		String signRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/blockentity/AbstractSignRenderer.java"));
		assertTrue(signRenderer.contains("submitNodeCollector.submitModelSemantic(")
			&& signRenderer.contains("submitNodeCollector.submitTextSemantic(")
			&& signRenderer.contains("RustGalVulkanWholeFrameMode.enabled()")
			&& !signRenderer.contains("submitNodeCollector.submitText("),
			"Rust whole-frame sign text must use copied semantic world-text submission without a legacy fallback");
		String displayRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/entity/DisplayRenderer.java"));
		assertTrue(displayRenderer.contains("orderedSubmitNodeCollector.submitTextSemantic(")
			&& !displayRenderer.contains("orderedSubmitNodeCollector.submitText("),
			"display-entity text must enter the Rust world-text collector explicitly without a legacy fallback");
		int selectedBlockEntities = levelRenderer.indexOf("private void submitSelectedWholeFrameModelBlockEntities");
		int pistonWholeFrame = levelRenderer.indexOf("private void submitPistonMovingBlocksForWholeFrame");
		assertTrue(selectedBlockEntities >= 0
			&& levelRenderer.indexOf("this.blockEntityRenderDispatcher.submitSemantic(", selectedBlockEntities) > selectedBlockEntities,
			"Rust whole-frame model block-entity replay must use the explicit semantic dispatcher");
		assertTrue(pistonWholeFrame >= 0
			&& levelRenderer.indexOf("this.blockEntityRenderDispatcher.submitSemantic(", pistonWholeFrame) > pistonWholeFrame,
			"Rust whole-frame piston replay must use the explicit semantic dispatcher");
		assertTrue(levelRenderer.contains("this.target.submitNameTag("));
		assertTrue(levelRenderer.contains("this.target.submitTextSemantic("));
		assertTrue(worldPrimitiveRenderer.contains("recordWorldTextTextSnapshot(submits, result)"));
		assertTrue(featureRenderDispatcher.contains("collectRustWorldTextSemanticsForWholeFrame(SubmitNodeStorage textSubmitStorage)"));
		assertTrue(modelFeatureRenderer.contains("Java model feature rendering is unavailable while Rust owns whole-frame presentation")
			&& itemFeatureRenderer.contains("Java item feature rendering is unavailable while Rust owns whole-frame presentation")
			&& textFeatureRenderer.contains("Java text feature rendering is unavailable while Rust owns whole-frame presentation"),
			"Java model, item, and text feature renderers must fail closed after Rust presentation ownership transfers");
		assertTrue(featureRenderDispatcher.contains("textSubmitStorage.getSubmitsPerOrder()"));
		int rustWorldTextRoute = featureRenderDispatcher.indexOf("if (WorldRenderRoutePolicy.currentWorldTextRoute().usesRustWholeFrameVulkan())");
		int javaWorldTextCompatibility = featureRenderDispatcher.indexOf("} else if (!WorldRenderRoutePolicy.currentWorldTextRoute().equals(WorldRenderRoutePolicy.Route.DISABLED))", rustWorldTextRoute);
		int javaTextDraw = featureRenderDispatcher.indexOf("this.textFeatureRenderer.render", rustWorldTextRoute);
		assertTrue(rustWorldTextRoute >= 0 && javaWorldTextCompatibility > rustWorldTextRoute);
		assertTrue(javaTextDraw > javaWorldTextCompatibility,
			"the selected Rust world-text route must enqueue semantics rather than invoke Java text rendering");
		String worldTextCaptureSource = Files.readString(Path.of("src/main/java/net/minecraft/client/dev/DeterministicCameraCapture.java"));
		assertTrue(worldTextCaptureSource.contains("mattmc.dev.rustGalWorldText.requireSourceCapture"));
		assertTrue(worldTextCaptureSource.contains("new Display.TextDisplay(EntityType.TEXT_DISPLAY"));
		assertTrue(worldTextCaptureSource.contains("Display.TextDisplay.FLAG_SEE_THROUGH"));
		assertTrue(worldTextCaptureSource.contains("mattmc.dev.rustGalWorldItemEntity.requireSourceCapture"));
		assertTrue(worldTextCaptureSource.contains("configureWorldTextCaptureCarrier(entity, scenario)"));
		assertTrue(worldTextCaptureSource.contains("world_text_execution")
			&& worldTextCaptureSource.contains("readJsonLongField(json.substring(textStart), \"draws\", 0L) <= 0L"),
			"the source capture must require executed Rust text, not just a name-tag producer callback");
		assertTrue(worldTextCaptureSource.contains("minecraft:item_entity/ground"),
			"the source capture must require the semantic dropped-item identity, not unrelated entity meshes");
		int wholeFrameAssetFlush = queue.indexOf("flushPendingWorldAssetsLocked();", queue.indexOf("if (wholeFrameVulkan) {"));
		int wholeFrameFrameConsume = queue.indexOf("primitiveFrame = RustGalWorldPrimitiveRenderer.consumeFrame();", wholeFrameAssetFlush);
		assertTrue(wholeFrameAssetFlush >= 0 && wholeFrameFrameConsume > wholeFrameAssetFlush);
		int lateWorldAssetFlush = queue.indexOf("flushPendingWorldAssetsAfterFrameConsumeLocked();", wholeFrameFrameConsume);
		assertTrue(lateWorldAssetFlush > wholeFrameFrameConsume,
			"resources discovered while freezing a whole frame must publish before native submission");
		assertTrue(queue.contains("flushPendingWorldAssetsLocked(true);"),
			"the post-consume flush must include mesh assets and textures rather than leaving a stale native registry");
		int shellBlockDisplaysStart = levelRenderer.indexOf("enqueueRustGalIndexedMeshFeaturesForWholeFrame");
		int shellBlockDisplaysEnd = levelRenderer.indexOf("public void extractVisibleBlockEntities", shellBlockDisplaysStart);
		String shellBlockDisplays = levelRenderer.substring(shellBlockDisplaysStart, shellBlockDisplaysEnd);
		assertFalse(shellBlockDisplays.contains("isSectionCompiled"));
		assertTrue(shellBlockDisplays.contains("BlockDisplayEntityRenderState"));
		assertTrue(shellBlockDisplays.contains("FallingBlockRenderState"));
		assertTrue(shellBlockDisplays.contains("TntRenderState"));
		assertTrue(shellBlockDisplays.contains("currentPrimedTntRoute()"));
		String deterministicCapture = Files.readString(Path.of("src/main/java/net/minecraft/client/dev/DeterministicCameraCapture.java"));
		assertTrue(deterministicCapture.contains("RUST_FINAL_OUTPUT_EVERY_POSE"));
		assertTrue(deterministicCapture.contains("captureWholeFrameAttachmentsForPose"));
		assertTrue(deterministicCapture.contains("resetWholeFrameAttachmentCaptureState();"));
		assertTrue(shellBlockDisplays.contains("submitPistonMovingBlocksForWholeFrame"));
		assertTrue(shellBlockDisplays.contains("ensurePistonMovingBlocksPresentForWholeFrame"));
		assertTrue(shellBlockDisplays.contains("currentPistonMovingBlockRoute()"));
		assertTrue(shellBlockDisplays.contains("entityShadows")
			&& shellBlockDisplays.contains("entityFlames")
			&& shellBlockDisplays.contains("entityLeashes")
			&& shellBlockDisplays.contains("debugLines")
			&& shellBlockDisplays.contains("|| entityShadows || entityFlames || entityLeashes || debugLines"),
			"indexed semantic extraction must not early-return before dispatcher-owned shadow, flame, leash, or debug-line routes");
		assertTrue(shellBlockDisplays.contains("blockEntitySemanticFamilies")
			&& shellBlockDisplays.contains("currentBeaconBeamRoute()")
			&& shellBlockDisplays.contains("currentGuardianBeamRoute()")
			&& shellBlockDisplays.contains("currentCrystalBeamRoute()"),
			"block-entity semantic traversal must remain live for standalone beam and billboard routes");
		assertTrue(levelRenderer.contains("PistonHeadRenderState"));
		assertTrue(levelRenderer.contains("ZombieVillagerModel")
			&& levelRenderer.contains("SheepModel")
			&& levelRenderer.contains("textures/entity/sheep/sheep.png"),
			"selected-source coverage must recognize Rust-owned zombie-villager and sheep body meshes");
		assertTrue(levelRenderer.contains("BeaconRenderState")
			&& levelRenderer.contains("EndPortalRenderState")
			&& levelRenderer.contains("CondiutRenderState")
			&& levelRenderer.contains("SpawnerRenderState")
			&& levelRenderer.contains("BlockEntityWithBoundingBoxRenderState")
			&& levelRenderer.contains("TestInstanceRenderState")
			&& levelRenderer.contains("CampfireRenderState")
			&& levelRenderer.contains("BrushableBlockRenderState")
			&& levelRenderer.contains("ShelfRenderState")
			&& levelRenderer.contains("VaultRenderState"),
			"Rust whole-frame block-entity replay must admit semantic geometry and indexed-item producers");
		String blockEntityDispatcher = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/blockentity/BlockEntityRenderDispatcher.java"));
		assertTrue(blockEntityDispatcher.contains("beginBlockEntityItemSubmission()")
			&& blockEntityDispatcher.contains("endBlockEntityItemSubmission()")
			&& blockEntityDispatcher.contains("!submitNodeCollector.isSemanticCoverageOnly()")
			&& blockEntityDispatcher.contains("currentModelPartMeshRoute(true).usesRustWholeFrameVulkan()")
			&& blockEntityDispatcher.contains("currentItemEntityMeshRoute(true).usesRustWholeFrameVulkan()"),
			"block-entity item scopes must be explicit, balanced, and absent from coverage-only traversal");
		assertTrue(worldPrimitiveRenderer.contains("BLOCK_ENTITY_ITEM")
			&& worldPrimitiveRenderer.contains("isBlockEntityItemSubmissionActive()"),
			"block-entity item meshes must retain a distinct semantic producer identity");
		assertTrue(worldPrimitiveRenderer.contains("block-entity item semantic scope ended without a matching begin")
			&& worldPrimitiveRenderer.contains("if (depth <= 0)"),
			"block-entity item semantic scopes must reject underflow instead of leaking producer identity");
		assertTrue(levelRenderer.contains("this.extractVisibleBlockEntities(camera, deltaTracker.getGameTimeDeltaPartialTick(false), this.levelRenderState);"));
		String chestRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/blockentity/ChestRenderer.java"));
		assertTrue(chestRenderer.contains("submitNodeCollector.submitModelSemantic(")
			&& chestRenderer.contains("doubleLeftModel")
			&& chestRenderer.contains("doubleRightModel"),
			"all chest model variants must enter the explicit Rust semantic model callback");
		String enchantTableRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/blockentity/EnchantTableRenderer.java"));
		assertTrue(enchantTableRenderer.contains("submitNodeCollector.submitModelSemantic(")
			&& enchantTableRenderer.contains("BookModel.State"),
			"the enchanting-table book must enter the explicit Rust semantic model callback");
		String bellRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/blockentity/BellRenderer.java"));
		assertTrue(bellRenderer.contains("submitNodeCollector.submitModelSemantic(")
			&& bellRenderer.contains("BellModel.State"),
			"the animated bell must enter the explicit Rust semantic model callback");
		String shulkerRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/blockentity/ShulkerBoxRenderer.java"));
		assertTrue(shulkerRenderer.contains("submitNodeCollector.submitModelSemantic(")
			&& shulkerRenderer.contains("ShulkerBoxModel"),
			"shulker boxes must enter the explicit Rust semantic model callback");
		String skullRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/blockentity/SkullBlockRenderer.java"));
		assertTrue(skullRenderer.contains("submitNodeCollector.submitModelSemantic(")
			&& skullRenderer.contains("submitNodeCollector.submitModelSemanticTexture(")
			&& skullRenderer.contains("semanticTexture")
			&& skullRenderer.contains("playerSkin().body().texturePath()")
			&& skullRenderer.contains("defaultTexture"),
			"skull models must enter explicit Rust semantic model callbacks with texture identity preserved");
		String lecternRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/blockentity/LecternRenderer.java"));
		assertTrue(lecternRenderer.contains("submitNodeCollector.submitModelSemantic(")
			&& lecternRenderer.contains("BookModel.State"),
			"lectern books must enter the explicit Rust semantic model callback");
		String signItemRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/blockentity/SignRenderer.java"));
		assertTrue(signItemRenderer.contains("submitNodeCollector.submitModelSemantic(")
			&& signItemRenderer.contains("submitSpecial"),
			"sign item models must enter the explicit Rust semantic model callback");
		String hangingSignRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/blockentity/HangingSignRenderer.java"));
		assertTrue(hangingSignRenderer.contains("submitNodeCollector.submitModelSemantic(")
			&& hangingSignRenderer.contains("submitSpecial"),
			"hanging-sign item models must enter the explicit Rust semantic model callback");
		String bedRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/blockentity/BedRenderer.java"));
		assertTrue(bedRenderer.contains("submitNodeCollector.submitModelSemantic(")
			&& bedRenderer.contains("this.headModel")
			&& bedRenderer.contains("this.footModel"),
			"both bed model pieces must enter the explicit Rust semantic model callback");
		String chestSpecialRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/special/ChestSpecialRenderer.java"));
		assertTrue(chestSpecialRenderer.contains("submitNodeCollector.submitModelSemantic(")
			&& chestSpecialRenderer.contains("this.model")
			&& chestSpecialRenderer.contains("this.openness"),
			"special-item chest models must enter the explicit Rust semantic model callback");
		String bannerRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/blockentity/BannerRenderer.java"));
		assertTrue(bannerRenderer.contains("submitNodeCollector.submitModelSemantic(")
			&& !bannerRenderer.contains("submitNodeCollector.submitModel(")
			&& bannerRenderer.contains("RenderType.entityGlint()"),
			"banner base, pattern, and glint models must all enter explicit Rust semantic callbacks");
		String conduitRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/blockentity/ConduitRenderer.java"));
		assertTrue(conduitRenderer.contains("submitNodeCollector.submitModelPartSemantic(")
			&& !conduitRenderer.contains("submitNodeCollector.submitModelPart("),
			"conduit model parts must enter the explicit Rust semantic model-part callback");
		String decoratedPotRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/blockentity/DecoratedPotRenderer.java"));
		assertTrue(decoratedPotRenderer.contains("submitNodeCollector.submitModelPartSemantic(")
			&& !decoratedPotRenderer.contains("submitNodeCollector.submitModelPart("),
			"decorated-pot model parts must enter the explicit Rust semantic model-part callback");
		String itemFrameRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/entity/ItemFrameRenderer.java"));
		String displayRendererBlock = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/entity/DisplayRenderer.java"));
		String carriedBlockLayer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/entity/layers/CarriedBlockLayer.java"));
		assertTrue(itemFrameRenderer.contains("submitBlockDisplaySemantic(")
			&& displayRendererBlock.contains("submitBlockDisplaySemantic(")
			&& carriedBlockLayer.contains("submitBlockSemantic("),
			"entity block-state producers must enter explicit Rust semantic block callbacks");
		String snowGolemHeadLayer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/entity/layers/SnowGolemHeadLayer.java"));
		String mushroomCowLayer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/entity/layers/MushroomCowMushroomLayer.java"));
		assertTrue(itemFrameRenderer.contains("submitBlockModelSemantic(")
			&& snowGolemHeadLayer.contains("submitBlockModelSemantic(")
			&& mushroomCowLayer.contains("submitBlockModelSemantic("),
			"block-model compatibility branches must retain the explicit semantic callback boundary");
		assertTrue(levelRenderer.contains("PistonMovingBlockEntity pistonMovingBlockEntity"));
		assertTrue(levelRenderer.contains("this.blockEntityRenderDispatcher.tryExtractRenderState"));
		assertTrue(levelRenderer.contains("this.level.getChunkSource().getChunk(chunkX, chunkZ, ChunkStatus.FULL, false)"));
		assertTrue(levelRenderer.contains("RustGalWorldPrimitiveRenderer.recordMovingBlockShellScan"));
		assertTrue(featureRenderDispatcher.contains("renderBlockFeaturesOnly"));
		assertTrue(featureRenderDispatcher.contains("blockFeatureRenderer.render"));
		assertTrue(gameRenderer.contains("RustGalFrameCoordinator.executeWholeFrameVulkan"));
		int shellStart = gameRenderer.indexOf("renderRustVulkanWholeFrameShell");
		int shellEnd = gameRenderer.indexOf("private void tryTakeScreenshotIfNeeded", shellStart);
		String shell = gameRenderer.substring(shellStart, shellEnd);
		assertFalse(shell.contains("renderLevel(deltaTracker)"));
		assertTrue(shell.contains("this.renderDistance = this.minecraft.options.getEffectiveRenderDistance() * 16;"),
			"the whole-frame shell must initialize the same projection depth distance as the normal world renderer");
		assertTrue(shell.contains("RustGalFrameCoordinator.isRustShaderPackSourceReady()"),
			"whole-frame camera semantics must use the staged Rust-owned shader-pack source readiness signal");
		assertFalse(shell.contains("Iris.isPackInUseQuick()"),
			"whole-frame camera semantics must not query Iris renderer runtime state");
		assertTrue(renderSystem.contains("RustGalVulkanWholeFrameMode.enabledForBackend"));
		assertTrue(renderSystem.indexOf("RustGalVulkanWholeFrameMode.enabledForBackend") < renderSystem.indexOf("VulkanicAPI.beginFrame()"));
		assertTrue(window.contains("RustGalVulkanWholeFrameMode.enabled()"));
		assertTrue(vulkanicApi.contains("Java Vulkan beginFrame is disabled while Rust owns whole-frame Vulkan presentation"));
		int fencedTaskDrain = vulkanicApi.indexOf("public static void executePendingFenceTasks()");
		int wholeFrameFenceGuard = vulkanicApi.indexOf("RustGalVulkanWholeFrameMode.enabled()", fencedTaskDrain);
		int pendingFenceContext = vulkanicApi.indexOf("CommandContext ctx = getCommandContext();", fencedTaskDrain);
		assertTrue(fencedTaskDrain >= 0 && wholeFrameFenceGuard > fencedTaskDrain && pendingFenceContext > wholeFrameFenceGuard,
			"whole-frame Vulkan must reject Java fenced callbacks before querying a Java command context");
		assertTrue(vulkanicApi.contains("Java Vulkan presentTextureToScreen is disabled while Rust owns whole-frame Vulkan presentation"));
		assertEquals(2, occurrences(vulkanicApi, "Java Vulkan texture-unit binding is unavailable; Rust owns the selected Vulkan route"),
			"both Java Vulkan texture binding overloads must fail closed for the selected Rust route");
		assertTrue((renderStateShard.contains("Java Vulkan render-state setup is unavailable while Rust owns whole-frame presentation")
			|| renderStateShard.contains("Java Vulkan render-state setup is unavailable on selected Vulkan"))
			&& (renderStateShard.contains("Java Vulkan render-state cleanup is unavailable while Rust owns whole-frame presentation")
			|| renderStateShard.contains("Java Vulkan render-state cleanup is unavailable on selected Vulkan")),
			"Java RenderStateShard compatibility callbacks must fail closed after Rust presentation ownership transfers");
		assertTrue(vulkanicApi.contains("RustGalVulkanWholeFrameMode.enabled()")
			&& !vulkanicApi.contains("shellCompatibilityBackend()"),
			"the Java OpenGL bootstrap backend must not remain reachable after Rust owns Vulkan selection");
		assertTrue(vulkanicApi.contains("The whole-frame route uses only semantic CPU buffers here"),
			"Rust presentation must not keep Java's dynamic-uniform GL ring active");
		assertFalse(vulkanicApi.contains("return getDevice().createCommandEncoder();"),
			"whole-frame command-encoder creation must fail closed instead of using a Java compatibility device");
		assertTrue(vulkanicApi.contains("Java Vulkan backend method '"),
			"the Java Vulkan proxy must become non-rendering once Rust owns presentation");
		assertTrue(vulkanicApi.contains("isRustWholeFrameBootstrapMethod")
			&& vulkanicApi.contains("prepareRendererBootstrapWindow")
			&& vulkanicApi.contains("createRendererDevice"),
			"only backend identity and explicit bootstrap may remain observable after the Rust handoff");
		String compatibilityDevice = Files.readString(Path.of("src/main/java/net/vulkanic/backends/vulkan/VulkanCompatibilityGpuDevice.java"));
		assertTrue(compatibilityDevice.contains("Java OpenGL compatibility device ")
			&& compatibilityDevice.contains("cannot execute rendering work. Port this callsite to explicit VulkanicGAL semantics."),
			"a live Rust presenter must reject, not execute, Java OpenGL compatibility rendering");
		assertTrue(compatibilityDevice.contains("withCompatibilityBackendForTeardown"),
			"the only post-handoff compatibility action must be bounded bootstrap teardown");
		assertTrue(compatibilityDevice.contains("ensureSelectedRustRoute")
			&& compatibilityDevice.contains("Selected Vulkan compatibility-device rendering is unavailable; Rust Vulkan semantic device is required"),
			"selected Vulkan must not execute through the compatibility device before Rust whole-frame admission");
		String nativeEncoder = Files.readString(Path.of("src/main/java/net/vulkanic/backends/vulkan/VulkanNativeCommandEncoder.java"));
		assertEquals(21, occurrences(nativeEncoder, "ensureJavaVulkanRenderingAvailable();"),
			"every Java Vulkan render-pass, transfer, mapping, fence, and direct-clear entry point must be fenced after Rust presentation takes ownership");
		assertTrue(nativeEncoder.contains("public void writeToBuffer(GpuBufferSlice slice, ByteBuffer data) {\n        this.ensureJavaVulkanRenderingAvailable();")
			&& nativeEncoder.contains("public void copyToBuffer(GpuBufferSlice source, GpuBufferSlice target) {\n        this.ensureJavaVulkanRenderingAvailable();")
			&& nativeEncoder.contains("public GpuFence createFence() {\n        this.ensureJavaVulkanRenderingAvailable();"),
			"stale Java Vulkan buffer transfers and fences must fail before touching native state");
		assertTrue(nativeEncoder.contains("private void checkOpen() {\n            VulkanNativeCommandEncoder.this.ensureJavaVulkanRenderingAvailable();"),
			"stale Java Vulkan render-pass objects must fail before issuing resource or draw commands");
		assertTrue(nativeEncoder.contains("MAX_RENDER_PASS_SAMPLERS = 128")
			&& nativeEncoder.contains("MAX_RENDER_PASS_UNIFORMS = 256")
			&& nativeEncoder.contains("MAX_RENDER_PASS_IRIS_PROGRAM_STATES = 128")
			&& nativeEncoder.contains("ensureBindingCapacity"),
			"transitional Java Vulkan render passes must keep sampler, uniform, and Iris-state resources bounded");
		assertTrue(nativeEncoder.contains("Java Vulkan render passes are unavailable while Rust owns whole-frame presentation"),
			"Java Vulkan render passes must fail closed instead of becoming a hidden fallback");
		assertTrue(nativeEncoder.contains("Selected Vulkan Java render passes are unavailable; Rust semantic rendering is not a fallback"),
			"selected Vulkan must not admit Java native render passes before whole-frame ownership is active");
		assertTrue(vulkanBackend.contains("Rust Vulkan route selected; skipping Java Vulkan and Iris GPU renderer startup."),
			"the selected Rust Vulkan route must not initialize Iris GPU state");
		assertFalse(vulkanBackend.contains("initializeVulkanCompatibilityHooks"),
			"the Rust whole-frame shell must not retain a Java/Iris GL initialization hook");
		assertTrue(vulkanBackend.contains("new VulkanWholeFrameSemanticGpuDevice()"),
			"whole-frame renderer startup must use a non-rendering semantic device instead of GlDevice");
		assertTrue(vulkanBackend.contains("Vulkan renderer device creation requires the Rust Vulkan whole-frame route to be selected")
			&& !vulkanBackend.contains("new net.blaze3d.opengl.GlDevice("),
			"selected Vulkan startup must choose the semantic device without constructing any Java OpenGL device");
		int bootstrapWindow = vulkanBackend.indexOf("public long prepareRendererBootstrapWindow(long mainWindowHandle)");
		assertTrue(bootstrapWindow >= 0
			&& vulkanBackend.indexOf("Vulkan renderer bootstrap requires the Rust Vulkan whole-frame route to be selected", bootstrapWindow) >= 0
			&& vulkanBackend.indexOf("GLFW.glfwWindowHint", bootstrapWindow) < 0,
			"selected Vulkan bootstrap must not create a hidden Java OpenGL window before Rust presentation admission");
		String semanticDevice = Files.readString(Path.of("src/main/java/net/vulkanic/backends/vulkan/VulkanWholeFrameSemanticGpuDevice.java"));
		String fontTexture = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/font/FontTexture.java"));
		String textureAtlas = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/texture/TextureAtlas.java"));
		String dynamicTexture = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/texture/DynamicTexture.java"));
		String textureManager = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/texture/TextureManager.java"));
		String rawImages = Files.readString(Path.of("src/main/java/net/vulkanic/gui/RustGalGuiRawImageAssets.java"));
		String worldTextCollector = Files.readString(Path.of("src/main/java/net/vulkanic/world/WorldTextSemanticCollector.java"));
		String playerGlyphProvider = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/font/PlayerGlyphProvider.java"));
		String skinDownloader = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/texture/SkinTextureDownloader.java"));
		String sodiumGpuSync = Files.readString(Path.of("src/main/java/net/sodium/fabric/SodiumGpuSyncHelper.java"));
		String shaderManager = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/ShaderManager.java"));
		String itemBlockRenderTypes = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/ItemBlockRenderTypes.java"));
		String skyRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/SkyRenderer.java"));
		String lightTexture = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/LightTexture.java"));
		String fogRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/fog/FogRenderer.java"));
		assertTrue(semanticDevice.contains("semantic-only; port this callsite to explicit VulkanicGAL semantics"),
			"the whole-frame bootstrap device must reject Java rendering rather than emulate it");
		assertFalse(semanticDevice.contains("org.lwjgl") || semanticDevice.contains("IrisRenderSystem"),
			"the whole-frame bootstrap device must own no native GL/Vulkan or Iris GPU state");
		assertTrue(fontTexture.contains("!net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()"),
			"whole-frame font-atlas updates must bypass Java texture upload");
		assertTrue(fontTexture.contains("uploaded through VulkanicGAL by the text collector"),
			"whole-frame font glyphs must retain a copied semantic atlas path");
		assertTrue(fontTexture.contains("semanticAtlasSnapshotCache")
			&& fontTexture.contains("cached.revision() == this.semanticAtlasRevision")
			&& fontTexture.contains("this.semanticAtlasSnapshotCache = null"),
			"semantic font atlas snapshots must be reused by revision without repeated full-atlas allocations");
		assertTrue(fontTexture.contains("MAX_SEMANTIC_ATLASES = 4096")
			&& fontTexture.contains("semantic font-atlas registry bound exceeded")
			&& fontTexture.contains("synchronized (SEMANTIC_ATLASES)"),
			"semantic font-atlas identities must have a bounded, synchronized registry");
		assertTrue(fontTexture.contains("? this.textureView")
			&& fontTexture.indexOf("VulkanicAPI.createTexture") > fontTexture.indexOf("!net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()"),
			"whole-frame font stitching must retain semantic metadata without allocating or tracking a Java atlas texture");
		assertTrue(textureAtlas.contains("boolean rustWholeFrame = net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()"),
			"whole-frame texture-atlas setup must make its non-rendering route explicit");
		assertTrue(textureAtlas.contains("|| net.vulkanic.VulkanicAPI.isVulkanBackendSelected()"),
			"selected Vulkan atlas animation must remain on the semantic ticker path");
		assertTrue(textureAtlas.contains("TextureAtlasSprite.Ticker ticker = rustWholeFrame ? null"),
			"whole-frame texture-atlas animation must not perform Java texture uploads");
		assertTrue(textureAtlas.contains("Selected Vulkan atlases are CPU-owned semantic snapshots")
			&& !textureAtlas.contains("IrisRenderSystem.generateMipmaps"),
			"selected Vulkan atlas reloads must not borrow Iris mipmap/GPU state");
		assertTrue(textureAtlas.contains("semanticSnapshotFrameKey")
			&& textureAtlas.contains("semanticFrameIndex() & 0xffffffffL")
			&& textureAtlas.contains("this.semanticSnapshotFrameKey == frameKey"),
			"Rust-owned atlas snapshots must refresh when an animated sprite changes frame, not only after resource reloads");
		assertTrue(dynamicTexture.contains("RustGalVulkanWholeFrameMode.enabled()")
			&& dynamicTexture.contains("RustGalGuiRawImageAssets.stageDynamicTexture(this)")
			&& dynamicTexture.contains("public void setClamp(boolean clamp)")
			&& dynamicTexture.contains("semanticClamp = clamp")
			&& dynamicTexture.contains("this.pixels = nativeImage;\n\t\tif (net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()")
			&& dynamicTexture.indexOf("stageDynamicTexture(this)") < dynamicTexture.indexOf("writeToTexture(this.texture, this.pixels)"),
			"whole-frame DynamicTexture updates and sampler intent must remain semantic before any Java upload path");
		String reloadableTexture = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/texture/ReloadableTexture.java"));
		assertTrue(reloadableTexture.contains("RustGalGuiRawImageAssets.stageNativeImage(this.resourceId, nativeImage)")
			&& reloadableTexture.indexOf("stageNativeImage(this.resourceId, nativeImage)")
			< reloadableTexture.indexOf("return;"),
			"whole-frame reloadable textures must stage decoded CPU pixels before suppressing Java GPU allocation");
		String cubeMapTexture = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/texture/CubeMapTexture.java"));
		assertTrue(cubeMapTexture.contains("stageNativeImage(")
			&& cubeMapTexture.contains("resourceLocation.withSuffix(SUFFIXES[k])")
			&& cubeMapTexture.contains("rustSemantic"),
			"Rust-owned cubemap reloads must stage every decoded face as CPU semantic data");
		assertTrue(textureManager.contains("RustGalGuiRawImageAssets.registerDynamicTexture(resourceLocation, dynamicTexture)")
			&& textureManager.contains("RustGalGuiRawImageAssets.unregisterDynamicTexture(resourceLocation, dynamicTexture)"),
			"DynamicTexture resource identities must be bound and retired by the texture manager, not by Java GPU handles");
		assertTrue(rawImages.contains("MemoryUtil.memByteBuffer(image.getPointer(), pixels.length).get(pixels)")
			&& rawImages.contains("stageCpuRgba8(ResourceLocation source, int width, int height, byte[] pixels)")
			&& rawImages.contains("pixels.length != Math.multiplyExact")
			&& rawImages.contains("MAX_STAGED_CUBEMAP_ASSETS = 4096")
			&& rawImages.contains("STAGED_CUBEMAP_ASSETS.size() >= MAX_STAGED_CUBEMAP_ASSETS")
			&& rawImages.contains("stage(asset)")
			&& rawImages.contains("DYNAMIC_TEXTURES.size() >= MAX_CACHED_ASSET_ENTRIES")
			&& rawImages.contains("STAGED_ASSETS")
			&& rawImages.contains("STAGED_ASSETS.get(asset.assetId()) == asset")
			&& rawImages.contains("STAGED_ASSETS.size() >= MAX_SEMANTIC_IDENTITIES")
			&& rawImages.contains("STAGED_ASSETS.clear()")
			&& rawImages.contains("DYNAMIC_TEXTURES")
			&& rawImages.contains("EARLY_VANILLA_CACHE.clear()")
			&& rawImages.contains("MAX_CACHED_ASSET_BYTES = 256L * 1024L * 1024L")
			&& rawImages.contains("cachedAssetBytesLocked()")
			&& rawImages.contains("Math.addExact(total, assetBytes(asset))")
			&& rawImages.contains("asset.pixelByteCount()")
			&& rawImages.contains("private int pixelByteCount()")
			&& (rawImages.contains("if (!cachePutLocked(EARLY_VANILLA_CACHE, source, asset)) return false;")
				|| rawImages.contains("if (!cachePutLocked(EARLY_VANILLA_CACHE, source, asset)) return null;"))
			&& rawImages.contains("if (hadEarly) EARLY_VANILLA_CACHE.put(source, previousEarly);")
			&& rawImages.contains("source.getPath().startsWith(\"textures/atlas/\")")
			&& rawImages.contains("resolveAtlas(source)"),
			"whole-frame dynamic images must copy bounded CPU pixels into the existing VulkanicGAL raw-image queue");
		assertTrue(queue.contains("MAX_PENDING_RAW_IMAGES = 4_096")
			&& queue.contains("pendingRawImages.size() >= MAX_PENDING_RAW_IMAGES"),
			"GUI raw-image staging must enforce a bounded coordinator-side identity queue");
		assertTrue(rawImages.contains("SemanticRawImageSnapshot")
			&& rawImages.contains("public static SemanticRawImageSnapshot semanticSnapshot")
			&& worldTextCollector.contains("semanticRawImageSnapshot(glyph.atlasIdentity())")
			&& worldTextCollector.contains("WorldTextImage::pixelByteCount")
			&& worldTextCollector.contains("private int pixelByteCount()")
			&& worldTextCollector.contains("WorldTextImage previous = images.get(assetId)")
			&& worldTextCollector.contains("matchesGeneration(imageGeneration, imageRevision"),
			"world text must consume dynamic skin images through the bounded semantic raw-image contract");
		int guiResourceLookup = rawImages.indexOf("getResourceManager().getResource(candidate)");
		int guiEarlyFallback = rawImages.indexOf("EARLY_VANILLA_CACHE.get(candidate)");
		assertTrue(guiResourceLookup >= 0 && guiEarlyFallback > guiResourceLookup,
			"GUI resource-pack images must resolve before early loading-overlay cache fallbacks");
		int cubeFace = rawImages.indexOf("private static Asset resolveCubeFace");
		int cubeResourceLookup = rawImages.indexOf("getResourceManager().getResource(source)", cubeFace);
		int cubeCacheLookup = rawImages.indexOf("Asset cached = CACHE.get(source)", cubeFace);
		assertTrue(cubeFace >= 0 && cubeResourceLookup > cubeFace && cubeCacheLookup > cubeResourceLookup,
			"GUI cubemap faces must resolve active resource-pack files before stale CPU cache fallbacks");
		int earlyStage = rawImages.indexOf("stageVanillaResource");
		int earlyCache = rawImages.indexOf("cachePutLocked(EARLY_VANILLA_CACHE", earlyStage);
		int normalCache = rawImages.indexOf("cachePutLocked(CACHE", earlyStage);
		assertTrue(earlyStage >= 0 && earlyCache > earlyStage && (normalCache < 0 || normalCache > rawImages.indexOf("stageNativeImage", earlyStage)),
			"pre-reload vanilla staging must not poison the authoritative resource-pack cache");
		assertTrue(playerGlyphProvider.contains("collectSemanticQuads")
			&& playerGlyphProvider.contains("textureIdentity")
			&& skinDownloader.contains("RustGalGuiRawImageAssets.registerDynamicTexture(texture.texturePath(), dynamicTexture)"),
			"player-skin glyphs must publish semantic quads and a copied CPU skin identity instead of remaining Java-only text");
		assertTrue(bridge.contains("MAX_RAW_IMAGE_BYTES = 64 * 1024 * 1024")
			&& rustFfi.contains("FFI_MAX_GUI_ASSET_BYTES: usize = 64 * 1024 * 1024"),
			"Java and Rust raw-image boundaries must share the explicit 64 MiB semantic-image limit");
		assertTrue(rustGuiFrontend.contains("if batches.is_empty()"));
		String rustGuiRenderer = Files.readString(Path.of("src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"));
		String rustGuiItemRenderer = Files.readString(Path.of("src/main/java/net/vulkanic/gui/RustGalGuiItemRenderer.java"));
		assertTrue(rustGuiRenderer.contains("MAX_GUI_ASSET_BYTES = 64 * 1024 * 1024")
			&& rustGuiRenderer.contains("MAX_GUI_DIAGNOSTIC_ENTRIES = 256")
			&& rustGuiRenderer.contains("MAX_TEXT_ATLAS_IDENTITIES = 4_096")
			&& rustGuiRenderer.contains("MAX_TEXT_ATLAS_GENERATIONS = 4_096")
			&& rustGuiRenderer.contains("TEXT_ROUTE_DIAGNOSTICS.size() >= MAX_GUI_DIAGNOSTIC_ENTRIES")
			&& rustGuiRenderer.contains("input.readNBytes(MAX_GUI_ASSET_BYTES + 1)")
			&& rustGuiRenderer.contains("GUI asset exceeds the \" + MAX_GUI_ASSET_BYTES + \" byte bound"),
			"whole-frame GUI asset copies must enforce the Rust-owned bounded payload contract before FFI");
		assertTrue(rustGuiItemRenderer.contains("MAX_DIAGNOSTIC_ENTRIES = 256")
			&& rustGuiItemRenderer.contains("DIAGNOSTICS.size() >= MAX_DIAGNOSTIC_ENTRIES"),
			"GUI item semantic diagnostics must remain bounded metadata");
		assertTrue(rustGuiRenderer.contains("admissibleAffineQuad(")
			&& rustGuiRenderer.contains("copied-blit-outside-affine-contract")
			&& rustGuiRenderer.indexOf("admissibleAffineQuad(") < rustGuiRenderer.indexOf("new VulkanicGalBridge.GuiAffineQuadRecord(", rustGuiRenderer.indexOf("tryEnqueueCopiedBlit")),
			"whole-frame copied blits must decline an unrepresentable affine contract before semantic submission");
		assertTrue(sodiumGpuSync.contains("RustGalVulkanWholeFrameMode.enabled()")
			&& sodiumGpuSync.indexOf("RustGalVulkanWholeFrameMode.enabled()") < sodiumGpuSync.indexOf("VulkanicAPI.getCommandContext()")
			&& sodiumGpuSync.contains("Rust owns the Vulkan queue, submission completion, and pacing."),
			"Sodium's Java GL-style fence queue must be unavailable when Rust owns whole-frame Vulkan completion");
		int wholeFrameShaderManagerGuard = shaderManager.indexOf("RustGalVulkanWholeFrameMode.enabled()");
		int javaPipelineCacheClear = shaderManager.indexOf("VulkanicAPI.clearBackendPipelineCache()");
		assertTrue(wholeFrameShaderManagerGuard >= 0 && javaPipelineCacheClear > wholeFrameShaderManagerGuard
			&& shaderManager.contains("Java render-pipeline compilation/cache ownership ends at the\n\t\t\t// whole-frame handoff."),
			"whole-frame resource reload must not compile or clear Java rendering pipelines after the Rust handoff");
		int irisMaterialMap = itemBlockRenderTypes.indexOf("WorldRenderingSettings.INSTANCE.getBlockTypeIds()");
		int terrainLayerGuard = itemBlockRenderTypes.lastIndexOf("!net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()", irisMaterialMap);
		assertTrue(irisMaterialMap > 0 && terrainLayerGuard >= 0 && terrainLayerGuard < irisMaterialMap
			&& itemBlockRenderTypes.contains("!net.vulkanic.VulkanicAPI.isVulkanBackendSelected()")
			&& itemBlockRenderTypes.contains("whole-frame terrain receives the ordinary semantic layer below"),
			"whole-frame terrain material classification must not read Iris's Java material map");
		int fabulousIrisConfig = levelRenderer.indexOf("net.irisshaders.iris.Iris.getIrisConfig().areShadersEnabled()");
		int fabulousWholeFrameGuard = levelRenderer.lastIndexOf("net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()", fabulousIrisConfig);
		assertTrue(fabulousIrisConfig > 0 && fabulousWholeFrameGuard >= 0 && fabulousWholeFrameGuard < fabulousIrisConfig
			&& levelRenderer.contains("Rust owns shader-pack admission for whole-frame Vulkan"),
			"whole-frame resource reload must not inspect Iris configuration while Rust owns shader-pack admission");
		int fabulousVulkanGuard = levelRenderer.indexOf("net.vulkanic.VulkanicAPI.isVulkanBackendSelected()", fabulousWholeFrameGuard);
		assertTrue(fabulousVulkanGuard > fabulousWholeFrameGuard && fabulousVulkanGuard < fabulousIrisConfig,
			"selected Vulkan resource reload must not query Iris configuration before Rust whole-frame activation");
		String clientPacketListener = Files.readString(Path.of("src/main/java/net/minecraft/client/multiplayer/ClientPacketListener.java"));
		int dhIrisCompatibility = clientPacketListener.indexOf("net.irisshaders.iris.Iris.loadedIncompatiblePack()");
		int dhWholeFrameGuard = clientPacketListener.lastIndexOf("if (!net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()", dhIrisCompatibility);
		assertTrue(dhIrisCompatibility > 0 && dhWholeFrameGuard >= 0 && dhWholeFrameGuard < dhIrisCompatibility
			&& clientPacketListener.contains("copied semantic configuration instead"),
			"whole-frame login must not invoke Iris/Distant Horizons Java compatibility state");
		String benchmark = Files.readString(Path.of("src/main/java/net/minecraft/client/dev/GraphicsFrameBenchmark.java"));
		assertTrue(benchmark.contains("!minecraft.getConnection().isAcceptingMessages()")
			&& benchmark.indexOf("!minecraft.getConnection().isAcceptingMessages()") < benchmark.indexOf("minecraft.setScreen(null);"),
			"benchmark cleanup must not mutate GUI state after connection teardown begins");
		assertTrue(benchmark.contains("scenarioRequiresProducerTraversal(PRIMED_TNT_SCENARIO)")
			&& benchmark.contains("routeObservedForProvenance(\"primed-tnt\")")
			&& benchmark.contains("scenarioRequiresProducerTraversal(ITEM_ENTITY_SCENARIO)")
			&& benchmark.contains("experienceOrbRouteDecisions()"),
			"shared gameplay readiness must require semantic traversal receipts for moving/entity producer families");
		int skyTextureInit = skyRenderer.indexOf("this.endSkyTexture = this.getTexture(END_SKY_LOCATION);");
		int skyWholeFrameGuard = skyRenderer.lastIndexOf("RustGalVulkanWholeFrameMode.enabled()", skyTextureInit);
		assertTrue(skyTextureInit > 0 && skyWholeFrameGuard >= 0 && skyWholeFrameGuard < skyTextureInit
			&& skyRenderer.contains("net.vulkanic.VulkanicAPI.isVulkanBackendSelected()")
			&& skyRenderer.contains("copies its celestial source assets"),
			"whole-frame sky reload must use the Rust-owned copied celestial assets instead of Java texture uploads");
		assertTrue(worldPrimitiveRenderer.contains("Mth.sin(state.sunAngle) < 0.0F")
			&& worldPrimitiveRenderer.contains("enqueueVanillaDarkDiscLocked(camera)"),
			"Rust sky semantics must preserve vanilla sunrise orientation and below-horizon dark-disc coverage");
		assertTrue(worldPrimitiveRenderer.contains("Rust Vulkan whole-frame sky requires copied semantic sun and moon texture assets")
			&& worldPrimitiveRenderer.contains("Rust Vulkan whole-frame End sky requires a copied semantic sky texture asset"),
			"Rust sky admission must fail closed when copied celestial assets are unavailable");
		int sodiumSetLevel = levelRenderer.indexOf("this.renderer.setLevel(clientLevel);");
		int sodiumSetLevelGuard = levelRenderer.lastIndexOf("if (!net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled())", sodiumSetLevel);
		int sodiumReload = levelRenderer.indexOf("this.renderer.reload();");
		int sodiumReloadGuard = levelRenderer.lastIndexOf("if (!net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled())", sodiumReload);
		assertTrue(sodiumSetLevel > 0 && sodiumSetLevelGuard >= 0 && sodiumSetLevelGuard < sodiumSetLevel
			&& sodiumReload > 0 && sodiumReloadGuard >= 0 && sodiumReloadGuard < sodiumReload,
			"whole-frame world changes must not initialize Sodium's Java GL render device");
		int sodiumCullSetup = levelRenderer.indexOf("this.renderer.setupTerrain(camera, viewport");
		int wholeFrameCullGuard = levelRenderer.lastIndexOf("if (net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled())", sodiumCullSetup);
		assertTrue(sodiumCullSetup > 0 && wholeFrameCullGuard >= 0 && wholeFrameCullGuard < sodiumCullSetup
			&& levelRenderer.contains("this.applyFrustum(frustum);"),
			"whole-frame terrain visibility must remain CPU semantic state without initializing Sodium's Java GL device");
		int sodiumSectionReady = levelRenderer.indexOf("this.renderer.isSectionReady(");
		int wholeFrameSectionGuard = levelRenderer.lastIndexOf("if (net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled())", sodiumSectionReady);
		assertTrue(sodiumSectionReady > 0 && wholeFrameSectionGuard >= 0 && wholeFrameSectionGuard < sodiumSectionReady
			&& levelRenderer.contains("section.getSectionMesh() != CompiledSectionMesh.UNCOMPILED"),
			"whole-frame entity extraction must use CPU section readiness instead of Sodium's Java GL manager");
		int sodiumBlockEntities = levelRenderer.indexOf("this.renderer.extractBlockEntities(camera, f, this.destructionProgress, levelRenderState);");
		int wholeFrameBlockEntityGuard = levelRenderer.lastIndexOf("if (net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled())", sodiumBlockEntities);
		assertTrue(sodiumBlockEntities > 0 && wholeFrameBlockEntityGuard >= 0 && wholeFrameBlockEntityGuard < sodiumBlockEntities
			&& levelRenderer.contains("compiled.getRenderableBlockEntities()")
			&& levelRenderer.contains("extractWholeFrameBlockEntity"),
			"whole-frame block entities must extract semantic state from CPU compiled sections without Sodium GL render lists");
		int sodiumRebuild = levelRenderer.indexOf("this.renderer.scheduleRebuildForChunk(x, y, z, important);");
		int wholeFrameDirtyGuard = levelRenderer.lastIndexOf("if (net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled())", sodiumRebuild);
		assertTrue(sodiumRebuild > 0 && wholeFrameDirtyGuard >= 0 && wholeFrameDirtyGuard < sodiumRebuild
			&& levelRenderer.contains("this.viewArea.setDirty(x, y, z, important)"),
			"packet-driven world updates must mark CPU semantic sections dirty before any Sodium GL rebuild path");
		assertTrue(lightTexture.contains("this.rustSemanticLightmapInputs = this.computeRustSemanticLightmapInputs(")
			&& lightTexture.contains("clientLevel, this.minecraft.player, f, false"),
			"whole-frame lightmap updates must publish gameplay scalars rather than a Java GPU texture");
		int wholeFrameLightmap = lightTexture.indexOf("RustGalVulkanWholeFrameMode.enabled()", lightTexture.indexOf("public void updateLightTexture"));
		int irisLightmapState = lightTexture.indexOf("CapturedRenderingState.INSTANCE.setDarknessLightFactor(0.0F)", wholeFrameLightmap);
		assertTrue(wholeFrameLightmap >= 0 && irisLightmapState > wholeFrameLightmap
			&& lightTexture.substring(wholeFrameLightmap, irisLightmapState).contains("return;"),
			"the whole-frame lightmap path must return before Java Iris and GPU lightmap work");
		assertTrue(lightTexture.contains("!net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()")
			&& lightTexture.contains("VulkanicAPI.createCommandEncoder().clearColorTexture"),
			"whole-frame lightmap construction must not submit a Java texture clear");
		assertTrue(fogRenderer.contains("return this.computeFogParameters(camera, i, bl, deltaTracker, f, clientLevel, false, true).parameters();"),
			"whole-frame fog extraction must use the semantic-only calculation path");
		assertTrue(fogRenderer.contains("collectFogParametersForRust"));
		assertTrue(fogRenderer.contains("computeFogParameters(camera, i, bl, deltaTracker, f, clientLevel, true, true)"),
			"the normal Java fog renderer must retain its legacy-Iris side effect explicitly");
		assertTrue(fogRenderer.contains("false, false);")
			&& fogRenderer.contains("legacy Iris/DH sentinel range"),
			"Rust fog extraction must omit both Java/Iris and unavailable-DH cancellation side effects");
		assertTrue(fogRenderer.contains("if (updateLegacyIrisFogState && camera.getFluidInCamera()"),
			"the copied whole-frame fog record must not update Iris runtime state");
		assertTrue(vulkanicApi.contains("RustGalVulkanWholeFrameMode.enabled()"));
		assertTrue(vulkanicApi.indexOf("RustGalVulkanWholeFrameMode.enabled()") < vulkanicApi.indexOf("if (configuredValue == null)"));
		assertTrue(rustBackends.contains("create_native_windowed_vulkan_backend"));
		assertTrue(rustVulkan.contains("struct NativeWindowSurface"));
		assertTrue(rustVulkan.contains("WINDOW_PLATFORM_X11"));
		assertTrue(rustVulkan.contains("WINDOW_PLATFORM_WAYLAND"));
		assertTrue(rustGuiFrontend.contains("if batches.is_empty()"));
		assertTrue(rustGuiFrontend.contains("gal.pass_target_color_format(frame_target)?"),
			"GUI pass creation must derive its format from the explicit GAL frame target");
		assertFalse(rustFfi.contains("ash::"));
	}

	@Test
	void rustGalGuiIntegrationHasCleanBridgeSchedulerAndGuiDomainBoundary() throws Exception {
		String bridge = Files.readString(Path.of("src/main/java/net/vulkanic/bridge/VulkanicGalBridge.java"));
		String scheduler = Files.readString(Path.of("src/main/java/net/vulkanic/bridge/RustGalFrameScheduler.java"));
		String guiRenderer = Files.readString(Path.of("src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"));
		String frameCoordinator = Files.readString(Path.of("src/main/java/net/vulkanic/gui/RustGalFrameCoordinator.java"));
		String guiElement = Files.readString(Path.of("src/main/java/net/vulkanic/gui/RustGalGuiElementRenderState.java"));
		String rustGuiFrontend = Files.readString(Path.of("src/main/rust/render/vulkanic/gui_frontend.rs"));
		String gui = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/Gui.java"));
		String bossOverlay = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/components/BossHealthOverlay.java"));
		String experienceBar = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/contextualbar/ExperienceBarRenderer.java"));

		assertFalse(Files.exists(Path.of("src/main/java/net/vulkanic/gui/GuiSpriteAtlas.java")));
		assertFalse(Files.exists(Path.of("src/main/java/net/vulkanic/gui/GuiResourceCache.java")));
		assertFalse(Files.exists(Path.of("src/main/java/net/vulkanic/gui/GuiBatchBuilder.java")));
		assertFalse(Files.exists(Path.of("src/main/java/net/vulkanic/gui/GuiPipelineLibrary.java")));
		assertFalse(bridge.contains("guiAlphaPipeline"));
		assertFalse(bridge.contains("guiInvertPipeline"));
		assertFalse(bridge.contains("CROSSHAIR_PRODUCER"));
		assertFalse(bridge.contains("HOTBAR_SELECTION_PRODUCER"));
		assertFalse(bridge.contains("ARMOR_ICON_PRODUCER"));
		assertFalse(scheduler.contains("GuiSprite"));
		assertFalse(scheduler.contains("VulkanicGalBridge"));
		assertFalse(scheduler.contains("shader"));
		assertFalse(scheduler.contains("atlas"));
		assertFalse(scheduler.contains("pipeline"));
		assertTrue(scheduler.contains("MAX_PENDING_BATCHES = 65_536")
			&& scheduler.contains("pending semantic batch bound exceeded"));
		assertFalse(guiRenderer.contains("RustGalFrameScheduler<VulkanicGalBridge.GuiSpriteRecord>"));
		assertTrue(frameCoordinator.contains("RustGalFrameScheduler<QueuedGuiRequest>"));
		assertTrue(frameCoordinator.contains("QueuedGuiRequest"));
		assertFalse(guiRenderer.contains("record CachedResources"));
		assertFalse(guiRenderer.contains("record TextureAtlas"));
		assertFalse(guiRenderer.contains("record FrameSpriteBatch"));
		assertFalse(guiRenderer.contains("class FrameSpriteBatchBuilder"));
		assertFalse(guiRenderer.contains("GuiBatchBuilder.packCompatibleSpriteBatches"));
		assertFalse(guiRenderer.contains("GuiResourceCache resources"));
		assertFalse(guiRenderer.contains("bridge.submitGuiFrame"));
		assertTrue(frameCoordinator.contains("bridge.submitGuiFrame"));
		assertFalse(guiRenderer.contains("createBorrowedOpenGl"));
		assertFalse(guiRenderer.contains("createWindowedVulkan"));
		assertFalse(guiRenderer.contains("acquireFrame"));
		assertFalse(guiRenderer.contains("presentFrame"));
		assertFalse(guiRenderer.contains("submitWholeFrame"));
		assertFalse(guiRenderer.contains("submitWorldPrimitives"));
		assertFalse(guiRenderer.contains("executeWorldPrimitiveFrame"));
		assertFalse(guiRenderer.contains("executeWholeFrameVulkan"));
		assertFalse(guiRenderer.contains("currentAuditMetricsLine"));
		assertFalse(guiRenderer.contains("MetricsSnapshot"));
		assertFalse(guiRenderer.contains("RustGalWorldPrimitiveRenderer"));
		assertTrue(frameCoordinator.contains("createBorrowedOpenGl"));
		assertTrue(frameCoordinator.contains("createWindowedVulkan"));
		assertTrue(frameCoordinator.contains("acquireFrame"));
		assertTrue(frameCoordinator.contains("presentFrame"));
		assertTrue(frameCoordinator.contains("submitWholeFrame"));
		assertTrue(frameCoordinator.contains("submitWorldPrimitives"));
		assertTrue(frameCoordinator.contains("executeWorldPrimitiveFrame"));
		assertTrue(frameCoordinator.contains("executeWholeFrameVulkan"));
		assertTrue(frameCoordinator.contains("currentAuditMetricsLine"));
		assertTrue(frameCoordinator.contains("MetricsSnapshot"));
		assertTrue(frameCoordinator.contains("RustGalWorldPrimitiveRenderer"));
		assertTrue(rustGuiFrontend.contains("struct TextureAtlas"));
		assertTrue(rustGuiFrontend.contains("struct GuiResources"));
		assertTrue(rustGuiFrontend.contains("fn create_resources"));
		assertTrue(rustGuiFrontend.contains("fn build_atlas"));
		assertFalse(guiRenderer.contains("layout(std140) uniform GuiSpriteBatch"));
		assertTrue(rustGuiFrontend.contains("layout(std140) uniform GuiSpriteBatch"));
		assertTrue(guiElement.contains("RustGalFrameScheduler.Token token"));
		assertFalse(gui.contains("VulkanicGalBridge"));
		assertFalse(gui.contains("MemorySegment"));
		assertFalse(gui.contains("HANDLE_"));
		assertFalse(bossOverlay.contains("VulkanicGalBridge"));
		assertFalse(experienceBar.contains("VulkanicGalBridge"));
		assertTrue(gui.contains("RustGalGuiRenderer.enqueueCrosshair"));
		assertTrue(gui.contains("RustGalGuiRenderer.enqueueArmorIcons"));
		assertTrue(gui.contains("RustGalGuiRenderer.enqueuePlayerHearts"));
		assertTrue(gui.contains("RustGalGuiRenderer.enqueueAbsorptionHearts"));
		assertTrue(gui.contains("RustGalGuiRenderer.enqueueHungerIcons"));
		assertTrue(gui.contains("RustGalGuiRenderer.enqueueAirBubbles"));
		assertTrue(gui.contains("RustGalGuiRenderer.enqueueMountHearts"));
		assertTrue(gui.contains("List<PlayerHeartRequest> rustPlayerHearts"));
		assertTrue(gui.contains("List<AbsorptionHeartRequest> rustAbsorptionHearts"));
		assertTrue(gui.contains("List<HungerIconRequest> rustHungerIcons"));
		assertTrue(gui.contains("List<AirBubbleRequest> rustAirBubbles"));
		assertTrue(gui.contains("List<MountHeartRequest> rustMountHearts"));
		assertTrue(gui.contains("rustHeartVariant(heartType)"));
		assertTrue(gui.contains("rustAbsorptionHeartVariant(heartType)"));
		assertTrue(gui.contains("diagnosticPlayerHealth(player.getHealth())"));
		assertTrue(gui.contains("diagnosticPlayerMaxHealth((float)player.getAttributeValue(Attributes.MAX_HEALTH))"));
		assertTrue(gui.contains("diagnosticPlayerAbsorption(player.getAbsorptionAmount())"));
		assertTrue(gui.contains("diagnosticFoodLevel(foodData.getFoodLevel())"));
		assertFalse(guiRenderer.contains("public record PlayerHeartRequest"));
		assertFalse(guiRenderer.contains("public enum PlayerHeartVariant"));
		assertFalse(guiRenderer.contains("public enum PlayerHeartState"));
		assertFalse(guiRenderer.contains("public enum ArmorIconState"));
		assertTrue(Files.exists(Path.of("src/main/java/net/vulkanic/gui/PlayerHeartRequest.java")));
		assertTrue(Files.exists(Path.of("src/main/java/net/vulkanic/gui/AbsorptionHeartRequest.java")));
		assertTrue(Files.exists(Path.of("src/main/java/net/vulkanic/gui/MountHeartRequest.java")));
		assertTrue(Files.exists(Path.of("src/main/java/net/vulkanic/gui/GuiHeartState.java")));
		assertTrue(Files.exists(Path.of("src/main/java/net/vulkanic/gui/ArmorIconState.java")));
		assertTrue(bossOverlay.contains("RustGalGuiRenderer.enqueueBossBar"));
		assertTrue(experienceBar.contains("RustGalGuiRenderer.enqueueExperienceBar"));
		assertFalse(guiRenderer.contains("GL_TEXTURE"));
		assertFalse(guiRenderer.contains("VK_"));
	}

	@Test
	void wholeFrameClientLevelShadingDoesNotReadIrisRuntimeState() throws Exception {
		String clientLevel = Files.readString(Path.of("src/main/java/net/minecraft/client/multiplayer/ClientLevel.java"));
		int irisRead = clientLevel.indexOf("WorldRenderingSettings.INSTANCE.shouldDisableDirectionalShading()");
		int guard = clientLevel.lastIndexOf("!net.vulkanic.VulkanicAPI.isVulkanBackendSelected()", irisRead);
		assertTrue(irisRead >= 0 && guard >= 0 && guard < irisRead,
			"semantic terrain shading must not read Iris material settings after Vulkan selection");
		String levelRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/LevelRenderer.java"));
		int cullIrisRead = levelRenderer.indexOf("Iris.getPipelineManager().getPipelineNullable()");
		int cullRustGuard = levelRenderer.lastIndexOf("RustGalVulkanWholeFrameMode.enabled()", cullIrisRead);
		assertTrue(cullIrisRead >= 0 && cullRustGuard >= 0 && cullRustGuard < cullIrisRead,
			"Rust whole-frame terrain culling must not query Iris pipeline state");
		int entityIrisRead = levelRenderer.indexOf("Iris.getPipelineManager().getPipelineNullable()", cullIrisRead + 1);
		int entityRustGuard = levelRenderer.lastIndexOf("RustGalVulkanWholeFrameMode.enabled()", entityIrisRead);
		assertTrue(entityIrisRead >= 0 && entityRustGuard >= 0 && entityRustGuard < entityIrisRead,
			"Rust whole-frame entity extraction must not query Iris skip-rendering state");
		int groupIrisRead = levelRenderer.indexOf("Iris.getPipelineManager().getPipelineNullable()", entityIrisRead + 1);
		int groupRustGuard = levelRenderer.lastIndexOf("RustGalVulkanWholeFrameMode.enabled()", groupIrisRead);
		assertTrue(groupIrisRead >= 0 && groupRustGuard >= 0 && groupRustGuard < groupIrisRead,
			"Rust whole-frame terrain-group dispatch must not query Iris skip-rendering state");
	}

	@Test
	void wholeFrameLevelFeatureWrappersRejectBeforeIrisPolicyReads() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/LevelRenderer.java"));
		assertTrue(source.contains("ensureJavaIrisFeaturePassAvailable(\"main feature pass\")")
			&& source.contains("ensureJavaIrisFeaturePassAvailable(\"particle submission\")")
			&& source.contains("ensureJavaIrisFeaturePassAvailable(\"particle feature pass\")")
			&& source.contains("ensureJavaIrisFeaturePassAvailable(\"sky pass\")")
			&& source.contains("ensureJavaIrisFeaturePassAvailable(\"translucent pass\")"));
		int helper = source.indexOf("private static void ensureJavaIrisFeaturePassAvailable");
		assertTrue(helper >= 0 && source.indexOf("RustGalVulkanWholeFrameMode.enabled()", helper) > helper);
	}

	@Test
    void wholeFrameTerrainBuildsSkipJavaGpuUploads() throws Exception {
		String renderSections = Files.readString(Path.of("src/main/java/net/sodium/client/render/chunk/RenderSectionManager.java"));
		String sectionCompiler = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/chunk/SectionCompiler.java"));
		String wholeFrameSource = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWholeFrameTerrainSource.java"));
		int upload = renderSections.indexOf("this.regions.uploadResults(");
		int uploadGuard = renderSections.lastIndexOf("if (!rustWholeFrame)", upload);
		assertTrue(upload >= 0 && uploadGuard >= 0 && uploadGuard < upload,
			"Rust whole-frame terrain must not upload chunk meshes through Java RenderDevice");
		assertTrue(wholeFrameSource.contains("RustGalTerrainRenderer.acceptWholeFrameChunkBuildOutput(output)"),
			"the independent Rust whole-frame producer must admit compact CPU mesh semantics explicitly");
		assertTrue(sectionCompiler.contains("rustWholeFrameTerrain")
			&& sectionCompiler.contains("!rustWholeFrameTerrain && !fluidState.isEmpty()")
			&& sectionCompiler.contains("!rustWholeFrameTerrain && blockState.getRenderShape() == RenderShape.MODEL"),
			"legacy section compilation must retain visibility bookkeeping without building a discarded Java terrain/fluid mesh");
		String pool = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/SectionBufferBuilderPool.java"));
		assertTrue(pool.contains("int k = rustWholeFrameVulkan ? 1 : Math.max(1, Math.min(i, j));"),
			"Rust whole-frame terrain must not reserve the heap-sized legacy Java staging pool");
		assertTrue(pool.contains("new SectionBufferBuilderPack(rustWholeFrameVulkan)"),
			"Rust whole-frame terrain must construct its bookkeeping pack without Java staging capacities");
		String buffers = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/RenderBuffers.java"));
		String pack = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/SectionBufferBuilderPack.java"));
		int hooks = buffers.indexOf("HookRegistry.getRenderBuffersHooks()");
		int rustGuard = buffers.lastIndexOf("if (!rustWholeFrameVulkan())", hooks);
		assertTrue(hooks >= 0 && rustGuard >= 0 && rustGuard < hooks,
			"Rust whole-frame terrain must not admit extension-provided Java staging pools");
		assertTrue(buffers.contains("new SectionBufferBuilderPack(rustWholeFrameVulkan())")
			&& buffers.contains("new ByteBufferBuilder(rustWholeFrameVulkan() ? 0 : 786432)")
			&& pack.contains("new ByteBufferBuilder(this.minimal ? 0 : chunkSectionLayer.bufferSize())"),
			"Rust whole-frame setup must not eagerly allocate the fixed Java terrain buffer pack");
	}

	@Test
	void wholeFrameCannotEnterJavaBufferSourceRendering() throws Exception {
		String buffers = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/MultiBufferSource.java"));
		assertTrue(buffers.contains("Java Vulkan buffer-source rendering is unavailable on selected Vulkan")
			|| buffers.contains("Java Vulkan buffer-source rendering is unavailable while Rust owns whole-frame presentation"),
			"Java buffer-source acquisition must fail closed after Rust presentation ownership transfers");
		int retire = buffers.indexOf("private void endBatch(RenderType renderType, BufferBuilder bufferBuilder)");
		int draw = buffers.indexOf("renderType.draw(meshData)", retire);
		assertTrue(retire >= 0 && buffers.indexOf("meshData.close()", retire) > retire && draw > retire,
			"outstanding Java meshes must be retired before any draw call during Rust handoff");
	}

	@Test
	void wholeFrameSodiumOptionsDoNotProbeJavaRenderDevice() throws Exception {
		String options = Files.readString(Path.of("src/main/java/net/sodium/client/gui/SodiumGameOptionPages.java"));
		int probe = options.indexOf("MappedStagingBuffer.isSupported(RenderDevice.instance())");
		int guard = options.lastIndexOf("!net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()", probe);
		assertTrue(probe >= 0 && guard >= 0 && guard < probe,
			"Rust whole-frame menu setup must not probe Java RenderDevice capabilities");
	}

	@Test
	void wholeFrameSodiumWorldRendererCannotCreateJavaGraph() throws Exception {
		String sodium = Files.readString(Path.of("src/main/java/net/sodium/client/render/SodiumWorldRenderer.java"));
		int setLevel = sodium.indexOf("public void setLevel(ClientLevel level)");
		int setLevelGuard = sodium.indexOf("RustGalVulkanWholeFrameMode.enabled()", setLevel);
		int reload = sodium.indexOf("public void reload()");
		int reloadGuard = sodium.indexOf("RustGalVulkanWholeFrameMode.enabled()", reload);
		assertTrue(setLevel >= 0 && setLevelGuard > setLevel && reload >= 0 && reloadGuard > reload,
			"Rust whole-frame terrain must not construct or reload Sodium's Java GPU graph");
	}

	@Test
	void wholeFrameSodiumWorldRendererCannotDrawJavaTerrain() throws Exception {
		String sodium = Files.readString(Path.of("src/main/java/net/sodium/client/render/SodiumWorldRenderer.java"));
		int draw = sodium.indexOf("public void drawChunkLayer(");
		int guard = sodium.indexOf("RustGalVulkanWholeFrameMode.enabled()", draw);
		int drawCall = sodium.indexOf("renderSectionManager.renderLayer", draw);
		assertTrue(draw >= 0 && guard > draw && drawCall > guard,
			"Rust whole-frame terrain must fail closed before Sodium can issue Java draw passes");
		String shaderRenderer = Files.readString(Path.of("src/main/java/net/sodium/client/render/chunk/ShaderChunkRenderer.java"));
		assertTrue(shaderRenderer.contains("Selected Vulkan Sodium shader chunk rendering is unavailable; Rust terrain semantics are required"),
			"selected Vulkan must not enter Sodium's Java shader chunk renderer during startup races");
	}

	@Test
	void wholeFrameSodiumSectionManagerCannotOpenJavaTerrainPass() throws Exception {
		String sections = Files.readString(Path.of("src/main/java/net/sodium/client/render/chunk/RenderSectionManager.java"));
		int render = sections.indexOf("public void renderLayer(");
		int guard = sections.indexOf("RustGalVulkanWholeFrameMode.enabled()", render);
		int commandList = sections.indexOf("createCommandList()", render);
		assertTrue(render >= 0 && guard > render && commandList > guard,
			"Rust whole-frame terrain must reject direct Sodium section-manager draws before opening a Java command list");
	}

	@Test
	void wholeFrameGameRendererCannotReenterLegacyLevelRendering() throws Exception {
		String gameRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
		int renderLevel = gameRenderer.indexOf("public void renderLevel(DeltaTracker deltaTracker)");
		int guard = gameRenderer.indexOf("RustGalVulkanWholeFrameMode.enabled()", renderLevel);
		int legacyLevelCall = gameRenderer.indexOf("levelRenderer\n\t\t\t.renderLevel", renderLevel);
		assertTrue(renderLevel >= 0 && guard > renderLevel && legacyLevelCall > guard,
			"Rust whole-frame mode must reject direct legacy GameRenderer level rendering before Java GPU setup");
		int constructor = gameRenderer.indexOf("public GameRenderer(");
		int firstProjectionBuffer = gameRenderer.indexOf("new PerspectiveProjectionMatrixBuffer", constructor);
		int selectedConstructorGuard = gameRenderer.indexOf("RustGalVulkanWholeFrameMode.enabled()", constructor);
		assertTrue(constructor >= 0 && selectedConstructorGuard > constructor && firstProjectionBuffer > selectedConstructorGuard,
			"selected Vulkan must avoid allocating Java projection UBOs during GameRenderer construction");
	}

	@Test
	void wholeFrameFeatureDispatcherCannotReopenJavaSubmissions() throws Exception {
		String dispatcher = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/feature/FeatureRenderDispatcher.java"));
		int all = dispatcher.indexOf("public void renderAllFeatures()");
		int allGuard = dispatcher.indexOf("RustGalVulkanWholeFrameMode.enabled()", all);
		int allLoop = dispatcher.indexOf("getSubmitsPerOrder()", all);
		int blocks = dispatcher.indexOf("public void renderBlockFeaturesOnly()");
		int blockLoop = dispatcher.indexOf("getSubmitsPerOrder()", blocks);
		assertTrue(all >= 0 && allGuard > all && allLoop > allGuard
			&& blocks >= 0 && blockLoop > blocks
			&& dispatcher.indexOf("WorldRenderRoutePolicy.currentEntityShadowRoute()", blocks) > blockLoop,
			"Rust whole-frame feature dispatch must consume routed semantic features instead of aborting before submission");
		assertTrue(dispatcher.contains("private static boolean rustPresenterActive()")
			&& occurrences(dispatcher, "if (!rustPresenterActive()") >= 4,
			"every Java feature renderer branch must use the shared Rust presenter ownership predicate");
	}

	@Test
	void levelDebugSemanticCollectorsHonorPresenterShellOwnership() throws Exception {
		String level = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/LevelRenderer.java"));
		assertTrue(occurrences(level,
			"if (!VulkanicAPI.isVulkanBackendSelected() && !net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()) return;") >= 11,
			"all Rust debug semantic collectors must remain active throughout presenter handoff");
		assertEquals(0, occurrences(level,
			"if (!VulkanicAPI.isVulkanBackendSelected() || !net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()) return;"),
			"debug semantic collectors must not require both ownership signals simultaneously");
	}

	@Test
	void wholeFrameFeatureDispatcherDoesNotRotateJavaParticleBuffers() throws Exception {
		String dispatcher = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/feature/FeatureRenderDispatcher.java"));
		int endFrame = dispatcher.indexOf("public void endFrame()");
		int guard = dispatcher.indexOf("RustGalVulkanWholeFrameMode.enabled()", endFrame);
		int rotate = dispatcher.indexOf("particleFeatureRenderer.endFrame()", endFrame);
		assertTrue(endFrame >= 0 && guard > endFrame && rotate > guard,
			"Rust whole-frame cleanup must not rotate Java particle GPU buffers");
	}

	@Test
	void wholeFrameHandsCannotReopenJavaFirstPersonRenderer() throws Exception {
		String hands = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/ItemInHandRenderer.java"));
		int render = hands.indexOf("public void renderHandsWithItems(");
		int guard = hands.indexOf("RustGalVulkanWholeFrameMode.enabled()", render);
		int arm = hands.indexOf("renderArmWithItem", render);
		assertTrue(render >= 0 && guard > render && arm > guard,
			"Rust whole-frame first-person rendering must reject the Java hand entrypoint before arm submission");
	}

	@Test
	void wholeFrameParticleRendererDoesNotRotateJavaBuffersDirectly() throws Exception {
		String particles = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/feature/ParticleFeatureRenderer.java"));
		int endFrame = particles.indexOf("public void endFrame()");
		int guard = particles.indexOf("RustGalVulkanWholeFrameMode.enabled()", endFrame);
		int rotate = particles.indexOf("particleBufferCache.rotate()", endFrame);
		assertTrue(endFrame >= 0 && guard > endFrame && rotate > guard,
			"Rust whole-frame particle lifecycle must reject direct Java buffer rotation");
	}

	@Test
	void wholeFrameHitboxCollectorCannotSilentlyDropDisabledRoute() throws Exception {
		String dispatcher = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/feature/FeatureRenderDispatcher.java"));
		int collector = dispatcher.indexOf("public void collectRustHitboxSemantics(");
		int activeGuard = dispatcher.indexOf("RustGalVulkanWholeFrameMode.enabled()", collector);
		int unavailable = dispatcher.indexOf("Rust whole-frame debug-hitbox route is unavailable while Rust owns presentation", collector);
		assertTrue(collector >= 0 && activeGuard > collector && unavailable > activeGuard,
			"direct Rust hitbox collection must reject disabled work while Rust owns presentation");
	}

	@Test
	void activeRustPresentationCannotMisrouteParticlesToJava() throws Exception {
		String particles = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/feature/ParticleFeatureRenderer.java"));
		int render = particles.indexOf("public void render(SubmitNodeCollection");
		int activeGuard = particles.indexOf("RustGalVulkanWholeFrameMode.enabled()", render);
		int javaPass = particles.indexOf("createRenderPass", render);
		assertTrue(render >= 0 && activeGuard > render && javaPass > activeGuard,
			"Rust presentation ownership must reject a misclassified particle route before Java pass creation");
	}

	@Test
	void wholeFrameCannotUseJavaTracyCaptureOrUpload() throws Exception {
		String tracy = Files.readString(Path.of("src/main/java/net/blaze3d/TracyFrameCapture.java"));
		int capture = tracy.indexOf("public void capture(");
		int captureGuard = tracy.indexOf("RustGalVulkanWholeFrameMode.enabled()", capture);
		int capturePass = tracy.indexOf("createRenderPass", capture);
		int upload = tracy.indexOf("public void upload()");
		int uploadGuard = tracy.indexOf("RustGalVulkanWholeFrameMode.enabled()", upload);
		int uploadEncoder = tracy.indexOf("createCommandEncoder", upload);
		assertTrue(capture >= 0 && captureGuard > capture && capturePass > captureGuard
			&& upload >= 0 && uploadGuard > upload && uploadEncoder > uploadGuard,
			"Rust whole-frame diagnostics must not reopen Java capture passes or encoders");
	}

	@Test
	void wholeFrameChunkSectionDrawCannotOpenJavaTerrainPass() throws Exception {
		String sections = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/chunk/ChunkSectionsToRender.java"));
		assertTrue(sections.contains("VulkanicAPI.isVulkanBackendSelected()")
			&& sections.contains("Selected Vulkan chunk-section rendering is unavailable"),
			"selected Vulkan terrain must fail closed before Java chunk-section rendering");
		assertTrue(sections.contains("Java chunk-section rendering is unavailable while Rust owns whole-frame presentation"),
			"vanilla chunk-section draw entry must fail closed after Rust presentation ownership transfers");
	}

	@Test
	void wholeFrameDistantHorizonsBuffersDoNotTouchIrisTracking() throws Exception {
		String buffers = Files.readString(Path.of("src/main/java/com/seibel/distanthorizons/core/render/glObject/buffer/GLBuffer.java"));
		String boxes = Files.readString(Path.of("src/main/java/com/seibel/distanthorizons/core/render/renderer/generic/RenderableBoxGroup.java"));
		assertTrue(buffers.contains("!net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()")
			&& boxes.contains("!net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()")
			&& boxes.contains("isVulkanBackendSelected()")
			&& boxes.contains("Java Distant Horizons renderable-box buffers are unavailable"),
			"Distant Horizons must not allocate Java renderable-box buffers in the selected Rust Vulkan route");
	}

	@Test
	void selectedVulkanDistantHorizonsProxyDoesNotBorrowOpenGlCapabilities() throws Exception {
		String proxy = Files.readString(Path.of("src/main/java/com/seibel/distanthorizons/core/render/glObject/GLProxy.java"));
		assertTrue(proxy.contains("vulkanBackend ? null : VulkanicAPI.getGLCapabilities()"));
		assertTrue(proxy.contains("!vulkanBackend && VulkanicAPI.getNamedBufferDataPointer()"));
		assertTrue(proxy.contains("!vulkanBackend && VulkanicAPI.getBufferStoragePointer()"));
		assertTrue(proxy.contains("!vulkanBackend && VulkanicAPI.getBindVertexBufferPointer()"),
			"selected Rust Vulkan DH capability setup must not probe Java OpenGL function state");
		assertTrue(proxy.contains("Java Distant Horizons upload-task draining is unavailable while Rust owns whole-frame presentation"),
			"selected Rust Vulkan must not drain queued Java DH upload work");
		String color = Files.readString(Path.of("src/main/java/com/seibel/distanthorizons/core/render/glObject/texture/DhColorTexture.java"));
		String depth = Files.readString(Path.of("src/main/java/com/seibel/distanthorizons/core/render/glObject/texture/DHDepthTexture.java"));
		String ssao = Files.readString(Path.of("src/main/java/com/seibel/distanthorizons/core/render/renderer/SSAORenderer.java"));
		assertTrue(color.contains("isVulkanBackendSelected()") && depth.contains("isVulkanBackendSelected()")
			&& ssao.contains("isVulkanBackendSelected()"),
			"selected Vulkan must fence DH texture and SSAO Java GPU paths before native initialization");
	}

	@Test
	void distantHorizonsSemanticPreflightFlushesBeforePresenterActivation() throws Exception {
		String coordinator = Files.readString(Path.of("src/main/java/net/vulkanic/gui/RustGalFrameCoordinator.java"));
		int helper = coordinator.indexOf("private static void flushPendingWorldLodAssetsLocked()");
		int guard = coordinator.indexOf("RustGalGuiRenderer.isWholeFrameVulkanEnabled()", helper);
		int flush = coordinator.indexOf("DistantHorizonsSemanticCollector.flushPendingAssets(bridge)", helper);
		assertTrue(helper >= 0 && guard > helper && flush > guard,
			"Distant Horizons semantic asset preflight must publish through Rust before presenter activation");
		assertTrue(coordinator.contains("flushPendingWorldLodAssetsForSemanticPreflight()"),
			"Distant Horizons route admission must expose an explicit semantic preflight asset flush");
	}

	@Test
	void wholeFrameGuiCannotAllocateJavaMappableRingBuffers() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/MappableRingBuffer.java"));
		int constructor = source.indexOf("public MappableRingBuffer(");
		int constructorGuard = source.indexOf("RustGalVulkanWholeFrameMode.enabled()", constructor);
		int rotate = source.indexOf("public void rotate()");
		int rotateGuard = source.indexOf("RustGalVulkanWholeFrameMode.enabled()", rotate);
		assertTrue(constructor >= 0 && constructorGuard > constructor,
			"Rust whole-frame GUI must not allocate Java mappable ring buffers");
		assertTrue(rotate >= 0 && rotateGuard > rotate,
			"Rust whole-frame GUI must not submit Java ring-buffer fences");
	}

	@Test
	void wholeFrameDebugOverlayDoesNotAllocateJavaCrosshairBuffer() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/components/DebugScreenOverlay.java"));
		int constructor = source.indexOf("public DebugScreenOverlay(");
		int guard = source.indexOf("RustGalVulkanWholeFrameMode.enabled()", constructor);
		int nullBuffer = source.indexOf("this.crosshairBuffer = null", guard);
		int createBuffer = source.indexOf("VulkanicAPI.createBuffer", constructor);
		assertTrue(constructor >= 0 && guard > constructor && nullBuffer > guard,
			"whole-frame debug overlay must leave its Java crosshair buffer absent");
		assertTrue(createBuffer < 0 || createBuffer > nullBuffer,
			"whole-frame debug overlay must not allocate the Java crosshair buffer before its guard");
	}

	@Test
	void selectedVulkanDebugOverlayCannotAllocateOrDrawJavaCrosshair() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/components/DebugScreenOverlay.java"));
		assertTrue(source.contains("net.vulkanic.VulkanicAPI.isVulkanBackendSelected()")
			&& source.contains("Java 3D crosshair rendering is unavailable on selected Vulkan"),
			"selected Vulkan debug overlay must fail closed before Java crosshair buffers or draws");
	}

	@Test
	void wholeFrameSodiumAndIrisCannotConstructCompatibilityBuffers() throws Exception {
		String sodium = Files.readString(Path.of("src/main/java/net/sodium/client/gl/buffer/GlBuffer.java"));
		String iris = Files.readString(Path.of("src/main/java/net/irisshaders/iris/gl/buffer/ShaderStorageBuffer.java"));
		String irisRenderSystem = Files.readString(Path.of("src/main/java/net/irisshaders/iris/gl/IrisRenderSystem.java"));
		assertTrue(sodium.contains("Java Sodium buffers are unavailable while Rust owns whole-frame presentation"));
		assertTrue(iris.contains("Java Iris shader-storage buffers are unavailable on selected Vulkan")
			|| iris.contains("Java Iris shader-storage buffers are unavailable while Rust owns whole-frame presentation"),
			"compatibility buffer constructors must fail closed before Java GPU allocation");
		assertTrue(iris.contains("Java Iris shader-storage buffer resizing is unavailable on selected Vulkan")
			|| iris.contains("Java Iris shader-storage buffer resizing is unavailable while Rust owns whole-frame presentation"),
			"compatibility buffer resizing must not reopen Java GPU allocation after construction");
		assertTrue(irisRenderSystem.contains("public static void genBuffers(int[] buffers) {\n\t\trejectJavaGpuObjectCreation(\"buffer\");"),
			"legacy Iris buffer generation must fail closed before Java GPU allocation");
		assertTrue(irisRenderSystem.contains("public static int bufferStorage(int target, float[] data, int usage) {\n\t\tRenderSystem.assertOnRenderThread();\n\t\trejectJavaGpuObjectCreation(\"buffer\");"),
			"legacy Iris bufferStorage overload must fail closed before DSA allocation");
		assertTrue(irisRenderSystem.contains("rejectJavaGpuMutation(\"texture upload\");"),
			"legacy Iris texture uploads must fail closed during Rust whole-frame rendering");
		assertTrue(irisRenderSystem.contains("rejectJavaGpuMutation(\"buffer upload\");"),
			"legacy Iris buffer uploads must fail closed during Rust whole-frame rendering");
		assertTrue(irisRenderSystem.contains("rejectJavaGpuMutation(\"texture parameter update\");"),
			"legacy Iris texture parameter updates must fail closed during Rust whole-frame rendering");
		assertTrue(irisRenderSystem.contains("rejectJavaGpuMutation(\"mipmap generation\");"),
			"legacy Iris mipmap generation must fail closed during Rust whole-frame rendering");
		assertTrue(irisRenderSystem.contains("rejectJavaGpuMutation(\"buffer binding\");"),
			"legacy Iris buffer bindings must fail closed during Rust whole-frame rendering");
		assertTrue(irisRenderSystem.contains("rejectJavaGpuMutation(\"sampler binding\");"),
			"legacy Iris sampler bindings must fail closed during Rust whole-frame rendering");
		assertTrue(irisRenderSystem.contains("rejectJavaGpuMutation(\"compute dispatch\");"),
			"legacy Iris compute dispatch must fail closed during Rust whole-frame rendering");
		assertTrue(irisRenderSystem.contains("rejectJavaGpuMutation(\"buffer clear\");"),
			"legacy Iris buffer clears must fail closed during Rust whole-frame rendering");
		assertTrue(irisRenderSystem.contains("rejectJavaGpuMutation(\"program binding\");"),
			"legacy Iris program binding must fail closed during Rust whole-frame rendering");
		assertTrue(irisRenderSystem.contains("rejectJavaGpuMutation(\"sampler parameter update\");"),
			"legacy Iris sampler parameter updates must fail closed during Rust whole-frame rendering");
		String uploadHelper = Files.readString(Path.of("src/main/java/net/irisshaders/iris/gl/texture/TextureUploadHelper.java"));
		assertTrue(uploadHelper.contains("Java Iris texture-upload state is unavailable while Rust owns whole-frame presentation")
			&& uploadHelper.contains("isVulkanBackendSelected()"),
			"legacy Iris pixel-store setup must fail closed during Rust whole-frame rendering");
	}

	@Test
	void wholeFrameSharedTerrainAndUniformHelpersCannotAllocateJavaResources() throws Exception {
		String uniform = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/GlobalSettingsUniform.java"));
		String mesh = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/chunk/CompiledSectionMesh.java"));
		assertTrue(uniform.contains("Java global-settings UBO is unavailable on selected Vulkan")
			|| uniform.contains("Java global-settings UBO is unavailable while Rust owns whole-frame presentation"));
		assertTrue(mesh.contains("Java terrain mesh uploads are unavailable while Rust owns whole-frame presentation"));
		assertTrue(mesh.contains("Java terrain index uploads are unavailable while Rust owns whole-frame presentation"),
			"shared terrain upload helpers must fail closed before Java command encoders are created");
	}

	@Test
	void sodiumShaderChunkRendererCannotRunDuringRustPresenterHandoff() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/sodium/client/render/chunk/ShaderChunkRenderer.java"));
		int guard = source.indexOf("if (VulkanicAPI.isVulkanBackendSelected()");
		int drawSetup = source.indexOf("CommandEncoder commandEncoder = VulkanicAPI.createCommandEncoder()", guard);
		assertTrue(guard >= 0 && source.indexOf("RustGalVulkanWholeFrameMode.enabled()", guard) > guard,
			"Sodium Java shader chunk rendering must honor the Rust presenter shell");
		assertTrue(drawSetup < 0 || guard < drawSetup,
			"Sodium Java chunk command setup must remain behind the Rust ownership fence");
	}

	@Test
	void sodiumSectionCullingDoesNotProbeIrisFogPolicyDuringHandoff() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/sodium/client/render/chunk/RenderSectionManager.java"));
		int fog = source.indexOf("boolean useFogOcclusion");
		int iris = source.indexOf("net.irisshaders.iris.Iris.getCurrentPack()", fog);
		assertTrue(fog >= 0 && iris > fog
			&& source.substring(fog, iris).contains("RustGalVulkanWholeFrameMode.enabled()"),
			"Sodium section culling must treat the Rust presenter shell as Vulkan before querying Iris fog policy");
	}

	@Test
	void sodiumDefaultChunkRendererDoesNotAllocateJavaBuffersOnSelectedVulkan() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/sodium/client/render/chunk/DefaultChunkRenderer.java"));
		int constructor = source.indexOf("public DefaultChunkRenderer(");
		int guard = source.indexOf("VulkanicAPI.isVulkanBackendSelected()", constructor);
		int allocation = source.indexOf("new SharedQuadIndexBuffer", constructor);
		assertTrue(constructor >= 0 && guard > constructor && guard < allocation,
			"selected Vulkan Sodium chunk renderer must not allocate Java index buffers");
	}

	@Test
    void sodiumSharedQuadIndexBufferCannotConstructJavaBufferUnderRustOwnership() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/sodium/client/render/chunk/SharedQuadIndexBuffer.java"));
		int constructor = source.indexOf("public SharedQuadIndexBuffer(");
		int guard = source.indexOf("RustGalVulkanWholeFrameMode.enabled()", constructor);
		int allocation = source.indexOf("commandList.createMutableBuffer()", constructor);
		assertTrue(constructor >= 0 && guard > constructor && guard < allocation,
			"shared Sodium index buffers must not allocate Java/OpenGL storage while Rust owns Vulkan");
    }

    @Test
    void sodiumRegionStagingCannotAllocateJavaBufferUnderRustOwnership() throws Exception {
        String source = Files.readString(Path.of("src/main/java/net/sodium/client/render/chunk/region/RenderRegionManager.java"));
        int selector = source.indexOf("private static StagingBuffer createStagingBuffer");
        int shell = source.indexOf("RustGalVulkanWholeFrameMode.enabled()", selector);
        int noop = source.indexOf("return new NoopStagingBuffer()", selector);
        int fallback = source.indexOf("return new FallbackStagingBuffer", selector);
        assertTrue(selector >= 0 && shell > selector && noop > shell && fallback > noop,
                "Sodium region staging must remain Java-buffer-free while Rust owns Vulkan presentation");
    }

    @Test
    void sodiumRegionUpdateCannotOpenJavaCommandListUnderRustOwnership() throws Exception {
        String source = Files.readString(Path.of("src/main/java/net/sodium/client/render/chunk/region/RenderRegionManager.java"));
        int method = source.indexOf("public void update()");
        int wholeFrameGuard = source.indexOf("RustGalVulkanWholeFrameMode.enabled()", method);
        int commandList = source.indexOf("createCommandList()", method);
        assertTrue(method >= 0 && wholeFrameGuard > method && wholeFrameGuard < commandList,
                "Sodium region updates must not open Java command lists while Rust owns Vulkan presentation");
    }

	@Test
	void sodiumRegionResourcesDoNotConstructJavaArenasUnderRustOwnership() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/sodium/client/render/chunk/region/RenderRegion.java"));
		int resources = source.indexOf("public DeviceResources(");
		int compactStride = source.indexOf("ChunkMeshFormats.COMPACT.getVertexFormat().getStride()", resources);
		int irisStride = source.indexOf("WorldRenderingSettings.INSTANCE.getVertexFormat()", resources);
		int branch = source.indexOf("if (VulkanicAPI.isVulkanBackendSelected()");
		int shell = source.indexOf("RustGalVulkanWholeFrameMode.enabled()", branch);
		int gpuArena = source.indexOf("new GpuChunkBufferArena", branch);
		int javaArena = source.indexOf("new GlBufferArena", branch);
		assertTrue(branch >= 0 && shell > branch && gpuArena > shell && javaArena > gpuArena,
				"Sodium region resources must use Rust-owned arena path while Rust owns Vulkan presentation");
		assertTrue(resources >= 0 && compactStride > resources && irisStride > compactStride,
				"Sodium region resources must choose the explicit compact stride before querying Iris format state");
	}

    @Test
    void sodiumSectionManagerUsesExplicitSemanticLayoutWithoutIrisState() throws Exception {
        String source = Files.readString(Path.of("src/main/java/net/sodium/client/render/chunk/RenderSectionManager.java"));
        int owned = source.indexOf("boolean rustVulkanOwned");
        int irisFormat = source.indexOf("WorldRenderingSettings.INSTANCE.getVertexFormat()", owned);
        int compact = source.indexOf("ChunkMeshFormats.COMPACT", owned);
        int builder = source.indexOf("new ChunkBuilder", compact);
        assertTrue(owned >= 0 && compact > owned && builder > compact && irisFormat > compact,
                "Sodium section setup must choose an explicit semantic layout before any Iris format query");
        int irisAo = source.indexOf("new ChunkBuilder(level, vertexType)", builder);
        assertTrue(irisAo > builder,
                "OpenGL must retain the Iris-configured builder path while Rust Vulkan uses explicit policy");
		assertTrue(source.contains("new ChunkBuilder(level, vertexType, false, 1)"),
			"Rust Vulkan must not create an unrestricted second native meshing pool beside the whole-frame producer");
    }

    @Test
    void sodiumSectionCollectorSeedingRunsForRustShellOwnership() throws Exception {
        String source = Files.readString(Path.of("src/main/java/net/sodium/client/render/chunk/RenderSectionManager.java"));
        int seed = source.indexOf("this.seedCollectorIfStarved(this.sectionCollector, viewport)");
        int shell = source.lastIndexOf("RustGalVulkanWholeFrameMode.enabled()", seed);
        int probe = source.indexOf("this.logVulkanTerrainCollectorProbe", seed);
        assertTrue(seed >= 0 && shell >= 0 && seed - shell < 180 && probe > seed,
                "Rust-owned terrain shell must retain collector coverage seeding and diagnostics");
    }

    @Test
    void sodiumNativeUpdatedQuadsDoesNotBorrowIrisAoPolicyOnRustRoute() throws Exception {
        String source = Files.readString(Path.of("src/main/java/net/sodium/client/render/chunk/translucent_sorting/bsp_tree/NativeUpdatedQuads.java"));
        int helper = source.indexOf("private static boolean usesSeparateAo()");
        int iris = source.indexOf("WorldRenderingSettings.INSTANCE.shouldUseSeparateAo()", helper);
        int selected = source.indexOf("VulkanicAPI.isVulkanBackendSelected()", helper);
        int shell = source.indexOf("RustGalVulkanWholeFrameMode.enabled()", helper);
        assertTrue(helper >= 0 && selected > helper && shell > selected && iris > shell,
                "Sodium translucent quad updates must use explicit AO policy while Rust owns Vulkan");
    }

    @Test
    void fontAtlasIdentityReplacementRetiresPreviousSemanticPixels() throws Exception {
        String source = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/font/FontTexture.java"));
        int registry = source.indexOf("SEMANTIC_ATLASES.put");
        int previous = source.indexOf("previousAtlas =", source.indexOf("synchronized (SEMANTIC_ATLASES)"));
        int close = source.indexOf("previousAtlas.close()", registry);
        assertTrue(previous >= 0 && close > registry,
                "font atlas identity replacement must retire the prior semantic atlas instead of leaking reload generations");
    }

    @Test
    void guiTextAtlasMetadataIsRetiredWithRawImageReloads() throws Exception {
        String renderer = Files.readString(Path.of("src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"));
        String assets = Files.readString(Path.of("src/main/java/net/vulkanic/gui/RustGalGuiRawImageAssets.java"));
        assertTrue(renderer.contains("invalidateTextAtlasMetadata()")
                        && renderer.contains("TEXT_ATLAS_IDENTITIES.clear()")
                        && renderer.contains("TEXT_ATLAS_GENERATIONS.clear()"),
                "GUI text atlas identity and generation metadata must have an explicit reload retirement hook");
        int invalidate = assets.indexOf("static void invalidate()");
        int retire = assets.indexOf("RustGalGuiRenderer.invalidateTextAtlasMetadata()", invalidate);
        assertTrue(invalidate >= 0 && retire > invalidate,
                "GUI raw-image invalidation must retire text atlas metadata before rebuilding assets");
    }

    @Test
    void semanticTerrainEvictionCancelsQueuedWorkBeforeScheduling() throws Exception {
        String source = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWholeFrameTerrainSource.java"));
        int eviction = source.indexOf("private void evictOutsideWindow");
        int queued = source.indexOf("this.queued.remove(key)", eviction);
        int pending = source.indexOf("this.pending.removeIf", eviction);
        int schedule = source.indexOf("private void scheduleBuilds", eviction);
        assertTrue(eviction >= 0 && queued > eviction && pending > queued && schedule > pending,
                "semantic terrain eviction must cancel queued work before the next worker scheduling pass");
        assertTrue(source.indexOf("this.invalidatedInFlight.add(key)", eviction) > pending,
                "in-flight terrain work must be discarded when its section leaves the active window");
        assertTrue(source.indexOf("this.pending.removeIf(position ->", eviction) > pending,
                "pending sections without retained meshes must also be swept on window eviction");
        int drained = source.indexOf("wholeFrameSurfaceQueueDrained =", source.indexOf("public void enqueue"));
        assertTrue(source.indexOf("this.invalidatedPending.isEmpty()", drained) > drained
                        && source.indexOf("this.invalidatedInFlight.isEmpty()", drained) > drained,
                "terrain readiness must remain unsettled while invalidated work is outstanding");
    }

	@Test
	void wholeFrameDistantHorizonsRenderHookSkipsJavaProxySetup() throws Exception {
		String clientApi = Files.readString(Path.of("src/main/java/com/seibel/distanthorizons/core/api/internal/ClientApi.java"));
		int method = clientApi.indexOf("private void renderLodLayer(boolean renderingDeferredLayer)");
		int guard = clientApi.indexOf("RustGalVulkanWholeFrameMode.enabled()", method);
		int proxy = clientApi.indexOf("GLProxy.getInstance()", method);
		assertTrue(method >= 0 && guard > method && proxy > guard,
			"Rust whole-frame DH hooks must skip Java GLProxy setup and upload-task draining");
	}

	@Test
	void wholeFrameDistantHorizonsFadeHooksCannotDrawJavaPasses() throws Exception {
		String clientApi = Files.readString(Path.of("src/main/java/com/seibel/distanthorizons/core/api/internal/ClientApi.java"));
		int opaque = clientApi.indexOf("public void renderFadeOpaque()");
		int transparent = clientApi.indexOf("public void renderFadeTransparent()");
		assertTrue((clientApi.indexOf("RustGalVulkanWholeFrameMode.enabled()", opaque) > opaque
			|| clientApi.indexOf("RustGalVulkanWholeFrameMode.enabled()", opaque) > opaque)
			&& (clientApi.indexOf("RustGalVulkanWholeFrameMode.enabled()", transparent) > transparent
			|| clientApi.indexOf("RustGalVulkanWholeFrameMode.enabled()", transparent) > transparent),
			"Rust whole-frame DH hooks must not execute Java fade renderers");
	}

	@Test
	void wholeFrameDistantHorizonsLevelHookDoesNotQueryIrisCompat() throws Exception {
		String hook = Files.readString(Path.of("src/main/java/com/seibel/distanthorizons/fabric/hooks/DistantHorizonsLevelRenderHook.java"));
		assertTrue(hook.contains("DHCompatInternal.shouldUseShaderOverrides()")
			&& hook.contains("WorldRenderRoutePolicy")
			&& hook.contains("currentDistantHorizonsOpaqueRoute()")
			&& !hook.contains("VulkanicAPI.isVulkanBackendSelected()"),
			"selected Vulkan DH level hooks must derive admission from route policy without a direct backend query");
	}

	@Test
	void wholeFrameDistantHorizonsTextureStateIsCompatibilityOnly() throws Exception {
		String textureState = Files.readString(Path.of("src/main/java/com/seibel/distanthorizons/core/render/glObject/DhTextureState.java"));
		assertTrue(textureState.contains("Java DH texture state is unavailable while Rust owns whole-frame presentation"),
			"Distant Horizons texture-unit compatibility helpers must fail closed after Rust presentation ownership transfers");
	}

	@Test
	void wholeFrameDistantHorizonsCustomRenderersCannotSubmitJavaPasses() throws Exception {
		String generic = Files.readString(Path.of("src/main/java/com/seibel/distanthorizons/core/render/renderer/generic/GenericObjectRenderer.java"));
		String screenQuad = Files.readString(Path.of("src/main/java/com/seibel/distanthorizons/core/render/renderer/ScreenQuad.java"));
		assertTrue(generic.contains("Java DH generic-object rendering is unavailable while Rust owns whole-frame presentation"),
			"DH generic-object entry points must fail closed under Rust whole-frame presentation");
		assertTrue(screenQuad.contains("Java DH screen-quad rendering is unavailable while Rust owns whole-frame presentation"),
			"DH screen-quad entry points must fail closed under Rust whole-frame presentation");
		assertTrue(screenQuad.contains("VulkanicAPI.isVulkanBackendSelected()"),
			"DH screen-quad entry points must fail closed as soon as Vulkan is selected");
		String shaderRenderer = Files.readString(Path.of("src/main/java/com/seibel/distanthorizons/core/render/renderer/shaders/AbstractShaderRenderer.java"));
		String debugRenderer = Files.readString(Path.of("src/main/java/com/seibel/distanthorizons/core/render/renderer/DebugRenderer.java"));
		String boxGroup = Files.readString(Path.of("src/main/java/com/seibel/distanthorizons/core/render/renderer/generic/RenderableBoxGroup.java"));
		assertTrue(shaderRenderer.contains("isVulkanBackendSelected()") && debugRenderer.contains("isVulkanBackendSelected()")
			&& boxGroup.contains("Java DH box-group buffer uploads are unavailable on the Rust Vulkan route"),
			"selected Vulkan must fence DH shader, debug, and box-buffer Java work before whole-frame admission");
	}

	@Test
	void wholeFramePublicPresentersFailClosedAtTheirOwnEntryPoints() throws Exception {
		String gui = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/render/GuiRenderer.java"));
		String clouds = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/CloudRenderer.java"));
		String composite = Files.readString(Path.of("src/main/java/net/irisshaders/iris/pipeline/CompositeRenderer.java"));
		String finalPass = Files.readString(Path.of("src/main/java/net/irisshaders/iris/pipeline/FinalPassRenderer.java"));
		String pipeline = Files.readString(Path.of("src/main/java/net/irisshaders/iris/pipeline/IrisRenderingPipeline.java"));
		assertTrue(gui.contains("Java GUI rendering is unavailable while Rust owns whole-frame presentation"));
		assertTrue(clouds.contains("Java cloud rendering is unavailable while Rust owns whole-frame presentation"));
		assertTrue(composite.contains("Java Iris composite rendering is unavailable on selected Vulkan")
			|| composite.contains("Java Iris composite rendering is unavailable while Rust owns whole-frame presentation"));
		assertTrue(finalPass.contains("Java Iris final-pass rendering is unavailable while Rust owns whole-frame presentation"));
		assertTrue(pipeline.contains("Java Iris shadow rendering is unavailable while Rust owns whole-frame presentation")
			&& pipeline.contains("Java Iris deferred rendering is unavailable while Rust owns whole-frame presentation")
			&& pipeline.contains("Java Iris composite/final rendering is unavailable while Rust owns whole-frame presentation"));
	}

	@Test
	void endPortalGuiUsesTheCopiedRustMeshContract() throws Exception {
		String graphics = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/GuiGraphics.java"));
		String renderer = Files.readString(Path.of("src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"));
		String loading = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/screens/LevelLoadingScreen.java"));
		String win = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/screens/WinScreen.java"));
		assertTrue(graphics.contains("tryEnqueueEndPortal") && graphics.contains("submitGuiElement(element)"),
			"End Portal GUI must enter Rust through copied semantic mesh elements");
		assertTrue(renderer.contains("Per-vertex UVs are retained")
			&& renderer.contains("END_SKY_LOCATION")
			&& renderer.contains("END_PORTAL_LOCATION")
			&& renderer.contains("enqueueGuiMeshItemRequest"),
			"Rust End Portal GUI admission must preserve both textures and rotated layer UVs");
		assertTrue(loading.contains("guiGraphics.submitRustEndPortal(gameTime)")
			&& win.contains("guiGraphics.submitRustEndPortal(gameTime)"),
			"loading and credits screens must not reopen the Java two-texture portal pass");
	}

	@Test
	void voxelMapGradientUsesRustOwnedSemanticMeshInterpolation() throws Exception {
		String voxelMap = Files.readString(Path.of("src/main/java/net/voxelmap/util/VoxelMapGuiGraphics.java"));
		String graphics = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/GuiGraphics.java"));
		String renderer = Files.readString(Path.of("src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"));
		assertTrue(voxelMap.contains("submitRustSemanticGradientBlit"),
			"VoxelMap gradients must use the semantic GUI path under Rust Vulkan");
		assertTrue(graphics.contains("tryEnqueueGradientBlit") && renderer.contains("tryEnqueueGradientBlit")
			&& renderer.contains("GuiMeshVertexRecord"),
			"gradient colors must be carried as explicit Rust GUI mesh vertex semantics");
		assertTrue(graphics.contains("isWholeFrameVulkanEnabled()")
			&& !graphics.contains("isWholeFrameVulkanActive()"),
			"core GUI texture paths must recognize Rust ownership before presenter activation");
		assertTrue(voxelMap.contains("isWholeFrameVulkanEnabled())")
			&& voxelMap.contains("graphics.submitRustSemanticBlit(texture"),
			"resource-backed VoxelMap blits must not retain a Java texture-view path under Rust Vulkan");
	}

	@Test
	void voxelMapMaskBlitsCannotFallThroughToJavaViewsOnSelectedVulkan() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/voxelmap/util/VoxelMapGuiGraphics.java"));
		int circular = source.indexOf("public static void blitCircular(\n            GuiGraphics graphics, ResourceLocation texture");
		int square = source.indexOf("public static void blitSquareMap(\n                    GuiGraphics graphics, ResourceLocation texture");
		assertTrue(circular >= 0 && square >= 0);
		assertTrue(source.substring(circular, square).contains("isVulkanBackendSelected()")
			&& source.substring(square).contains("isVulkanBackendSelected()"),
			"VoxelMap circular and square masks must not acquire Java texture views on selected Vulkan");
	}

	@Test
	void wholeFrameIrisAuxiliaryResourcesCannotBeConstructedOrDrawn() throws Exception {
		String depth = Files.readString(Path.of("src/main/java/net/irisshaders/iris/pathways/CenterDepthSampler.java"));
		String depthCopy = Files.readString(Path.of("src/main/java/net/irisshaders/iris/gl/texture/DepthCopyStrategy.java"));
		String color = Files.readString(Path.of("src/main/java/net/irisshaders/iris/pathways/colorspace/ColorSpaceFragmentConverter.java"));
		String horizon = Files.readString(Path.of("src/main/java/net/irisshaders/iris/pathways/HorizonRenderer.java"));
		assertTrue(depth.contains("Java Iris center-depth resources are unavailable while Rust owns whole-frame presentation"));
		assertTrue(depthCopy.contains("Java Iris depth snapshots are unavailable while Rust owns whole-frame presentation"),
			"Iris depth-copy compatibility strategies must fail closed before touching Java framebuffer state");
		assertTrue(color.contains("Java Iris color-space resources are unavailable on selected Vulkan")
			|| color.contains("Java Iris color-space resources are unavailable while Rust owns whole-frame presentation"));
		assertTrue(horizon.contains("Java Iris horizon resources are unavailable while Rust owns whole-frame presentation")
			&& horizon.contains("Java Iris horizon rendering is unavailable while Rust owns whole-frame presentation"));
	}

	@Test
	void wholeFrameDoesNotConstructJavaTracyCaptureResources() throws Exception {
		String minecraft = Files.readString(Path.of("src/main/java/net/minecraft/client/Minecraft.java"));
		int tracy = minecraft.indexOf("TracyCompat.isAvailable() && gameConfig.game.captureTracyImages");
		int guard = minecraft.indexOf("!net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()", tracy);
		assertTrue(tracy >= 0 && guard > tracy,
			"Rust whole-frame startup must not allocate Java Tracy GPU capture resources");
	}

	@Test
	void wholeFrameTaczImmediateMeshHelperCannotOpenJavaPass() throws Exception {
		String tacz = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/special/TaczGlock17SpecialRenderer.java"));
		assertTrue(tacz.contains("Java TACZ immediate mesh rendering is unavailable while Rust owns whole-frame presentation"),
			"TACZ's final Java immediate-mesh helper must fail closed under Rust whole-frame presentation");
		int immediateGuard = tacz.indexOf("Java TACZ immediate mesh rendering is unavailable");
		int irisTextureLookup = tacz.indexOf("IrisRenderSystem.getTextureBinding", immediateGuard);
		assertTrue(immediateGuard >= 0 && irisTextureLookup > immediateGuard,
			"TACZ must reject Java immediate rendering before consulting Iris texture-unit state");
	}

	@Test
	void selectedVulkanSubmitStorageCannotCaptureIrisModelState() throws Exception {
		String storage = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/SubmitNodeStorage.java"));
		int helper = storage.indexOf("private static boolean rustWholeFrame()");
		assertTrue(helper >= 0 && storage.indexOf("isVulkanBackendSelected()", helper) > helper,
			"selected Vulkan semantic submissions must disable Iris model-state capture");
		String hitbox = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/feature/HitboxFeatureRenderer.java"));
		assertTrue(hitbox.contains("isVulkanBackendSelected()")
			&& hitbox.contains("Java hitbox rendering is unavailable while Rust Vulkan owns presentation"),
			"selected Vulkan hitbox feature rendering must fail closed");
	}

	@Test
	void selectedVulkanFirstPersonJavaHandsFailClosed() throws Exception {
		String hands = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/ItemInHandRenderer.java"));
		int method = hands.indexOf("public void renderHandsWithItems");
		int guard = hands.indexOf("isVulkanBackendSelected()", method);
		assertTrue(method >= 0 && guard > method,
			"selected Vulkan must reject the legacy first-person hand renderer before Java submission");
	}

	@Test
	void selectedVulkanSubmitNodesCannotCaptureIrisModelStorage() throws Exception {
		String nodes = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/SubmitNodeCollection.java"));
		assertTrue(nodes.contains("&& !net.vulkanic.VulkanicAPI.isVulkanBackendSelected()")
			&& nodes.contains("net.irisshaders.iris.mixinterface.ModelStorage"),
			"selected Vulkan submit nodes must not capture Iris model storage");
		assertTrue(nodes.contains("&& !net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()\n\t\t\t&& net.irisshaders.iris.vertices.ImmediateState.isRenderingBEs"),
			"selected Vulkan submit nodes must not query Iris immediate block-entity state");
	}

	@Test
	void selectedVulkanOutlineBuffersCannotEnterJavaRendering() throws Exception {
		String outline = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/OutlineBufferSource.java"));
		assertTrue(outline.contains("Java entity-outline buffers are unavailable on selected Vulkan")
			&& outline.contains("isVulkanBackendSelected()"),
			"selected Vulkan entity outlines must not acquire Java outline buffers");
	}

	@Test
	void selectedVulkanEndPortalScreensUseSemanticBackgrounds() throws Exception {
		String loading = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/screens/LevelLoadingScreen.java"));
		String win = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/screens/WinScreen.java"));
		assertTrue(loading.contains("isVulkanBackendSelected()") && loading.contains("submitRustEndPortal"),
			"selected Vulkan loading screens must not acquire Java end-portal texture views");
		assertTrue(win.contains("isVulkanBackendSelected()") && win.contains("submitRustEndPortal"),
			"selected Vulkan win screen must not acquire Java end-portal texture views");
	}

	@Test
	void selectedVulkanDebugRenderersCannotEnterJavaGeometry() throws Exception {
		String gameTest = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/debug/GameTestBlockHighlightRenderer.java"));
		String light = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/debug/LightSectionDebugRenderer.java"));
		assertTrue(gameTest.contains("isVulkanBackendSelected()")
			&& light.contains("Java light-section debug rendering is unavailable on selected Vulkan"),
			"selected Vulkan debug renderers must fail closed before Java geometry");
	}

	@Test
	void selectedVulkanCannotInitializeOrMutateJavaVoxelMapAndDhState() throws Exception {
		String minecraft = Files.readString(Path.of("src/main/java/net/minecraft/client/Minecraft.java"));
		String map = Files.readString(Path.of("src/main/java/net/voxelmap/Map.java"));
		String dhBuffer = Files.readString(Path.of("src/main/java/com/seibel/distanthorizons/core/render/glObject/buffer/GLBuffer.java"));
		assertTrue(minecraft.contains("VoxelMapInitializer.initialize()")
			&& minecraft.contains("!net.vulkanic.VulkanicAPI.isVulkanBackendSelected()"),
			"selected Vulkan startup must not initialize VoxelMap Java GPU state");
		assertTrue(map.contains("TEXTURE_UPLOAD_WRITES_VISIBLE_TO_TEXTURE_FETCH")
			&& map.contains("isVulkanBackendSelected()"),
			"selected Vulkan VoxelMap uploads must not issue Java resource barriers");
		assertTrue(dhBuffer.contains("IrisRenderSystem.decrementTrackedBuffers()")
			&& dhBuffer.contains("!VulkanicAPI.isVulkanBackendSelected()"),
			"selected Vulkan DH buffer teardown must not mutate Iris tracking state");
	}

	@Test
	void wholeFrameRemainingJavaPresenterHelpersFailClosed() throws Exception {
		String debug = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/components/DebugScreenOverlay.java"));
		String post = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/PostPass.java"));
		String renderType = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/RenderType.java"));
		assertTrue(debug.contains("Java 3D crosshair rendering is unavailable while Rust owns whole-frame presentation")
			|| debug.contains("Java 3D crosshair rendering is unavailable on selected Vulkan"));
		assertTrue(post.contains("Java post-pass rendering is unavailable while Rust owns whole-frame presentation"));
		assertTrue(renderType.contains("Java immediate RenderType drawing is unavailable while Rust owns whole-frame presentation")
			|| renderType.contains("Java immediate RenderType drawing is unavailable on selected Vulkan"));
	}

	@Test
	void selectedVulkanGuiCannotFallThroughToJavaPresenterOrPictureInPicture() throws Exception {
		String gui = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/render/GuiRenderer.java"));
		int render = gui.indexOf("public void render(GpuBufferSlice");
		int guard = gui.indexOf("VulkanicAPI.isVulkanBackendSelected()", render);
		int prepare = gui.indexOf("this.prepare()", render);
		assertTrue(render >= 0 && guard > render && guard < prepare,
			"selected Vulkan must reject Java GUI rendering before Java preparation");
		assertTrue(gui.contains("Java Vulkan GUI picture-in-picture items are unavailable until the Rust whole-frame GUI route is admitted"),
			"selected Vulkan GUI must not use Java picture-in-picture fallback items");
	}

	@Test
	void selectedVulkanFontAtlasesStaySemanticAndDoNotUploadJavaTextures() throws Exception {
		String font = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/font/FontTexture.java"));
		assertTrue(font.contains("!net.vulkanic.VulkanicAPI.isVulkanBackendSelected()"),
			"font atlas construction must not allocate a Java Vulkan texture");
		assertTrue(font.contains("|| net.vulkanic.VulkanicAPI.isVulkanBackendSelected()"),
			"font glyph views must remain absent on selected Vulkan until Rust admission");
		String special = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/font/glyphs/SpecialGlyphs.java"));
		String bitmap = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/font/providers/BitmapProvider.java"));
		assertTrue(special.contains("&& !net.vulkanic.VulkanicAPI.isVulkanBackendSelected()"));
		assertTrue(bitmap.contains("&& !net.vulkanic.VulkanicAPI.isVulkanBackendSelected()"));
	}

	@Test
	void selectedVulkanFontAtlasHandoffReleasesJavaGpuButKeepsSemanticPixels() throws Exception {
		String texture = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/font/FontTexture.java"));
		String manager = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/font/FontManager.java"));
		assertTrue(texture.contains("public void ensureRustSemanticRoute()")
			&& texture.contains("closeGpuAllocation();")
			&& texture.contains("semanticAtlasPixels"),
			"font atlas handoff must release only Java GPU state while retaining semantic pixels");
		assertFalse(texture.substring(texture.indexOf("public void ensureRustSemanticRoute()"))
			.contains("super.close();"),
			"font atlas handoff must not destroy the CPU semantic atlas registry");
		assertTrue(manager.contains("this.fontSets.values().forEach(FontSet::ensureRustSemanticRoute)")
			&& manager.contains("this.missingFontSet.ensureRustSemanticRoute()"),
			"all active and missing font sets must participate in the Rust handoff");
		String gameRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
		assertTrue(gameRenderer.contains("this.minecraft.getFontManager().ensureRustSemanticRoute()"),
			"Rust whole-frame startup must release pre-selection font GPU state");
	}

	@Test
	void selectedVulkanTextureManagerHandoffReleasesRegisteredJavaGpuAllocations() throws Exception {
		String texture = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/texture/AbstractTexture.java"));
		String manager = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/texture/TextureManager.java"));
		String gameRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
		assertTrue(texture.contains("public void ensureRustSemanticRoute()")
			&& texture.contains("closeGpuAllocation()"),
			"texture handoff must release GPU state without invoking source-data close");
		assertTrue(manager.contains("this.byPath.values().forEach(AbstractTexture::ensureRustSemanticRoute)"),
			"all registered textures, including skins, must receive the handoff");
		assertTrue(gameRenderer.contains("this.minecraft.getTextureManager().ensureRustSemanticRoute()"),
			"Rust frame startup must perform the registered-texture handoff");
	}

	@Test
	void selectedVulkanLevelRendererRetiresStandaloneJavaOutlineTarget() throws Exception {
		String level = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/LevelRenderer.java"));
		String gameRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
		assertTrue(level.contains("public void ensureRustSemanticRoute()")
			&& level.contains("this.entityOutlineTarget.destroyBuffers()")
			&& level.contains("this.entityOutlineTarget = null"),
			"Rust handoff must retire the standalone Java outline target");
		assertTrue(gameRenderer.contains("this.minecraft.levelRenderer.ensureRustSemanticRoute()"),
			"Rust frame startup must retire LevelRenderer-owned Java targets");
	}

	@Test
	void selectedVulkanGameRendererReleasesPreSelectionProjectionUbos() throws Exception {
		String perspective = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/PerspectiveProjectionMatrixBuffer.java"));
		String cached = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/CachedPerspectiveProjectionMatrixBuffer.java"));
		String settings = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/GlobalSettingsUniform.java"));
		String gameRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
		assertTrue(perspective.contains("ensureRustSemanticRoute()") && cached.contains("ensureRustSemanticRoute()")
			&& settings.contains("ensureRustSemanticRoute()"),
			"long-lived Java UBO wrappers must expose a Rust ownership handoff");
		assertTrue(gameRenderer.contains("this.globalSettingsUniform.ensureRustSemanticRoute()")
			&& gameRenderer.contains("this.levelProjectionMatrixBuffer.ensureRustSemanticRoute()")
			&& gameRenderer.contains("this.handProjectionMatrixBuffer.ensureRustSemanticRoute()")
			&& gameRenderer.contains("this.hud3dProjectionMatrixBuffer.ensureRustSemanticRoute()"),
			"Rust frame startup must release every GameRenderer-owned Java projection UBO");
	}

	@Test
	void selectedVulkanGameRendererReleasesLightingAndPanoramaProjectionUbos() throws Exception {
		String lighting = Files.readString(Path.of("src/main/java/net/blaze3d/platform/Lighting.java"));
		String cubeMap = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/CubeMap.java"));
		String gameRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
		assertTrue(lighting.contains("ensureRustSemanticRoute()") && lighting.contains("this.buffer = null"),
			"lighting handoff must release the Java UBO without retaining it");
		assertTrue(cubeMap.contains("ensureRustSemanticRoute()"),
			"panorama cubemap must expose its projection UBO handoff");
		assertTrue(gameRenderer.contains("this.lighting.ensureRustSemanticRoute()")
			&& gameRenderer.contains("this.cubeMap.ensureRustSemanticRoute()"),
			"Rust frame startup must release lighting and panorama UBOs");
	}

	@Test
	void selectedVulkanDistantHorizonsFogCannotEnterJavaCompatibilityRenderer() throws Exception {
		String fog = Files.readString(Path.of("src/main/java/com/seibel/distanthorizons/core/render/renderer/FogRenderer.java"));
		int render = fog.indexOf("public void render(");
		int guard = fog.indexOf("VulkanicAPI.isVulkanBackendSelected()", render);
		assertTrue(render >= 0 && guard > render,
			"selected Vulkan must fail closed before Distant Horizons Java fog rendering");
		assertTrue(fog.contains("Java Distant Horizons fog rendering is unavailable while Rust owns Vulkan presentation"),
			"Distant Horizons fog fallback must remain unavailable until a Rust route is admitted");
	}

	@Test
	void selectedVulkanGameRendererReleasesPreSelectionFogUbos() throws Exception {
		String fog = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/fog/FogRenderer.java"));
		String gameRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
		assertTrue(fog.contains("public void ensureRustSemanticRoute()")
			&& fog.contains("this.emptyBuffer = null")
			&& fog.contains("this.regularBuffer = null"),
			"vanilla fog handoff must release pre-selection Java UBO state");
		assertTrue(gameRenderer.contains("this.fogRenderer.ensureRustSemanticRoute()"),
			"Rust frame startup must release GameRenderer-owned fog UBOs");
	}

	@Test
	void selectedVulkanGameRendererClearsPooledJavaPhysicalResources() throws Exception {
		String pool = Files.readString(Path.of("src/main/java/net/blaze3d/resource/CrossFrameResourcePool.java"));
		String gameRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
		assertTrue(pool.contains("public void ensureRustSemanticRoute()")
			&& pool.contains("this.clear()"),
			"Rust handoff must free pooled Java physical resources");
		assertTrue(gameRenderer.contains("this.resourcePool.ensureRustSemanticRoute()"),
			"Rust frame startup must clear the GameRenderer resource pool");
	}

	@Test
	void selectedVulkanGuiRendererReleasesJavaGpuStateWithoutClosingSemanticQueues() throws Exception {
		String gui = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/render/GuiRenderer.java"));
		String gameRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
		assertTrue(gui.contains("public void ensureRustSemanticRoute()")
			&& gui.contains("this.vertexBuffers.clear()")
			&& gui.contains("this.itemsAtlas = null")
			&& gui.contains("this.itemsAtlasDepth = null"),
			"Rust GUI handoff must release Java item/vertex GPU state");
		assertTrue(gameRenderer.contains("this.guiRenderer.ensureRustSemanticRoute()"),
			"Rust frame startup must release Java GUI renderer allocations");
	}

	@Test
	void selectedVulkanDoesNotCreateJavaDynamicUniformRing() throws Exception {
		String api = Files.readString(Path.of("src/main/java/net/vulkanic/VulkanicAPI.java"));
		int init = api.indexOf("public static void initializeDynamicUniforms()");
		int reset = api.indexOf("public static void resetDynamicUniforms()");
		assertTrue(init >= 0 && api.indexOf("isVulkanBackendSelected()", init) < reset,
			"selected Vulkan must suppress Java dynamic-uniform initialization");
		assertTrue(reset >= 0 && api.indexOf("isVulkanBackendSelected()", reset) > reset,
			"selected Vulkan must not rotate a Java dynamic-uniform ring");
	}

	@Test
	void selectedVulkanReleasesSharedJavaIndexBuffers() throws Exception {
		String api = Files.readString(Path.of("src/main/java/net/vulkanic/VulkanicAPI.java"));
		String gameRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
		assertTrue(api.contains("public static void ensureRustSemanticRoute()")
			&& api.contains("sharedSequential.releaseRustSemanticRoute()")
			&& api.contains("sharedSequentialQuad.releaseRustSemanticRoute()")
			&& api.contains("sharedSequentialLines.releaseRustSemanticRoute()"),
			"shared Java index buffers must have an explicit Rust handoff");
		assertTrue(gameRenderer.contains("VulkanicAPI.ensureRustSemanticRoute()"),
			"Rust frame startup must release shared Java index buffers");
	}

	@Test
	void selectedVulkanClearsSharedJavaUniformAndTextureReferences() throws Exception {
		String api = Files.readString(Path.of("src/main/java/net/vulkanic/VulkanicAPI.java"));
		assertTrue(api.contains("dynamicUniforms.close()")
			&& api.contains("projectionMatrixBuffer = null")
			&& api.contains("shaderFog = null")
			&& api.contains("shaderLightDirections = null")
			&& api.contains("globalSettingsUniform = null")
			&& api.contains("outputColorTextureOverride = null")
			&& api.contains("outputDepthTextureOverride = null"),
			"Rust handoff must clear stale shared Java buffer and texture-view references");
	}

	@Test
	void selectedVulkanReleasesIrisCompatibilityProjectionBuffer() throws Exception {
		String iris = Files.readString(Path.of("src/main/java/net/irisshaders/iris/gl/IrisRenderSystem.java"));
		String api = Files.readString(Path.of("src/main/java/net/vulkanic/VulkanicAPI.java"));
		assertTrue(iris.contains("public static void ensureRustSemanticRoute()")
			&& iris.contains("perspectiveProjectionMatrixBuffer.close()")
			&& iris.contains("perspectiveProjectionMatrixBuffer = null"),
			"Iris compatibility projection state must be released on Rust ownership");
		int handoff = api.indexOf("public static void ensureRustSemanticRoute()");
		int nextMethod = api.indexOf("\n\t}\n\n    public static", handoff);
		assertTrue(handoff >= 0 && nextMethod > handoff,
			"the Rust ownership transition must remain explicit and bounded");
		assertFalse(api.substring(handoff, nextMethod).contains("IrisRenderSystem.ensureRustSemanticRoute()"),
			"the Rust ownership transition must not initialize Iris runtime classes");
	}

	@Test
	void selectedVulkanGlyphProvidersDoNotDereferenceJavaAtlasOrSkinViews() throws Exception {
		String atlas = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/font/AtlasGlyphProvider.java"));
		String player = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/font/PlayerGlyphProvider.java"));
		String renderTypes = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/font/GlyphRenderTypes.java"));
		assertTrue(atlas.contains("net.vulkanic.VulkanicAPI.isVulkanBackendSelected()")
			&& player.contains("net.vulkanic.VulkanicAPI.isVulkanBackendSelected()"),
			"selected Vulkan semantic glyphs must not dereference Java atlas or skin texture views");
		assertTrue(renderTypes.contains("!net.vulkanic.VulkanicAPI.isVulkanBackendSelected()"),
			"selected Vulkan glyph pipeline selection must not enter Iris Java block-entity wrapping");
	}

	@Test
	void selectedVulkanWeatherCannotFallThroughToJavaOrIrisPolicy() throws Exception {
		String weather = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/WeatherEffectRenderer.java"));
		int render = weather.indexOf("public void render(");
		int route = weather.indexOf("currentWeatherRoute()", render);
		int guard = weather.indexOf("Java Vulkan weather rendering is unavailable until the Rust whole-frame weather route is admitted", render);
		int iris = weather.indexOf("Iris.getPipelineManager()", render);
		assertTrue(render >= 0 && route > render && guard > route && guard < iris,
			"selected Vulkan weather must reject before consulting Iris or Java weather buffers");
		int particles = weather.indexOf("public void tickRainParticles(");
		int particleGuard = weather.indexOf("if ((net.vulkanic.VulkanicAPI.isVulkanBackendSelected() || rustWholeFrame) && !rustWeather)", particles);
		int particleIris = weather.indexOf("Iris.getPipelineManager()", particles);
		assertTrue(particles >= 0 && particleGuard > particles && particleGuard < particleIris,
			"unadmitted Vulkan weather particles must not query Iris policy");
	}

	@Test
	void selectedVulkanLightmapStaysSemanticAndHasNoJavaTextureRoute() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/LightTexture.java"));
		int constructor = source.indexOf("public LightTexture(GameRenderer gameRenderer, Minecraft minecraft)");
		int create = source.indexOf("VulkanicAPI.createTexture(", constructor);
		assertTrue(constructor >= 0 && source.indexOf("VulkanicAPI.isVulkanBackendSelected()", constructor) < create
			&& source.indexOf("RustGalVulkanWholeFrameMode.enabled()", constructor) < create
			&& source.indexOf("this.texture = null", constructor) < create,
			"selected Vulkan lightmaps must not allocate a Java GPU texture");
		assertTrue(source.contains("use semantic lightmap inputs")
			&& source.contains("!VulkanicAPI.isVulkanBackendSelected()"),
			"selected Vulkan lightmap consumers must fail closed before Java binding or Iris state");
		assertTrue(source.contains("if (this.minecraft.player == null)")
			&& source.contains("this.rustSemanticLightmapInputs = null"),
			"semantic lightmap extraction must remain unavailable until the client player exists");
	}

	@Test
	void selectedVulkanFeatureDispatcherCannotInvokeJavaEntityBlockOrParticleDraws() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/feature/FeatureRenderDispatcher.java"));
		int method = source.indexOf("public void renderAllFeatures()");
		int wholeFrame = source.indexOf("RustGalVulkanWholeFrameMode.enabled()", method);
		int selected = source.indexOf("VulkanicAPI.isVulkanBackendSelected()", wholeFrame);
		int loop = source.indexOf("for (SubmitNodeCollection", selected);
		assertTrue(method >= 0 && wholeFrame > method && selected > wholeFrame && selected < loop,
			"selected Vulkan must fail closed before Java entity/block/particle feature dispatch");
		assertTrue(source.contains("Java Vulkan feature rendering is unavailable until the Rust whole-frame entity route is admitted"));
	}

	@Test
	void selectedVulkanGameRendererAndPostChainCannotReopenJavaPresenter() throws Exception {
		String gameRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
		int render = gameRenderer.indexOf("public void render(DeltaTracker");
		int selected = gameRenderer.indexOf("VulkanicAPI.isVulkanBackendSelected()", render);
		int timing = gameRenderer.indexOf("float realTickDelta", render);
		assertTrue(render >= 0 && selected > render && selected < timing,
			"selected Vulkan must reject the legacy GameRenderer before any Java frame work");

		String chain = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/PostChain.java"));
		assertTrue(chain.contains("Java Vulkan post-chain admission is unavailable until the Rust whole-frame route is admitted"));
		assertTrue(chain.contains("Java Vulkan post-chain processing is unavailable until the Rust whole-frame route is admitted"));
		String pass = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/PostPass.java"));
		assertTrue(pass.contains("Java Vulkan post-pass rendering is unavailable until the Rust whole-frame route is admitted"));
	}

	@Test
	void selectedVulkanCannotOpenASecondJavaPresenter() throws Exception {
		String target = Files.readString(Path.of("src/main/java/net/blaze3d/pipeline/RenderTarget.java"));
		assertTrue(target.contains("Java Vulkan RenderTarget presentation is unavailable until the Rust whole-frame presenter is admitted"));
		String renderSystem = Files.readString(Path.of("src/main/java/net/blaze3d/systems/RenderSystem.java"));
		int flip = renderSystem.indexOf("public static void flipFrame(");
		int guard = renderSystem.indexOf("Java Vulkan frame presentation is unavailable until the Rust whole-frame presenter is admitted", flip);
		assertTrue(flip >= 0 && guard > flip,
			"selected Vulkan must reject Java frame flip before beginFrame/endFrame");
		String api = Files.readString(Path.of("src/main/java/net/vulkanic/VulkanicAPI.java"));
		assertTrue(api.contains("Java Vulkan beginFrame is unavailable until the Rust whole-frame presenter is admitted"));
		assertTrue(api.contains("Java Vulkan endFrame is unavailable until the Rust whole-frame presenter is admitted"));
		assertTrue(api.contains("Java Vulkan presentTextureToScreen is unavailable until the Rust whole-frame presenter is admitted"));
	}

	@Test
	void selectedVulkanResourcePackTexturesStayCpuOwnedUntilRustAdmission() throws Exception {
		String reloadable = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/texture/ReloadableTexture.java"));
		int load = reloadable.indexOf("protected void doLoad(");
		int guard = reloadable.indexOf("VulkanicAPI.isVulkanBackendSelected()", load);
		int create = reloadable.indexOf("VulkanicAPI.createTexture", load);
		assertTrue(load >= 0 && guard > load && guard < create,
			"selected Vulkan resource textures must remain CPU-owned before Rust asset admission");

		String atlas = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/texture/TextureAtlas.java"));
		assertTrue(atlas.contains("|| net.vulkanic.VulkanicAPI.isVulkanBackendSelected()"));
		String sprite = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/texture/SpriteContents.java"));
		assertTrue(sprite.contains("|| net.vulkanic.VulkanicAPI.isVulkanBackendSelected()"));
		assertTrue(sprite.contains("&& !net.vulkanic.VulkanicAPI.isVulkanBackendSelected()"));
	}

	@Test
	void selectedVulkanTextureViewsCannotPublishIrisGpuTrackingState() throws Exception {
		String abstractTexture = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/texture/AbstractTexture.java"));
		String reloadable = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/texture/ReloadableTexture.java"));
		String manager = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/texture/TextureManager.java"));
		assertTrue(abstractTexture.contains("!net.vulkanic.VulkanicAPI.isVulkanBackendSelected()")
			&& reloadable.contains("!net.vulkanic.VulkanicAPI.isVulkanBackendSelected()"),
			"selected Vulkan texture access must not publish Java GPU objects to Iris tracking");
		assertTrue(manager.contains("!net.vulkanic.VulkanicAPI.isVulkanBackendSelected()")
			&& manager.contains("PBRTextureManager.INSTANCE.clear()"),
			"selected Vulkan resource reloads must not maintain Iris PBR GPU registries");
	}

	@Test
	void wholeFrameScreenshotReadbackCannotOpenJavaCommandEncoder() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/Screenshot.java"));
		int method = source.indexOf("public static void takeScreenshot(RenderTarget renderTarget, int i");
		int guard = source.indexOf("RustGalVulkanWholeFrameMode.enabled()", method);
		int createEncoder = source.indexOf("VulkanicAPI.createCommandEncoder()", method);
		assertTrue(method >= 0 && guard > method,
			"screenshot readback must fail closed before Java GPU work on whole-frame Vulkan");
		assertTrue(createEncoder < 0 || createEncoder > guard,
			"whole-frame screenshot readback must not open a Java command encoder");
	}

	@Test
	void wholeFrameSodiumRendererCannotSelectJavaVulkanImplementation() throws Exception {
		String sodium = Files.readString(Path.of("src/main/java/net/sodium/client/render/chunk/DefaultChunkRenderer.java"));
		int render = sodium.indexOf("public void render(ChunkRenderMatrices");
		int guard = sodium.indexOf("Java Sodium chunk rendering is unavailable while Rust owns whole-frame presentation", render);
		int backendBranch = sodium.indexOf("VulkanicAPI.isVulkanBackendSelected()", render);
		assertTrue(render >= 0 && guard > render && backendBranch > guard,
			"Sodium's Java Vulkan terrain branch must be fenced before backend selection");
		assertTrue(sodium.contains("Java Sodium Vulkan chunk rendering is unavailable until the Rust whole-frame terrain route is admitted"),
			"Sodium must fail closed when Vulkan is selected but Rust terrain ownership is unavailable");
	}

	@Test
	void sodiumTerrainManagerCannotReopenJavaVulkanWhenRustRouteIsUnavailable() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/sodium/client/render/chunk/RenderSectionManager.java"));
		int method = source.indexOf("public void renderLayer(");
		int wholeFrameGuard = source.indexOf("RustGalVulkanWholeFrameMode.enabled()", method);
		int selectedGuard = source.indexOf("VulkanicAPI.isVulkanBackendSelected()", wholeFrameGuard);
		int createCommandList = source.indexOf("device.createCommandList()", method);
		assertTrue(method >= 0 && wholeFrameGuard > method && selectedGuard > wholeFrameGuard,
			"Sodium's terrain manager must fence selected Vulkan before Java command-list creation");
		assertTrue(selectedGuard < createCommandList,
			"selected Vulkan must fail closed before the Java terrain command list is created");
	}

	@Test
	void sodiumWorldOrchestratorCannotCreateJavaVulkanTerrainResources() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/sodium/client/render/SodiumWorldRenderer.java"));
		int legacyEnqueue = source.indexOf("public void enqueueRustGalStaticTerrain(");
		int legacyGuard = source.indexOf("usesRustWholeFrameVulkan()", legacyEnqueue);
		int legacyLists = source.indexOf("getRenderLists()", legacyEnqueue);
		assertTrue(legacyEnqueue >= 0 && legacyGuard > legacyEnqueue && legacyGuard < legacyLists,
			"legacy Sodium render-list terrain enqueue must be fenced while Rust owns whole-frame Vulkan");

		int setLevel = source.indexOf("public void setLevel(");
		int setLevelGuard = source.indexOf("VulkanicAPI.isVulkanBackendSelected()", setLevel);
		int setLevelCommandList = source.indexOf("createCommandList()", setLevel);
		assertTrue(setLevel >= 0 && setLevelGuard > setLevel,
			"Sodium world setup must reject selected Vulkan before Java level initialization");
		assertTrue(setLevelCommandList < 0 || setLevelGuard < setLevelCommandList,
			"selected Vulkan must be fenced before Sodium can create a Java command list during level setup");

		int draw = source.indexOf("public void drawChunkLayer(");
		int drawGuard = source.indexOf("VulkanicAPI.isVulkanBackendSelected()", draw);
		assertTrue(draw >= 0 && drawGuard > draw,
			"Sodium world draw must reject selected Vulkan before Java terrain dispatch");

		int reload = source.indexOf("public void reload(");
		int reloadGuard = source.indexOf("VulkanicAPI.isVulkanBackendSelected()", reload);
		int reloadCommandList = source.indexOf("createCommandList()", reload);
		assertTrue(reload >= 0 && reloadGuard > reload && reloadGuard < reloadCommandList,
			"selected Vulkan must be fenced before Sodium reload creates Java GPU work");
	}

	@Test
	void wholeFrameSodiumEntityHookCannotReopenJavaShadowBuffers() throws Exception {
		String hook = Files.readString(Path.of("src/main/java/net/sodium/fabric/SodiumEntityRenderHook.java"));
		int method = hook.indexOf("onRenderEntityShadows(");
		int buffer = hook.indexOf("bufferSource.getBuffer", method);
		assertTrue(method >= 0 && buffer > method);
		String body = hook.substring(method, buffer);
		assertTrue(body.contains("RustGalVulkanWholeFrameMode.enabled()"));
		assertTrue(body.contains("Java Sodium entity-shadow hook is unavailable"));
	}

	@Test
	void wholeFrameDistantHorizonsLayerHookCannotReopenJavaFadePasses() throws Exception {
		String hook = Files.readString(Path.of("src/main/java/com/seibel/distanthorizons/fabric/hooks/DistantHorizonsChunkRenderHook.java"));
		int method = hook.indexOf("onBeforeRenderLayer(");
		int state = hook.indexOf("ClientApi.RENDER_STATE", method);
		assertTrue(method >= 0 && state > method);
		String body = hook.substring(method, state);
		assertTrue(body.contains("RustGalVulkanWholeFrameMode.enabled()"));
		assertTrue(body.contains("Java Distant Horizons layer hook is unavailable"));
		assertTrue(body.contains("net.vulkanic.VulkanicAPI.isVulkanBackendSelected()"),
			"DH layer hook must fence selected Vulkan before Java fade dispatch");
	}

	@Test
	void wholeFrameDistantHorizonsCompatibilityPassCannotCreateJavaVulkanState() throws Exception {
		String lod = Files.readString(Path.of("src/main/java/com/seibel/distanthorizons/core/render/renderer/LodRenderer.java"));
		int method = lod.indexOf("private RenderPass createVulkanCompatibilityRenderPass(");
		int backend = lod.indexOf("VulkanicAPI.isVulkanBackendSelected()", method);
		assertTrue(method >= 0 && backend > method);
		String body = lod.substring(method, backend);
		assertTrue(body.contains("RustGalVulkanWholeFrameMode.enabled()"));
		assertTrue(body.contains("Java Distant Horizons compatibility render pass is unavailable"));
	}

	@Test
	void wholeFrameRenderTargetCannotBecomeASecondPresenter() throws Exception {
		String target = Files.readString(Path.of("src/main/java/net/blaze3d/pipeline/RenderTarget.java"));
		int blit = target.indexOf("public void blitToScreen()");
		int guard = target.indexOf("Java RenderTarget presentation is unavailable while Rust owns whole-frame presentation", blit);
		assertTrue(blit >= 0 && guard > blit,
			"RenderTarget's legacy presentation entry must fail closed under Rust whole-frame ownership");
		assertTrue(target.contains("rejectRustWholeFrameOperation(\"Iris framebuffer binding\")"),
			"RenderTarget Iris framebuffer binding must fail closed under Rust whole-frame ownership");
		assertTrue(target.contains("|| net.vulkanic.VulkanicAPI.isVulkanBackendSelected()"),
			"RenderTarget compatibility operations must also fail closed during selected-Vulkan startup races");
		String api = Files.readString(Path.of("src/main/java/net/vulkanic/VulkanicAPI.java"));
		assertTrue(api.contains("rejectJavaVulkanWholeFrameOperation(\"framebuffer binding\")"),
			"VulkanicAPI framebuffer binding must fail closed before Java compatibility state is touched");
		assertTrue(api.contains("rejectJavaVulkanWholeFrameOperation(\"render-target binding\")"),
			"VulkanicAPI render-target binding must fail closed before backend dispatch");
	}

	@Test
	void wholeFrameJavaFrameGraphCannotReopenVanillaPasses() throws Exception {
		String frameGraph = Files.readString(Path.of("src/main/java/net/blaze3d/framegraph/FrameGraphBuilder.java"));
		assertTrue(frameGraph.contains("Java frame-graph execution is unavailable while Rust owns whole-frame presentation"),
			"FrameGraphBuilder must fail closed before Java post/sky/weather execution under Rust ownership");
		assertTrue(frameGraph.contains("VulkanicAPI.isVulkanBackendSelected()"),
			"selected Vulkan must not execute the Java frame graph before whole-frame admission");
	}

	@Test
	void wholeFrameCannotPublishJavaNativeGpuHandles() throws Exception {
		String api = Files.readString(Path.of("src/main/java/net/vulkanic/VulkanicAPI.java"));
		assertTrue(api.contains("Java Vulkan texture handles are unavailable while Rust owns whole-frame presentation"));
		assertTrue(api.contains("Java Vulkan buffer handles are unavailable while Rust owns whole-frame presentation"));
		assertTrue(api.contains("Java Vulkan framebuffer handles are unavailable while Rust owns whole-frame presentation"));
		assertTrue(api.contains("Java Vulkan legacy texture handles are unavailable while Rust owns the selected Vulkan route"));
	}

	@Test
	void selectedVulkanCannotReconstructLegacyGlBuffersForDhParity() throws Exception {
		String api = Files.readString(Path.of("src/main/java/net/vulkanic/VulkanicAPI.java"));
		int helper = api.indexOf("private static @Nullable GpuBuffer shaderInputParityLegacyDrawBuffer");
		int branch = api.indexOf("case VULKAN -> null", helper);
		assertTrue(helper >= 0 && branch > helper,
			"Selected Vulkan must leave legacy DH GL-handle parity probing unavailable");
		assertFalse(api.substring(helper, Math.min(api.length(), branch + 160)).contains("new net.blaze3d.opengl.LegacyHandleGlBuffer"),
			"Vulkan parity probing must not construct Java LegacyHandleGlBuffer wrappers");
	}

	@Test
	void wholeFrameIrisShadowCompositeCannotReopenJavaPasses() throws Exception {
		String shadow = Files.readString(Path.of("src/main/java/net/irisshaders/iris/shadows/ShadowCompositeRenderer.java"));
		int render = shadow.indexOf("public void renderAll()");
		int guard = shadow.indexOf("Java Iris shadow-composite rendering is unavailable on selected Vulkan", render);
		if (guard < 0) {
			guard = shadow.indexOf("Java Iris shadow-composite rendering is unavailable while Rust owns whole-frame presentation", render);
		}
		assertTrue(render >= 0 && guard > render,
			"Iris shadow-composite entry must fail closed under Rust whole-frame ownership");
	}

	@Test
	void wholeFrameIrisPipelineCannotBindJavaFramebuffersOrSkyPasses() throws Exception {
		String pipeline = Files.readString(Path.of("src/main/java/net/irisshaders/iris/pipeline/IrisRenderingPipeline.java"));
		String iris = Files.readString(Path.of("src/main/java/net/irisshaders/iris/Iris.java"));
		assertTrue((iris.contains("isVulkanBackendSelected()") || iris.contains("isVulkanBackendInitializedAndSelected()"))
			&& iris.contains("RustGalVulkanWholeFrameMode.enabled()")
			&& iris.indexOf("return new VanillaRenderingPipeline()") < iris.indexOf("new IrisRenderingPipeline(programs)"),
			"Iris must not construct its Java shader pipeline after Rust Vulkan ownership is selected");
		assertTrue(pipeline.contains("Java Iris level begin/clear passes are unavailable while Rust owns whole-frame presentation"));
		assertTrue(pipeline.contains("Java Iris depth-copy hand prepass is unavailable while Rust owns whole-frame presentation"));
		assertTrue(pipeline.contains("Java Iris color-space post-processing is unavailable while Rust owns whole-frame presentation"));
		assertTrue(pipeline.contains("Java Iris sky clear/horizon rendering is unavailable while Rust owns whole-frame presentation"));
		assertTrue(pipeline.contains("Java Iris default framebuffer binding is unavailable while Rust owns whole-frame presentation"));
		assertTrue(pipeline.contains("Java Iris sky render-pass creation is unavailable while Rust owns whole-frame presentation"));
		assertTrue(pipeline.contains("Java Iris shadow framebuffer binding is unavailable while Rust owns whole-frame presentation"));
	}

	@Test
	void wholeFrameDistantHorizonsCannotBindIrisJavaProgramsOrFramebuffers() throws Exception {
		String generic = Files.readString(Path.of("src/main/java/net/irisshaders/iris/compat/dh/IrisGenericRenderProgram.java"));
		String lod = Files.readString(Path.of("src/main/java/net/irisshaders/iris/compat/dh/IrisLodRenderProgram.java"));
		String compat = Files.readString(Path.of("src/main/java/net/irisshaders/iris/compat/dh/DHCompatInternal.java"));
		String framebuffer = Files.readString(Path.of("src/main/java/net/irisshaders/iris/compat/dh/DhFrameBufferWrapper.java"));
		assertTrue(generic.contains("Java Iris Distant Horizons shader binding is unavailable while Rust owns whole-frame presentation"));
		assertTrue(generic.contains("Java Iris Distant Horizons shader unbinding is unavailable while Rust owns whole-frame presentation"));
		assertTrue(generic.contains("Java Iris Distant Horizons vertex-buffer binding is unavailable while Rust owns whole-frame presentation"));
		assertTrue(generic.contains("Java Iris Distant Horizons shader programs are unavailable on the Rust Vulkan route")
			&& lod.contains("Java Iris Distant Horizons shader programs are unavailable on the Rust Vulkan route"),
			"selected Vulkan must reject DH Iris shader-program construction before Java handles are created");
		assertTrue(lod.contains("Java Iris Distant Horizons shader binding is unavailable while Rust owns whole-frame presentation"));
		assertTrue(lod.contains("Java Iris Distant Horizons shader unbinding is unavailable while Rust owns whole-frame presentation"));
		assertTrue(compat.contains("isVulkanBackendSelected()")
			&& compat.contains("RustGalVulkanWholeFrameMode.enabled()")
			&& compat.indexOf("incompatible = true") < compat.indexOf("createDepthTex("),
			"DH Iris compatibility construction must fail closed before Java depth/program allocation");
		assertTrue(framebuffer.contains("Java Iris Distant Horizons framebuffer binding is unavailable while Rust owns whole-frame presentation"));
		assertTrue(framebuffer.contains("Java Iris Distant Horizons framebuffer attachment mutation is unavailable while Rust owns whole-frame presentation"));
		assertTrue(generic.contains("VulkanicAPI.isVulkanBackendSelected()")
			&& lod.contains("VulkanicAPI.isVulkanBackendSelected()")
			&& framebuffer.contains("VulkanicAPI.isVulkanBackendSelected()"),
			"selected Vulkan must reject every DH Iris Java bind path before whole-frame admission");
	}

	@Test
	void wholeFrameIrisShadowRendererCannotEnterJavaShadowPass() throws Exception {
		String shadow = Files.readString(Path.of("src/main/java/net/irisshaders/iris/shadows/ShadowRenderer.java"));
		int render = shadow.indexOf("public void renderShadows(LevelRenderer levelRenderer");
		int guard = shadow.indexOf("Java Iris shadow renderer entrypoint is unavailable on selected Vulkan", render);
		if (guard < 0) {
			guard = shadow.indexOf("Java Iris shadow renderer entrypoint is unavailable while Rust owns whole-frame presentation", render);
		}
		assertTrue(render >= 0 && guard > render);
	}

	@Test
	void wholeFrameDistantHorizonsCannotBorrowJavaLightmapOrTargetHandles() throws Exception {
		String lightmap = Files.readString(Path.of("src/main/java/com/seibel/distanthorizons/common/wrappers/misc/LightMapWrapper.java"));
		String render = Files.readString(Path.of("src/main/java/com/seibel/distanthorizons/common/wrappers/minecraft/MinecraftRenderWrapper.java"));
		String hook = Files.readString(Path.of("src/main/java/com/seibel/distanthorizons/fabric/hooks/DhLightTextureHook.java"));
		assertTrue(lightmap.contains("Java Distant Horizons lightmap binding is unavailable while Rust owns whole-frame presentation"));
		assertTrue(lightmap.contains("Java Distant Horizons lightmap unbinding is unavailable while Rust owns whole-frame presentation"));
		assertTrue(render.contains("Java Distant Horizons render-target binding is unavailable while Rust owns whole-frame presentation"));
		assertTrue(render.contains("Java Distant Horizons render-target identity is unavailable while Rust owns whole-frame presentation"));
		assertTrue(render.contains("Java Distant Horizons framebuffer handles are unavailable while Rust owns whole-frame presentation"));
		assertTrue(render.contains("Java Distant Horizons depth-texture handles are unavailable while Rust owns whole-frame presentation"));
		assertTrue(render.contains("Java Distant Horizons color-texture handles are unavailable while Rust owns whole-frame presentation"));
		assertTrue(hook.contains("Rust owns the copied semantic lightmap inputs"),
			"DH lightmap hooks must not convert Java lightmap state into handles on the Rust route");
	}

	@Test
	void wholeFrameDistantHorizonsApiCannotReopenJavaLodOrFadeRendering() throws Exception {
		String clientApi = Files.readString(Path.of("src/main/java/com/seibel/distanthorizons/core/api/internal/ClientApi.java"));
		assertTrue(clientApi.contains("Java Distant Horizons LOD rendering is unavailable while Rust owns whole-frame presentation"));
		assertTrue(clientApi.contains("Java Distant Horizons deferred LOD rendering is unavailable while Rust owns whole-frame presentation"));
		assertTrue(clientApi.contains("Java Distant Horizons opaque fade rendering is unavailable while Rust owns whole-frame presentation"));
		assertTrue(clientApi.contains("Java Distant Horizons transparent fade rendering is unavailable while Rust owns whole-frame presentation"));
		assertTrue(clientApi.contains("net.vulkanic.VulkanicAPI.isVulkanBackendSelected()"),
			"DH public LOD and fade APIs must fence selected Vulkan before shell activation");
	}

	@Test
	void wholeFrameDistantHorizonsCannotConstructLegacyGpuObjects() throws Exception {
		String color = Files.readString(Path.of("src/main/java/com/seibel/distanthorizons/core/render/glObject/texture/DhColorTexture.java"));
		String depth = Files.readString(Path.of("src/main/java/com/seibel/distanthorizons/core/render/glObject/texture/DHDepthTexture.java"));
		String framebuffer = Files.readString(Path.of("src/main/java/com/seibel/distanthorizons/core/render/glObject/texture/DhFramebuffer.java"));
		String shader = Files.readString(Path.of("src/main/java/com/seibel/distanthorizons/core/render/glObject/shader/Shader.java"));
		String shaderProgram = Files.readString(Path.of("src/main/java/com/seibel/distanthorizons/core/render/glObject/shader/ShaderProgram.java"));
		String vertexAttribute = Files.readString(Path.of("src/main/java/com/seibel/distanthorizons/core/render/glObject/vertexAttribute/AbstractVertexAttribute.java"));
		String buffer = Files.readString(Path.of("src/main/java/com/seibel/distanthorizons/core/render/glObject/buffer/GLBuffer.java"));
		String state = Files.readString(Path.of("src/main/java/com/seibel/distanthorizons/core/render/glObject/GLState.java"));
		assertTrue(color.contains("Java Distant Horizons color textures are unavailable while Rust owns whole-frame presentation"));
		assertTrue(depth.contains("Java Distant Horizons depth textures are unavailable while Rust owns whole-frame presentation"));
		assertTrue(framebuffer.contains("Java Distant Horizons framebuffers are unavailable while Rust owns whole-frame presentation"));
		assertTrue(color.contains("isVulkanBackendSelected()") && depth.contains("isVulkanBackendSelected()"));
		assertTrue(framebuffer.contains("isVulkanBackendSelected()"),
			"DH legacy GPU object constructors must reject selected Vulkan before Java handles are created");
		assertTrue(shader.contains("Java Distant Horizons shaders are unavailable while Rust owns whole-frame presentation"));
		assertTrue(shaderProgram.contains("Java Distant Horizons shader programs are unavailable while Rust owns whole-frame presentation"));
		assertTrue(vertexAttribute.contains("Java Distant Horizons vertex arrays are unavailable while Rust owns whole-frame presentation"),
			"DH shader and vertex-array constructors must not create Java GPU objects on selected Vulkan");
		assertTrue(buffer.contains("Java Distant Horizons buffers are unavailable while Rust owns whole-frame presentation"));
		assertTrue(state.contains("Java Distant Horizons GL state is unavailable while Rust owns whole-frame presentation"),
			"DH compatibility state snapshots must not borrow Java GPU state on selected Vulkan");
	}

	@Test
	void selectedVulkanIrisPassesCannotFallThroughToJavaRendering() throws Exception {
		String horizon = Files.readString(Path.of("src/main/java/net/irisshaders/iris/pathways/HorizonRenderer.java"));
		assertTrue(horizon.contains("Java Iris Vulkan horizon resources are unavailable until the Rust whole-frame route is admitted"));
		assertTrue(horizon.contains("Java Iris Vulkan horizon rendering is unavailable until the Rust whole-frame route is admitted"));

		String centerDepth = Files.readString(Path.of("src/main/java/net/irisshaders/iris/pathways/CenterDepthSampler.java"));
		assertTrue(centerDepth.contains("Java Iris Vulkan center-depth resources are unavailable until the Rust whole-frame route is admitted"));
		assertTrue(centerDepth.contains("Java Iris Vulkan center-depth sampling is unavailable until the Rust whole-frame route is admitted"));

		String finalPass = Files.readString(Path.of("src/main/java/net/irisshaders/iris/pipeline/FinalPassRenderer.java"));
		assertTrue(finalPass.contains("Java Iris Vulkan final-pass rendering is unavailable until the Rust whole-frame route is admitted"));
		String programCreator = Files.readString(Path.of("src/main/java/net/irisshaders/iris/gl/shader/ProgramCreator.java"));
		String shaderCreator = Files.readString(Path.of("src/main/java/net/irisshaders/iris/pipeline/programs/ShaderCreator.java"));
		assertTrue(programCreator.contains("Java Iris shader-program creation is unavailable on the Rust Vulkan route")
			&& shaderCreator.contains("Java Iris shader creation is unavailable on the Rust Vulkan route"),
			"selected Vulkan must reject direct Iris shader creation before Java program handles are allocated");
		String renderTargets = Files.readString(Path.of("src/main/java/net/irisshaders/iris/targets/RenderTargets.java"));
		assertTrue(renderTargets.contains("Java Iris render-target resources are unavailable on the Rust Vulkan route"),
			"Iris render-target snapshot resources must fail closed before Java textures are allocated");
		String clearPass = Files.readString(Path.of("src/main/java/net/irisshaders/iris/targets/ClearPass.java"));
		String imageClearPass = Files.readString(Path.of("src/main/java/net/irisshaders/iris/gl/image/ImageClearPass.java"));
		assertTrue(clearPass.contains("Java Iris clear passes are unavailable on the Rust Vulkan route")
			&& imageClearPass.contains("Java Iris image clear passes are unavailable on the Rust Vulkan route"),
			"Iris Java clear-pass overrides must fail closed on selected Vulkan");
	}

	@Test
	void wholeFrameVoxelMapCannotReopenJavaWaypointBuffers() throws Exception {
		String waypoints = Files.readString(Path.of("src/main/java/net/voxelmap/util/WaypointContainer.java"));
		assertTrue(waypoints.contains("Java VoxelMap waypoint rendering is unavailable while Rust owns whole-frame presentation"));
		assertTrue(waypoints.contains("Java VoxelMap waypoint beam rendering is unavailable while Rust owns whole-frame presentation"));
		assertTrue(waypoints.contains("Java VoxelMap waypoint label rendering is unavailable while Rust owns whole-frame presentation"));
		assertTrue(waypoints.contains("VulkanicAPI.isVulkanBackendSelected()"),
			"selected Vulkan must not reopen Java VoxelMap waypoint buffers before whole-frame admission");
		String constants = Files.readString(Path.of("src/main/java/net/voxelmap/VoxelConstants.java"));
		assertTrue(constants.contains("Selected Vulkan VoxelMap overlay is unavailable before Rust whole-frame admission")
			&& constants.contains("Selected Vulkan VoxelMap waypoint rendering is unavailable before Rust whole-frame admission"),
			"selected Vulkan VoxelMap entrypoints must fail closed before semantic admission");
	}

	@Test
	void wholeFrameVoxelMapEntityImageRendererCannotAllocateJavaFbo() throws Exception {
		String manager = Files.readString(Path.of("src/main/java/net/voxelmap/entityrender/EntityMapImageManager.java"));
		int constructor = manager.indexOf("public EntityMapImageManager()");
		int guard = manager.indexOf("Java VoxelMap entity-image rendering is unavailable while Rust owns whole-frame presentation", constructor);
		int allocation = manager.indexOf("VulkanicAPI.createTexture(\"voxelmap-radarfbotexture\"", constructor);
		assertTrue(constructor >= 0 && guard > constructor && allocation > guard,
			"VoxelMap entity-image Java FBO allocation must be unavailable under Rust whole-frame ownership");
	}

	@Test
	void wholeFrameVoxelMapDoesNotConstructJavaOffscreenRenderer() throws Exception {
		String map = Files.readString(Path.of("src/main/java/net/voxelmap/Map.java"));
		int resource = map.indexOf("VulkanicAPI.createTexture(\"voxelmap-fbotexture\"");
		int guard = map.lastIndexOf("!RustGalVulkanWholeFrameMode.enabled()", resource);
		assertTrue(resource >= 0 && guard >= 0 && guard < resource,
			"Rust whole-frame VoxelMap construction must not allocate the legacy Java offscreen target");
		assertTrue(map.contains("Java VoxelMap minimap rendering is unavailable while Rust owns whole-frame presentation"),
			"VoxelMap's legacy Java minimap entry must fail closed after Rust presentation ownership transfers");
		int drawMinimap = map.indexOf("public void drawMinimap");
		assertTrue(drawMinimap >= 0 && map.indexOf("isVulkanBackendSelected()", drawMinimap) > drawMinimap,
			"selected Vulkan must not enter VoxelMap's Java minimap renderer before Rust admission");
		String cachedRegion = Files.readString(Path.of("src/main/java/net/voxelmap/persistent/CachedRegion.java"));
		String image = Files.readString(Path.of("src/main/java/net/voxelmap/persistent/CompressibleGLBufferedImage.java"));
		assertTrue(cachedRegion.contains("Java VoxelMap persistent-region rendering is unavailable while Rust owns whole-frame presentation")
			&& image.contains("Java VoxelMap persistent-image upload is unavailable while Rust owns whole-frame presentation")
			&& image.contains("isVulkanBackendSelected()"),
			"VoxelMap world-map persistence must not upload Java textures in Rust whole-frame mode");
		String moveable = Files.readString(Path.of("src/main/java/net/voxelmap/util/DynamicMoveableTexture.java"));
		assertTrue(moveable.contains("Java VoxelMap texture handles are unavailable while Rust owns whole-frame presentation")
			&& moveable.contains("isVulkanBackendSelected()"),
			"VoxelMap dynamic texture IDs must not borrow Java GPU handles on selected Vulkan");
		String projection = Files.readString(Path.of("src/main/java/net/voxelmap/util/VoxelMapCachedOrthoProjectionMatrixBuffer.java"));
		assertTrue(projection.contains("Java VoxelMap projection UBO rendering is unavailable while Rust owns whole-frame presentation"),
			"VoxelMap projection UBO construction must fail closed under Rust whole-frame ownership");
	}

	@Test
	void wholeFrameVulkanBackendCannotBorrowIrisTextureUnitState() throws Exception {
		String backend = Files.readString(Path.of("src/main/java/net/vulkanic/backends/vulkan/VulkanBackend.java"));
		String encoder = Files.readString(Path.of("src/main/java/net/vulkanic/backends/vulkan/VulkanNativeCommandEncoder.java"));
		int plan = backend.indexOf("private PipelineResourcePlanner.Plan buildLegacyProgramResourcePlan(");
		int planGuard = backend.indexOf("Legacy Java shader-resource reconstruction is not part of the Rust-owned", plan);
		int resolve = backend.indexOf("VulkanicTextureView resolveLegacySamplerViewForProgram(");
		int resolveGuard = backend.indexOf("borrowed Iris GPU state is", resolve);
		int recover = encoder.indexOf("private GpuTextureView recoverSamplerView(");
		int recoverGuard = encoder.indexOf("never recover bindings from Iris' runtime texture cache", recover);
		assertTrue(plan >= 0 && planGuard > plan && backend.indexOf("isVulkanBackendSelected()", plan) < planGuard,
			"legacy Vulkan resource planning must be fenced whenever Vulkan is selected");
		assertTrue(resolve >= 0 && resolveGuard > resolve && backend.indexOf("isVulkanBackendSelected()", resolve) < resolveGuard,
			"legacy Vulkan sampler resolution must be fenced whenever Vulkan is selected");
		assertTrue(recover >= 0 && recoverGuard > recover, "native encoder sampler recovery must not borrow Iris state");
		assertTrue(backend.contains("Java Vulkan command encoder creation is unavailable while Rust owns whole-frame presentation")
			|| backend.contains("Java Vulkan command encoder creation is unavailable on the selected Rust Vulkan route"),
			"direct Java Vulkan command-encoder construction must fail closed under Rust whole-frame ownership");
		assertTrue(backend.contains("Java Vulkan terrain command encoder creation is unavailable while Rust owns whole-frame presentation")
			|| backend.contains("Java Vulkan terrain command encoder creation is unavailable on the selected Rust Vulkan route"),
			"direct Java Vulkan terrain encoder construction must fail closed under Rust whole-frame ownership");
		assertTrue(backend.contains("private static void rejectJavaWholeFramePass(String operation)")
			&& backend.contains("rejectJavaWholeFramePass(\"createRenderPass\")")
			&& backend.contains("rejectJavaWholeFramePass(\"beginCommandBuffer\")")
			&& backend.contains("rejectJavaWholeFramePass(\"submitCommandBuffer\")")
			&& backend.contains("rejectJavaWholeFramePass(\"beginFrame\")")
			&& backend.contains("rejectJavaWholeFramePass(\"endFrame\")")
			&& backend.contains("rejectJavaWholeFramePass(\"beginRenderPass\")")
			&& backend.contains("rejectJavaWholeFramePass(\"beginRenderPass(framebuffer)\")")
			&& backend.contains("rejectJavaWholeFramePass(\"beginRenderPass(renderTargetDescriptor)\")"),
			"direct Java Vulkan pass/frame lifecycle entry points must fail closed under Rust whole-frame ownership");
	}

	@Test
	void wholeFrameVulkanBackendCannotConstructJavaGpuObjects() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/vulkanic/backends/vulkan/VulkanBackend.java"));
		assertGuarded(source, "public CompiledRenderPipeline precompileRenderPipeline(", "rejectJavaWholeFramePass");
		assertGuarded(source, "public GpuTexture createTexture(", "rejectJavaWholeFramePass");
		assertGuarded(source, "public GpuBuffer createBuffer(", "rejectJavaWholeFramePass");
		assertGuarded(source, "public GpuTextureView createTextureView(", "rejectJavaWholeFramePass");
		assertGuarded(source, "public VulkanicBuffer createManagedBuffer(", "rejectJavaWholeFramePass");
		assertGuarded(source, "public VulkanicTexture createManagedTexture(", "rejectJavaWholeFramePass");
		assertGuarded(source, "public VulkanicTextureView createManagedTextureView(", "rejectJavaWholeFramePass");
		assertGuarded(source, "public int createShader(", "rejectJavaWholeFramePass");
		assertGuarded(source, "public int createShaderProgram(", "rejectJavaWholeFramePass");
		assertGuarded(source, "public void linkProgram(", "rejectJavaWholeFramePass");
		assertGuarded(source, "public PipelineHandle createPipeline(", "rejectJavaWholeFramePass");
		assertGuarded(source, "public net.vulkanic.DescriptorPoolHandle createDescriptorPool(", "rejectJavaWholeFramePass");
		assertGuarded(source, "public net.vulkanic.DescriptorSetHandle allocateDescriptorSet(", "rejectJavaWholeFramePass");
		assertGuarded(source, "public void updateDescriptorSet(", "rejectJavaWholeFramePass");
		assertGuarded(source, "public void bindDescriptorSet(", "rejectJavaWholeFramePass");
		assertGuarded(source, "public void executeGraphicsDraw(", "rejectJavaWholeFramePass");
		assertGuarded(source, "public VulkanicGalExecutionRequest.GraphicsDrawRequest captureGraphicsRequest(", "rejectJavaWholeFramePass");
		assertGuarded(source, "public void executeComputeDispatch(", "rejectJavaWholeFramePass");
		assertGuarded(source, "public VulkanicGalExecutionRequest.ComputeDispatchRequest captureComputeDispatchRequest(", "rejectJavaWholeFramePass");
		assertGuarded(source, "public void destroySync(", "rejectJavaWholeFramePass");
		assertGuarded(source, "public int retrieveQueryObjectInt(", "rejectJavaWholeFramePass");
		assertGuarded(source, "public long retrieveQueryObjectInt64(", "rejectJavaWholeFramePass");
		assertGuarded(source, "public net.vulkanic.PipelineHandle resolvePipelineHandle(", "rejectJavaWholeFramePass");
		assertGuarded(source, "public net.vulkanic.VulkanicUniformLocation resolveUniformLocation(", "rejectJavaWholeFramePass");
		assertGuarded(source, "public String retrieveActiveUniformBlockName(", "rejectJavaWholeFramePass");
		assertGuarded(source, "public List<VulkanicSpirvModule> getLinkedProgramSpirvModules(", "rejectJavaWholeFramePass");
		assertGuarded(source, "public PipelineDescriptor.ResourceLayout getLinkedProgramResourceLayout(", "rejectJavaWholeFramePass");
		assertGuarded(source, "public int getTexParameteri(", "rejectJavaWholeFramePass");
		assertGuarded(source, "public int getTextureLevelParameter(", "rejectJavaWholeFramePass");
		assertGuarded(source, "public int resolveBufferHandle(", "rejectJavaWholeFramePass");
		assertGuarded(source, "public int resolveTextureHandle(", "rejectJavaWholeFramePass");
		assertGuarded(source, "public int resolveFramebufferForTextures(", "rejectJavaWholeFramePass");
		assertGuarded(source, "public int createTextures(", "rejectJavaWholeFramePass");
		assertGuarded(source, "public int createFramebuffer(", "rejectJavaWholeFramePass");
		assertGuarded(source, "public int createVertexArray(", "rejectJavaWholeFramePass");
		assertGuarded(source, "public int createSampler(", "rejectJavaWholeFramePass");
		assertGuarded(source, "public long createFenceSync(", "rejectJavaWholeFramePass");
		int commandGuard = source.indexOf("private static long requireVulkanCommandBufferHandle");
		int commandReject = source.indexOf("rejectJavaWholeFramePass(operation)", commandGuard);
		assertTrue(commandGuard >= 0 && commandReject > commandGuard,
			"all legacy Vulkan command-context entry points must share the whole-frame ownership guard");
		String texture = Files.readString(Path.of("src/main/java/net/vulkanic/backends/vulkan/VulkanGpuTexture.java"));
		int handle = texture.indexOf("public int getGlHandle()");
		int handleGuard = texture.indexOf("RustGalVulkanWholeFrameMode.enabled()", handle);
		assertTrue(handle >= 0 && handleGuard > handle,
			"Java Vulkan texture native-handle access must fail closed while Rust owns whole-frame presentation");
		assertTrue(texture.contains("VulkanicAPI.isVulkanBackendSelected()"),
			"Java Vulkan texture native-handle access must fail closed as soon as Vulkan is selected");
		String buffer = Files.readString(Path.of("src/main/java/net/vulkanic/backends/vulkan/VulkanGpuBuffer.java"));
		assertFalse(buffer.contains("public int getHandle()"),
			"Java Vulkan buffer handles must remain backend-internal rather than public cross-boundary state");
	}

	@Test
	void semanticStartupDeviceBoundsAggregateCpuBuffers() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/vulkanic/backends/vulkan/VulkanWholeFrameSemanticGpuDevice.java"));
		assertTrue(source.contains("MAX_BUFFER_BYTES = 256L * 1024L * 1024L"),
			"semantic startup buffers must have an aggregate CPU-memory bound");
		assertTrue(source.contains("allocatedBufferBytes > MAX_BUFFER_BYTES - size"),
			"semantic startup allocation must enforce the aggregate bound before allocation");
		assertTrue(source.contains("this.release.run()"),
			"semantic startup buffer close must release its aggregate budget");
	}

	@Test
	void dynamicWorldTextureAssetsHaveAggregateResidencyBound() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		assertTrue(source.contains("MAX_DYNAMIC_WORLD_ASSET_BYTES_TOTAL = 256L * 1024L * 1024L"),
			"copied dynamic world textures must have an aggregate encoded-payload bound");
		assertTrue(source.contains("DYNAMIC_WORLD_ASSET_BYTES.clear()"),
			"dynamic texture residency accounting must reset with world asset reloads");
		assertTrue(source.contains("retainedBytes - previousBytes > MAX_DYNAMIC_WORLD_ASSET_BYTES_TOTAL - payload.length"),
			"dynamic texture replacement must enforce the aggregate bound before publication");
	}

	@Test
	void semanticWorldBackgroundCannotSilentlyUseDiagnosticFallback() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/vulkanic/gui/RustGalFrameCoordinator.java"));
		int result = source.indexOf("wholeFrameResult = bridge.submitWholeFrameWithAffineGuiAndWorldTextAndFirstPerson(");
		int guard = source.indexOf("worldBackgroundDiagnosticFallbackCount()", result);
		assertTrue(result >= 0 && guard > result,
			"semantic world frames must reject native diagnostic-background fallback counts");
	}

	@Test
	void wholeFrameTerrainSourceRetainsFailureReasonInsteadOfSwallowingIt() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalWholeFrameTerrainSource.java"));
		assertTrue(source.contains("lastWholeFrameTerrainFailure"),
			"whole-frame terrain source must retain a bounded failure reason");
		assertTrue(source.contains("recordTerrainFailure(\"prepare\""),
			"terrain snapshot failures must be diagnosed at the prepare boundary");
		assertTrue(source.contains("recordTerrainFailure(\"unwrap\""),
			"terrain worker failures must be diagnosed at completion unwrap");
	}

	@Test
	void wholeFrameJavaTextureTeardownCannotMutateCompatibilityState() throws Exception {
		String api = Files.readString(Path.of("src/main/java/net/vulkanic/VulkanicAPI.java"));
		int delete = api.indexOf("public static void deleteTexture(");
		int guard = api.indexOf("rejectJavaVulkanWholeFrameOperation", delete);
		assertTrue(delete >= 0 && guard > delete,
			"Java texture teardown must be rejected before touching compatibility state");
		assertApiDeletionGuarded(api, "public static void deleteBuffer(");
		assertApiDeletionGuarded(api, "public static void deleteFramebuffer(");
		assertApiDeletionGuarded(api, "public static void deleteProgram(");
		assertApiDeletionGuarded(api, "public static void deleteVertexArrays(");
	}

	private static void assertApiDeletionGuarded(String source, String method) {
		int start = source.indexOf(method);
		int guard = source.indexOf("rejectJavaVulkanWholeFrameOperation", start);
		assertTrue(start >= 0 && guard > start, method + " must reject before compatibility-state deletion");
	}

	@Test
	void sodiumFogCullingChecksVulkanOwnershipBeforeIrisPackState() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/sodium/client/render/chunk/RenderSectionManager.java"));
		int method = source.indexOf("private float getSearchDistance(");
		int vulkan = source.indexOf("boolean useFogOcclusion = VulkanicAPI.isVulkanBackendSelected()", method);
		int iris = source.indexOf("net.irisshaders.iris.Iris.getCurrentPack().isPresent()", vulkan);
		assertTrue(method >= 0 && vulkan > method && iris > vulkan,
			"Sodium fog culling must short-circuit Vulkan ownership before consulting Iris pack state");
	}

	@Test
	void wholeFramePbrSetupCannotReopenJavaTextureState() throws Exception {
		String cache = Files.readString(Path.of("src/main/java/net/irisshaders/iris/pbr/TextureInfoCache.java"));
		String manager = Files.readString(Path.of("src/main/java/net/irisshaders/iris/pbr/texture/PBRTextureManager.java"));
		int image = cache.indexOf("public void onTexImage2D(");
		int imageGuard = cache.indexOf("RustGalVulkanWholeFrameMode.enabled()", image);
		int holder = manager.indexOf("public PBRTextureHolder getOrLoadHolder(");
		int holderGuard = manager.indexOf("RustGalVulkanWholeFrameMode.enabled()", holder);
		assertTrue(image >= 0 && imageGuard > image
			&& cache.contains("Java Iris texture metadata is unavailable on the Rust Vulkan route"),
			"PBR metadata callbacks must be inert on the Rust Vulkan route");
		assertTrue(holder >= 0 && holderGuard > holder, "PBR holder loading must not consult Java texture state in Rust whole-frame mode");
	}

	@Test
	void wholeFrameIrisJavaTextureObjectsStayUnavailable() throws Exception {
		String custom = Files.readString(Path.of("src/main/java/net/irisshaders/iris/targets/backed/NativeImageBackedCustomTexture.java"));
		String noise = Files.readString(Path.of("src/main/java/net/irisshaders/iris/targets/backed/NativeImageBackedNoiseTexture.java"));
		int customConstructor = custom.indexOf("public NativeImageBackedCustomTexture(");
		int customConstructorGuard = custom.indexOf("RustGalVulkanWholeFrameMode.enabled()", customConstructor);
		int customUpload = custom.indexOf("public void upload()");
		int customUploadGuard = custom.indexOf("RustGalVulkanWholeFrameMode.enabled()", customUpload);
		int noiseConstructor = noise.indexOf("public NativeImageBackedNoiseTexture(");
		int noiseConstructorGuard = noise.indexOf("RustGalVulkanWholeFrameMode.enabled()", noiseConstructor);
		int noiseUpload = noise.indexOf("public void upload()");
		int noiseUploadGuard = noise.indexOf("RustGalVulkanWholeFrameMode.enabled()", noiseUpload);
		assertTrue(customConstructor >= 0 && customConstructorGuard > customConstructor,
			"Iris custom textures must remain unavailable while Rust owns whole-frame presentation");
		assertTrue(customUpload >= 0 && customUploadGuard > customUpload,
			"Iris custom texture uploads must remain unavailable while Rust owns whole-frame presentation");
		assertTrue(noiseConstructor >= 0 && noiseConstructorGuard > noiseConstructor,
			"Iris noise textures must remain unavailable while Rust owns whole-frame presentation");
		assertTrue(noiseUpload >= 0 && noiseUploadGuard > noiseUpload,
			"Iris noise texture uploads must remain unavailable while Rust owns whole-frame presentation");
	}

	@Test
	void rustHandoffClosesAndBlocksIrisPbrCompatibilityTextures() throws Exception {
		String manager = Files.readString(Path.of("src/main/java/net/irisshaders/iris/pbr/texture/PBRTextureManager.java"));
		String api = Files.readString(Path.of("src/main/java/net/vulkanic/VulkanicAPI.java"));
		int init = manager.indexOf("public void init()");
		int initGuard = manager.indexOf("VulkanicAPI.isVulkanBackendSelected()", init);
		assertTrue(init >= 0 && initGuard > init && manager.indexOf("close();", initGuard) > initGuard,
			"Iris PBR compatibility defaults must not be initialized on the Rust Vulkan route");
		int handoff = api.indexOf("public static void ensureRustSemanticRoute()");
		assertTrue(handoff >= 0,
			"the central Rust ownership handoff must remain present");
		int nextMethod = api.indexOf("\n\t}\n\n    public static", handoff);
		assertTrue(nextMethod > handoff && !api.substring(handoff, nextMethod).contains("PBRTextureManager.INSTANCE.close()"),
			"the central Rust ownership handoff must not initialize Iris PBR holders");
	}

	@Test
	void fishingHookUsesSemanticBillboardAndLineRoutesOnVulkan() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/entity/FishingHookRenderer.java"));
		String routePolicy = Files.readString(Path.of("src/main/java/net/vulkanic/world/WorldRenderRoutePolicy.java"));
		assertTrue(source.contains("submitTexturedQuad") && source.contains("submitLineSegments"),
			"fishing-hook rendering must expose both primitive families through semantic collector calls");
		assertTrue(source.contains("currentFishingLineRoute().usesRustWholeFrameVulkan()")
			&& source.contains("currentTexturedBillboardRoute().usesRustWholeFrameVulkan()"),
			"fishing-hook primitive admission must be explicit per family");
		assertTrue(source.contains("Rust whole-frame fishing-hook route rejected semantic billboard")
			&& source.contains("Rust whole-frame fishing-hook route rejected semantic line segments"),
			"fishing-hook Vulkan failures must remain fail-closed rather than invoke Java callbacks");
		assertTrue(source.contains("Rust whole-frame fishing-hook route rejected non-finite line endpoints")
			&& source.contains("for (float endpoint : lineEndpoints)"),
			"fishing-hook Rust line admission must reject non-finite copied endpoints before ABI submission");
		for (String method : new String[] {
			"currentTexturedBillboardRoute()",
			"currentFishingLineRoute()",
			"currentDebugLineRoute()",
			"currentProceduralQuadRoute()"
		}) {
			int route = routePolicy.indexOf("public static Route " + method);
			assertTrue(route >= 0, "missing route policy " + method);
			String body = routePolicy.substring(route, routePolicy.indexOf("\n\t}\n", route));
			assertTrue(body.contains("RustGalVulkanWholeFrameMode.enabled()"),
				method + " must remain Rust-owned during the pre-selection whole-frame handoff");
		}
	}

	@Test
	void firstPersonHandsHaveSeparateRustSemanticAndJavaCompatibilityEntrypoints() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/ItemInHandRenderer.java"));
		int javaEntry = source.indexOf("public void renderHandsWithItems(");
		int javaGuard = source.indexOf("Java first-person hand rendering is unavailable", javaEntry);
		int rustEntry = source.indexOf("public void renderRustVulkanHands(");
		int semanticSeed = source.indexOf("beginFirstPersonGuiCapture()", rustEntry);
		assertTrue(javaEntry >= 0 && javaGuard > javaEntry,
			"the legacy first-person entrypoint must fail closed when Vulkan is selected");
		assertTrue(rustEntry >= 0 && semanticSeed > rustEntry
			&& source.indexOf("collectFirstPersonTextSemantics", rustEntry) > semanticSeed,
			"Rust first-person rendering must collect semantic hand/item/text data through its dedicated entrypoint");
	}

	@Test
	void guiRendererPrivateJavaPassPathFailsClosedUnderRustPresentation() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/render/GuiRenderer.java"));
		int draw = source.indexOf("private void draw(GpuBufferSlice gpuBufferSlice)");
		int guard = source.indexOf("Java GUI render-pass execution is unavailable", draw);
		int pass = source.indexOf("VulkanicAPI.createRenderPass", draw);
		assertTrue(draw >= 0 && guard > draw && pass > guard,
			"GUI's private Java pass executor must reject Rust-owned Vulkan before creating any pass");
	}

	@Test
	void wholeFrameIrisUniformHelpersDoNotQueryJavaTextureBindings() throws Exception {
		String uniforms = Files.readString(Path.of("src/main/java/net/irisshaders/iris/uniforms/CommonUniforms.java"));
		String shader = Files.readString(Path.of("src/main/java/net/irisshaders/iris/pipeline/programs/ExtendedShader.java"));
		int atlas = uniforms.indexOf("uniforms.uniform2i(\"atlasSize\"");
		int atlasGuard = uniforms.indexOf("RustGalVulkanWholeFrameMode.enabled()", atlas);
		int gtexture = uniforms.indexOf("uniforms.uniform2i(\"gtextureSize\"");
		int gtextureGuard = uniforms.indexOf("RustGalVulkanWholeFrameMode.enabled()", gtexture);
		int swizzle = shader.indexOf("if (intensitySwizzle");
		int swizzleGuard = shader.indexOf("RustGalVulkanWholeFrameMode.enabled()", swizzle);
		assertTrue(atlas >= 0 && atlasGuard > atlas, "atlas-size uniform must not query Iris texture state in whole-frame mode");
		assertTrue(gtexture >= 0 && gtextureGuard > gtexture, "gtexture-size uniform must not query Iris texture state in whole-frame mode");
		assertTrue(swizzle >= 0 && swizzleGuard > swizzle, "Iris intensity swizzle must not mutate Java texture state in whole-frame mode");
		assertTrue(uniforms.contains("VulkanicAPI.isVulkanBackendSelected()"),
			"selected Vulkan Iris uniforms must not query Java texture metadata before whole-frame admission");
	}

	@Test
	void wholeFrameIrisSamplerCompatibilityPathsStayClosed() throws Exception {
		String samplers = Files.readString(Path.of("src/main/java/net/irisshaders/iris/gl/program/ProgramSamplers.java"));
		String custom = Files.readString(Path.of("src/main/java/net/irisshaders/iris/pipeline/CustomTextureManager.java"));
		String tracker = Files.readString(Path.of("src/main/java/net/irisshaders/iris/pbr/TextureTracker.java"));
		String depthCopy = Files.readString(Path.of("src/main/java/net/irisshaders/iris/gl/texture/DepthCopyStrategy.java"));
		String dhFramebuffer = Files.readString(Path.of("src/main/java/net/irisshaders/iris/compat/dh/DhFrameBufferWrapper.java"));
		int update = samplers.indexOf("public void update()");
		int updateGuard = samplers.indexOf("RustGalVulkanWholeFrameMode.enabled()", update);
		int bind = samplers.indexOf("public void bindToRenderPass(");
		int bindGuard = samplers.indexOf("RustGalVulkanWholeFrameMode.enabled()", bind);
		assertTrue(update >= 0 && updateGuard > update, "Iris sampler updates must fail closed under Rust whole-frame ownership");
		assertTrue(bind >= 0 && bindGuard > bind, "Iris render-pass sampler binding must fail closed under Rust whole-frame ownership");
		assertTrue(custom.contains("!net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()"),
			"Iris custom texture wrappers must not bind Java texture units in whole-frame mode");
		int setTexture = tracker.indexOf("public void onSetShaderTexture(");
		int trackerGuard = tracker.indexOf("RustGalVulkanWholeFrameMode.enabled()", setTexture);
		assertTrue(setTexture >= 0 && trackerGuard > setTexture,
			"Iris texture tracking must not publish Java shader bindings in whole-frame mode");
		int textureView = tracker.indexOf("public GpuTextureView getTextureView(");
		int textureViewGuard = tracker.indexOf("RustGalVulkanWholeFrameMode.enabled()", textureView);
		int shaderTexture = tracker.indexOf("public GpuTextureView getShaderTexture(");
		int shaderTextureGuard = tracker.indexOf("RustGalVulkanWholeFrameMode.enabled()", shaderTexture);
		assertTrue(textureView >= 0 && textureViewGuard > textureView,
			"Iris texture-view lookup must not expose Java GPU views in whole-frame mode");
		assertTrue(shaderTexture >= 0 && shaderTextureGuard > shaderTexture,
			"Iris shader-texture lookup must not expose Java GPU views in whole-frame mode");
		assertTrue(tracker.contains("VulkanicAPI.isVulkanBackendSelected()"),
			"selected Vulkan must not borrow Iris Java texture state before whole-frame admission");
		assertTrue(depthCopy.contains("VulkanicAPI.isVulkanBackendSelected()"),
			"selected Vulkan must not select a Java Iris depth-copy strategy");
		assertTrue(dhFramebuffer.contains("VulkanicAPI.isVulkanBackendSelected()"),
			"selected Vulkan must not bind or mutate the Java Iris DH framebuffer wrapper");
	}

	@Test
	void wholeFrameIrisTextureUnitStateCannotBorrowJavaBindings() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/irisshaders/iris/gl/IrisRenderSystem.java"));
		int getBinding = source.indexOf("public static int getTextureBinding(");
		int getGuard = source.indexOf("RustGalVulkanWholeFrameMode.enabled()", getBinding);
		int setBinding = source.indexOf("public static void setTextureBinding(");
		int setGuard = source.indexOf("RustGalVulkanWholeFrameMode.enabled()", setBinding);
		int bindUnit = source.indexOf("public static void bindTextureToUnit(int target");
		int bindGuard = source.indexOf("RustGalVulkanWholeFrameMode.enabled()", bindUnit);
		int setup = source.indexOf("public static void bindTextureForSetup(int glType");
		int setupGuard = source.indexOf("RustGalVulkanWholeFrameMode.enabled()", setup);
		int createFramebuffer = source.indexOf("public static int createFramebuffer()");
		int createFramebufferGuard = source.indexOf("rejectJavaGpuObjectCreation", createFramebuffer);
		int createTexture = source.indexOf("public static int createTexture(int target)");
		int createTextureGuard = source.indexOf("rejectJavaGpuObjectCreation", createTexture);
		int createBuffers = source.indexOf("public static int createBuffers()");
		int createBuffersGuard = source.indexOf("rejectJavaGpuObjectCreation", createBuffers);
		assertTrue(getBinding >= 0 && getGuard > getBinding,
			"Iris texture binding reads must remain inert while Rust owns whole-frame presentation");
		assertTrue(setBinding >= 0 && setGuard > setBinding,
			"Iris texture binding writes must remain inert while Rust owns whole-frame presentation");
		assertTrue(bindUnit >= 0 && bindGuard > bindUnit,
			"Iris texture-unit binds must not reach Java GPU state while Rust owns whole-frame presentation");
		assertTrue(setup >= 0 && setupGuard > setup,
			"Iris texture setup must not reach Java GPU state while Rust owns whole-frame presentation");
		assertTrue(createFramebuffer >= 0 && createFramebufferGuard > createFramebuffer,
			"Iris framebuffer creation must remain unavailable while Rust owns whole-frame presentation");
		assertTrue(createTexture >= 0 && createTextureGuard > createTexture,
			"Iris texture creation must remain unavailable while Rust owns whole-frame presentation");
		assertTrue(createBuffers >= 0 && createBuffersGuard > createBuffers,
			"Iris buffer creation must remain unavailable while Rust owns whole-frame presentation");
	}

	@Test
	void wholeFrameIrisGpuMutatorsFailClosed() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/irisshaders/iris/gl/IrisRenderSystem.java"));
		assertGuarded(source, "public static void destroySampler(", "rejectJavaGpuMutation");
		assertGuarded(source, "public static void deleteBuffers(", "rejectJavaGpuMutation");
		assertGuarded(source, "public static void dispatchComputeIndirect(", "rejectJavaGpuMutation");
		assertGuarded(source, "public static void copyImageSubData(", "rejectJavaGpuMutation");
		assertGuarded(source, "public static void deleteTextureId(", "rejectJavaGpuMutation");
		assertGuarded(source, "public static void clearColor(", "rejectJavaGpuMutation");
		assertGuarded(source, "public static void blitFramebuffer(", "rejectJavaGpuMutation");
	}

	private static void assertGuarded(String source, String method, String guard) {
		int start = source.indexOf(method);
		int next = source.indexOf("\n\tpublic ", start + method.length());
		int guardAt = source.indexOf(guard, start);
		assertTrue(start >= 0 && guardAt > start && (next < 0 || guardAt < next),
			method + " must reject Java GPU ownership mutations in whole-frame mode");
	}

	@Test
	void wholeFrameItemModelExtractionDoesNotPublishIrisDisplayContext() throws Exception {
		String resolver = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/item/ItemModelResolver.java"));
		int context = resolver.indexOf("ItemContextState");
		int guard = resolver.lastIndexOf("RustGalVulkanWholeFrameMode.enabled()", context);
		assertTrue(context >= 0 && guard >= 0 && guard < context
			&& resolver.contains("VulkanicAPI.isVulkanBackendSelected()"),
			"semantic item extraction must not publish Iris display-item runtime state");
	}

	@Test
	void wholeFrameFireworkParticlesDoNotReadIrisLayerOverrides() throws Exception {
		String fireworks = Files.readString(Path.of("src/main/java/net/minecraft/client/particle/FireworkParticles.java"));
		int iris = fireworks.indexOf("net.irisshaders.iris.Iris.IS_FOOL");
		int guard = fireworks.lastIndexOf("RustGalVulkanWholeFrameMode.enabled()", iris);
		assertTrue(iris >= 0 && guard >= 0 && guard < iris,
			"Rust particle extraction must not consult Iris firework layer overrides");
	}

	@Test
	void wholeFrameCloudOptionDoesNotQueryIrisPipeline() throws Exception {
		String options = Files.readString(Path.of("src/main/java/net/minecraft/client/Options.java"));
		int method = options.indexOf("public CloudStatus getCloudsType()");
		int iris = options.indexOf("Iris.getPipelineManager()", method);
		int guard = options.indexOf("RustGalVulkanWholeFrameMode.enabled()", method);
		int selectedGuard = options.indexOf("VulkanicAPI.isVulkanBackendSelected()", method);
		assertTrue(method >= 0 && iris > method && guard > method && guard < iris
			&& selectedGuard > method && selectedGuard < iris,
			"Rust cloud extraction must use the copied option without querying Iris pipeline state");
	}

	@Test
	void wholeFrameGlyphSelectionDoesNotReadIrisImmediateState() throws Exception {
		String glyphs = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/font/GlyphRenderTypes.java"));
		int immediate = glyphs.indexOf("ImmediateState.isRenderingBEs");
		int guard = glyphs.lastIndexOf("RustGalVulkanWholeFrameMode.enabled()", immediate);
		assertTrue(immediate >= 0 && guard >= 0 && guard < immediate,
			"Rust semantic glyph selection must not consult Iris ImmediateState");
	}

	@Test
	void resourceAtlasPbrFilteringDoesNotDependOnIrisRuntimeClasses() throws Exception {
		String lister = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/texture/atlas/sources/DirectoryLister.java"));
		assertTrue(lister.contains("removePbrSuffix(resourceLocation.getPath())"));
		assertFalse(lister.contains("net.irisshaders.iris.pbr.texture.PBRType"),
			"resource-pack atlas filtering must remain backend-neutral and Rust-owned");
	}

	@Test
	void wholeFrameIrisRenderTargetContractCannotUseFramebufferFallback() throws Exception {
		String contract = Files.readString(Path.of("src/main/java/net/irisshaders/iris/pipeline/IrisVulkanRenderTargetContract.java"));
		int recorded = contract.indexOf("boolean vulkanRecordedPass");
		int fallback = contract.indexOf("Iris framebuffer-compatible render-target fallback is unavailable", recorded);
		assertTrue(recorded >= 0 && fallback > recorded);
		String body = contract.substring(recorded, fallback);
		assertTrue(body.contains("RustGalVulkanWholeFrameMode.enabled()"));
		assertTrue(body.contains("!descriptorPathEnabled || !vulkanRecordedPass"));
		assertTrue(contract.contains("Iris render-target descriptor is incompatible with Rust whole-frame presentation"));
		assertTrue(contract.contains("Iris Java Vulkan pipeline construction is unavailable; Rust owns the selected Vulkan route"));
		assertTrue(contract.contains("Iris Java Vulkan render-pass creation is unavailable; Rust owns the selected Vulkan route"));
		assertTrue(contract.contains("if (VulkanicAPI.isVulkanBackendSelected())"),
			"Iris target selection must fence Java Vulkan even before whole-frame shell activation");
	}

	@Test
	void wholeFrameCannotReachJavaVulkanPresenterInternals() throws Exception {
		String backend = Files.readString(Path.of("src/main/java/net/vulkanic/backends/vulkan/VulkanBackend.java"));
		String encoder = Files.readString(Path.of("src/main/java/net/vulkanic/backends/vulkan/VulkanNativeCommandEncoder.java"));
		int backendPresent = backend.indexOf("public void presentTextureToScreen(CommandContext ctx, GpuTextureView textureView)");
		int backendGuard = backend.indexOf("Java Vulkan backend presentation is unavailable while Rust owns whole-frame presentation", backendPresent);
		int encoderPresent = encoder.indexOf("public void presentTexture(GpuTextureView textureView)");
		int encoderGuard = encoder.indexOf("Java Vulkan command-encoder presentation is unavailable while Rust owns the selected Vulkan presentation route", encoderPresent);
		assertTrue(backendPresent >= 0 && backendGuard > backendPresent, "Vulkan backend presenter must fail closed in Rust whole-frame mode");
		assertTrue(encoderPresent >= 0 && encoderGuard > encoderPresent, "Vulkan native encoder presenter must fail closed in Rust whole-frame mode");
		assertTrue(encoder.substring(encoderPresent, encoderGuard).contains("isVulkanBackendSelected()"),
			"Vulkan native encoder presenter must fail closed as soon as Vulkan is selected, before whole-frame activation");
	}

	@Test
	void wholeFrameJavaVulkanClearBuffersCannotReopenRendering() throws Exception {
		String backend = Files.readString(Path.of("src/main/java/net/vulkanic/backends/vulkan/VulkanBackend.java"));
		assertTrue(backend.contains("Java Vulkan clearBuffers is unavailable while Rust owns whole-frame presentation"));
	}

	@Test
	void wholeFrameJavaVulkanShaderBindingCannotMutateCompatibilityState() throws Exception {
		String backend = Files.readString(Path.of("src/main/java/net/vulkanic/backends/vulkan/VulkanBackend.java"));
		int method = backend.indexOf("public void bindShaderProgram(CommandContext ctx, int programId)");
		assertTrue(method >= 0);
		String body = backend.substring(method, Math.min(backend.length(), method + 320));
		assertTrue(body.contains("rejectJavaWholeFramePass(\"bindShaderProgram\")"));
	}

	@Test
	void wholeFrameJavaVulkanCapabilityStateCannotMutateCompatibilityState() throws Exception {
		String backend = Files.readString(Path.of("src/main/java/net/vulkanic/backends/vulkan/VulkanBackend.java"));
		int method = backend.indexOf("public void setCapabilityEnabled(CommandContext ctx, int cap, boolean enabled)");
		assertTrue(method >= 0);
		String body = backend.substring(method, Math.min(backend.length(), method + 320));
		assertTrue(body.contains("rejectJavaWholeFramePass(\"setCapabilityEnabled\")"));
	}

	@Test
	void wholeFrameJavaVulkanIndexedBlendStateCannotMutateCompatibilityState() throws Exception {
		String backend = Files.readString(Path.of("src/main/java/net/vulkanic/backends/vulkan/VulkanBackend.java"));
		int method = backend.indexOf("public void setIndexedEnabled(CommandContext ctx, int capability, int index, boolean enabled)");
		assertTrue(method >= 0);
		String body = backend.substring(method, Math.min(backend.length(), method + 260));
		assertTrue(body.contains("rejectJavaWholeFramePass(\"setIndexedEnabled\")"));
	}

	@Test
	void wholeFrameJavaVulkanFixedFunctionStateCannotMutateCompatibilityState() throws Exception {
		String backend = Files.readString(Path.of("src/main/java/net/vulkanic/backends/vulkan/VulkanBackend.java"));
		String[] operations = {
			"setDynamicViewport", "setDynamicScissor", "setBlendEnabled", "setBlendFunction",
			"setBlendEquation", "setBlendEquationSeparate", "setDepthTest", "setDepthWriteMask",
			"setColorMask", "setCullFaceMode", "setPolygonMode", "setPolygonOffset"
		};
		for (String operation : operations) {
			int method = backend.indexOf("public void " + operation + "(");
			assertTrue(method >= 0, "missing Java Vulkan fixed-function method: " + operation);
			String body = backend.substring(method, Math.min(backend.length(), method + 420));
			assertTrue(body.contains("rejectJavaWholeFramePass(\"" + operation + "\")"),
				"Java Vulkan fixed-function state must fail closed: " + operation);
		}
	}

	@Test
	void wholeFrameJavaVulkanDebugControlsCannotMutateCompatibilityState() throws Exception {
		String backend = Files.readString(Path.of("src/main/java/net/vulkanic/backends/vulkan/VulkanBackend.java"));
		String[] operations = {"debugMessageControl", "debugMessageControlARB", "debugMessageControlKHR", "debugMessageEnableAMD"};
		for (String operation : operations) {
			int method = backend.indexOf("public void " + operation + "(");
			assertTrue(method >= 0, "missing Java Vulkan debug-control method: " + operation);
			String body = backend.substring(method, Math.min(backend.length(), method + 320));
			assertTrue(body.contains("rejectJavaWholeFramePass(\"" + operation + "\")"),
				"Java Vulkan debug controls must fail closed under Rust whole-frame ownership: " + operation);
		}
	}

	@Test
	void wholeFrameJavaVulkanTextureAndSamplerBindingsCannotMutateCompatibilityState() throws Exception {
		String backend = Files.readString(Path.of("src/main/java/net/vulkanic/backends/vulkan/VulkanBackend.java"));
		String[][] operations = {
			{"setActiveTextureUnit", "public void setActiveTextureUnit(CommandContext ctx, int unit)"},
			{"bindTexture", "public void bindTexture(CommandContext ctx, int target, int textureId)"},
			{"bindTextureUnit", "public void bindTextureUnit(CommandContext ctx, int unit, int texture)"},
			{"deleteSampler", "public void deleteSampler(CommandContext ctx, int sampler)"},
			{"bindSampler", "public void bindSampler(CommandContext ctx, int unit, int sampler)"},
			{"bindSamplers", "public void bindSamplers(CommandContext ctx, int first, int[] samplers)"},
			{"setSamplerParameteri", "public void setSamplerParameteri(CommandContext ctx, int sampler, int pname, int param)"},
			{"setSamplerParameterf", "public void setSamplerParameterf(CommandContext ctx, int sampler, int pname, float param)"},
			{"setSamplerParameteriv", "public void setSamplerParameteriv(CommandContext ctx, int sampler, int pname, int[] params)"}
		};
		for (String[] operation : operations) {
			int method = backend.indexOf(operation[1]);
			assertTrue(method >= 0, "missing Java Vulkan texture/sampler method: " + operation[0]);
			String body = backend.substring(method, Math.min(backend.length(), method + 420));
			assertTrue(body.contains("rejectJavaWholeFramePass(\"" + operation[0] + "\")"),
				"Java Vulkan texture/sampler state must fail closed: " + operation[0]);
		}
	}

	@Test
	void wholeFrameJavaVulkanVertexInputStateCannotMutateCompatibilityState() throws Exception {
		String backend = Files.readString(Path.of("src/main/java/net/vulkanic/backends/vulkan/VulkanBackend.java"));
		String[][] operations = {
			{"bindBuffer", "public void bindBuffer(CommandContext ctx, int target, int buffer)"},
			{"bindVertexArray", "public void bindVertexArray(CommandContext ctx, int vao)"},
			{"enableVertexAttribArray", "public void enableVertexAttribArray(CommandContext ctx, int index)"},
			{"disableVertexAttribArray", "public void disableVertexAttribArray(CommandContext ctx, int index)"},
			{"setVertexAttribPointer", "public void setVertexAttribPointer(CommandContext ctx, int index, int size, int type"},
			{"setVertexAttribIPointer", "public void setVertexAttribIPointer(CommandContext ctx, int index, int size, int type"},
			{"setVertexAttribDivisor", "public void setVertexAttribDivisor(CommandContext ctx, int index, int divisor)"},
			{"setVertexAttrib4f", "public void setVertexAttrib4f(CommandContext ctx, int index"},
			{"setVertexAttribFormat", "public void setVertexAttribFormat(CommandContext ctx, int attribindex"},
			{"setVertexAttribIFormat", "public void setVertexAttribIFormat(CommandContext ctx, int attribindex"},
			{"setVertexAttribBinding", "public void setVertexAttribBinding(CommandContext ctx, int attribindex"},
			{"bindVertexBuffer", "public void bindVertexBuffer(CommandContext ctx, int bindingindex"}
		};
		for (String[] operation : operations) {
			int method = backend.indexOf(operation[1]);
			assertTrue(method >= 0, "missing Java Vulkan vertex-input method: " + operation[0]);
			String body = backend.substring(method, Math.min(backend.length(), method + 420));
			assertTrue(body.contains("rejectJavaWholeFramePass(\"" + operation[0] + "\")"),
				"Java Vulkan vertex-input state must fail closed: " + operation[0]);
		}
	}

	@Test
	void wholeFrameJavaVulkanUniformSettersCannotMutateCompatibilityState() throws Exception {
		String backend = Files.readString(Path.of("src/main/java/net/vulkanic/backends/vulkan/VulkanBackend.java"));
		String[] operations = {
			"setUniform1i", "setUniform1f", "setUniform2f", "setUniform2i", "setUniform3f", "setUniform3i",
			"setUniform4f", "setUniform4i", "setUniformMatrix3fv", "setUniformMatrix4fv",
			"setUniform2fv", "setUniform3fv", "setUniform4fv"
		};
		for (String operation : operations) {
			int method = backend.indexOf("public void " + operation + "(");
			assertTrue(method >= 0, "missing Java Vulkan uniform setter: " + operation);
			String body = backend.substring(method, Math.min(backend.length(), method + 360));
			assertTrue(body.contains("rejectJavaWholeFramePass(\"" + operation + "\")"),
				"Java Vulkan uniforms must fail closed: " + operation);
		}
	}

	@Test
	void wholeFrameJavaVulkanFramebufferRoutingCannotMutateCompatibilityState() throws Exception {
		String backend = Files.readString(Path.of("src/main/java/net/vulkanic/backends/vulkan/VulkanBackend.java"));
		String[][] operations = {
			{"setClearColor", "public void setClearColor(CommandContext ctx"},
			{"setClearDepth", "public void setClearDepth(CommandContext ctx"},
			{"setClearStencil", "public void setClearStencil(CommandContext ctx"},
			{"setReadBuffer", "public void setReadBuffer(CommandContext ctx"},
			{"setDrawBuffer", "public void setDrawBuffer(CommandContext ctx"},
			{"bindFramebuffer", "public void bindFramebuffer(CommandContext ctx"},
			{"bindRenderTarget", "public void bindRenderTarget(CommandContext ctx"},
			{"drawBuffers", "public void drawBuffers(CommandContext ctx"}
		};
		for (String[] operation : operations) {
			int method = backend.indexOf(operation[1]);
			assertTrue(method >= 0, "missing Java Vulkan framebuffer-routing method: " + operation[0]);
			String body = backend.substring(method, Math.min(backend.length(), method + 420));
			assertTrue(body.contains("rejectJavaWholeFramePass(\"" + operation[0] + "\")"),
				"Java Vulkan framebuffer routing must fail closed: " + operation[0]);
		}
	}

	@Test
	void wholeFrameJavaVulkanDescriptorBindingCompatibilityStateCannotMutate() throws Exception {
		String backend = Files.readString(Path.of("src/main/java/net/vulkanic/backends/vulkan/VulkanBackend.java"));
		String[][] operations = {
			{"setAttributeLocation", "public void setAttributeLocation(CommandContext ctx"},
			{"uniformBlockBinding", "public void uniformBlockBinding(CommandContext ctx"},
			{"bindBufferBase", "public void bindBufferBase(CommandContext ctx"},
			{"bindUniformBufferBase", "public void bindUniformBufferBase(CommandContext ctx"},
			{"bindUniformBufferRange", "public void bindUniformBufferRange(CommandContext ctx"},
			{"bindFragDataLocation", "public void bindFragDataLocation(CommandContext ctx"},
			{"bindImageTexture", "public void bindImageTexture(CommandContext ctx"}
		};
		for (String[] operation : operations) {
			int method = backend.indexOf(operation[1]);
			assertTrue(method >= 0, "missing Java Vulkan descriptor-binding method: " + operation[0]);
			String body = backend.substring(method, Math.min(backend.length(), method + 420));
			assertTrue(body.contains("rejectJavaWholeFramePass(\"" + operation[0] + "\")"),
				"Java Vulkan descriptor binding must fail closed: " + operation[0]);
		}
	}

	@Test
	void wholeFrameJavaVulkanLegacyTextureMutationCannotMutateCompatibilityState() throws Exception {
		String backend = Files.readString(Path.of("src/main/java/net/vulkanic/backends/vulkan/VulkanBackend.java"));
		String[][] operations = {
			{"setTextureParameter", "public void setTextureParameter(CommandContext ctx"},
			{"texParameterf", "public void texParameterf(CommandContext ctx"},
			{"texParameteri", "public void texParameteri(CommandContext ctx, int target"},
			{"setPixelStore", "public void setPixelStore(CommandContext ctx"},
			{"uploadTexture2D", "public void uploadTexture2D(CommandContext ctx"},
			{"uploadTexture2DSubImage", "public void uploadTexture2DSubImage(CommandContext ctx"},
			{"texBuffer", "public void texBuffer(CommandContext ctx"},
			{"texParameteriv", "public void texParameteriv(CommandContext ctx"},
			{"textureParameterf", "public void textureParameterf(CommandContext ctx"},
			{"textureParameteri", "public void textureParameteri(CommandContext ctx"},
			{"textureParameteriv", "public void textureParameteriv(CommandContext ctx"},
			{"uploadTexture1D", "public void uploadTexture1D(CommandContext ctx"},
			{"uploadTexture3D", "public void uploadTexture3D(CommandContext ctx"}
		};
		for (String[] operation : operations) {
			int method = backend.indexOf(operation[1]);
			assertTrue(method >= 0, "missing Java Vulkan texture mutation method: " + operation[0]);
			String body = backend.substring(method, Math.min(backend.length(), method + 900));
			assertTrue(body.contains("rejectJavaWholeFramePass(\"" + operation[0] + "\")"),
				"Java Vulkan texture mutation must fail closed: " + operation[0]);
		}
	}

	@Test
	void wholeFrameJavaVulkanClearAndTransferOperationsCannotMutateCompatibilityState() throws Exception {
		String backend = Files.readString(Path.of("src/main/java/net/vulkanic/backends/vulkan/VulkanBackend.java"));
		String[] operations = {
			"blitFramebuffer", "blitNamedFramebuffer", "blitNamedFramebufferDSA", "clearBufferSubData",
			"clearBufferfv", "clearBufferiv", "clearBufferuiv", "clearNamedFramebufferfv",
			"clearNamedFramebufferiv", "clearNamedFramebufferuiv", "clearTexImage", "copyImageSubData",
			"copyTexImage2D", "copyTexSubImage2D", "copyTextureSubImage2D"
		};
		for (String operation : operations) {
			int method = backend.indexOf("public void " + operation + "(");
			assertTrue(method >= 0, "missing Java Vulkan clear/transfer method: " + operation);
			String body = backend.substring(method, Math.min(backend.length(), method + 700));
			assertTrue(body.contains("rejectJavaWholeFramePass(\"" + operation + "\")"),
				"Java Vulkan clear/transfer operation must fail closed: " + operation);
		}
	}

	@Test
	void wholeFrameJavaVulkanBarrierAndNamedFramebufferMutationCannotRun() throws Exception {
		String backend = Files.readString(Path.of("src/main/java/net/vulkanic/backends/vulkan/VulkanBackend.java"));
		assertTrue(backend.contains("rejectJavaWholeFramePass(\"memoryBarrier\")"));
		String[] operations = {
			"namedFramebufferDrawBuffers", "namedFramebufferReadBuffer", "namedFramebufferTexture",
			"namedFramebufferTextureDSA"
		};
		for (String operation : operations) {
			int method = backend.indexOf("public void " + operation + "(");
			assertTrue(method >= 0, "missing Java Vulkan named-framebuffer method: " + operation);
			String body = backend.substring(method, Math.min(backend.length(), method + 360));
			assertTrue(body.contains("rejectJavaWholeFramePass(\"" + operation + "\")"),
				"Java Vulkan named-framebuffer mutation must fail closed: " + operation);
		}
	}

	@Test
	void wholeFrameJavaVulkanBufferStorageAndMappingCannotMutateCompatibilityState() throws Exception {
		String backend = Files.readString(Path.of("src/main/java/net/vulkanic/backends/vulkan/VulkanBackend.java"));
		String[] operations = {
			"bufferData", "bufferSubData", "bufferStorage", "copyBufferSubData", "mapBuffer", "unmapBuffer",
			"flushMappedBufferRange", "namedBufferDataDSA", "namedBufferData", "namedBufferSubDataDSA",
			"namedBufferStorageDSA", "mapNamedBufferRangeDSA", "unmapNamedBufferDSA",
			"flushMappedNamedBufferRangeDSA", "copyNamedBufferSubDataDSA"
		};
		for (String operation : operations) {
			int method = backend.indexOf(operation.equals("mapBuffer") || operation.startsWith("mapNamed")
				? "public java.nio.ByteBuffer " + operation + "("
				: "public void " + operation + "(");
			assertTrue(method >= 0, "missing Java Vulkan buffer method: " + operation);
			String body = backend.substring(method, Math.min(backend.length(), method + 500));
			assertTrue(body.contains("rejectJavaWholeFramePass(\"" + operation + "\")"),
				"Java Vulkan buffer state must fail closed: " + operation);
		}
	}

	@Test
	void wholeFrameJavaVulkanBlendAndStencilStateCannotMutateCompatibilityState() throws Exception {
		String backend = Files.readString(Path.of("src/main/java/net/vulkanic/backends/vulkan/VulkanBackend.java"));
		String[] operations = {
			"blendFunc", "blendFuncSeparatei", "setStencilFunc", "setStencilFuncSeparate", "setStencilOp",
			"setStencilOpSeparate", "setStencilWriteMask", "setStencilWriteMaskSeparate"
		};
		for (String operation : operations) {
			int method = backend.indexOf("public void " + operation + "(");
			assertTrue(method >= 0, "missing Java Vulkan blend/stencil method: " + operation);
			String body = backend.substring(method, Math.min(backend.length(), method + 500));
			assertTrue(body.contains("rejectJavaWholeFramePass(\"" + operation + "\")"),
				"Java Vulkan blend/stencil state must fail closed: " + operation);
		}
	}

	@Test
	void wholeFrameJavaVulkanAttachmentAndMipmapOperationsCannotMutateCompatibilityState() throws Exception {
		String backend = Files.readString(Path.of("src/main/java/net/vulkanic/backends/vulkan/VulkanBackend.java"));
		String[][] operations = {
			{"framebufferTexture", "public void framebufferTexture(CommandContext ctx"},
			{"framebufferTexture2D", "public void framebufferTexture2D(CommandContext ctx"},
			{"generateMipmap", "public void generateMipmap(CommandContext ctx"},
			{"generateTextureMipmap", "public void generateTextureMipmap(CommandContext ctx"},
			{"generateTextureMipmapDSA", "public void generateTextureMipmapDSA(CommandContext ctx"}
		};
		for (String[] operation : operations) {
			int method = backend.indexOf(operation[1]);
			assertTrue(method >= 0, "missing Java Vulkan attachment/mipmap method: " + operation[0]);
			String body = backend.substring(method, Math.min(backend.length(), method + 500));
			assertTrue(body.contains("rejectJavaWholeFramePass(\"" + operation[0] + "\")"),
				"Java Vulkan attachment/mipmap operation must fail closed: " + operation[0]);
		}
	}

	@Test
	void wholeFrameJavaVulkanShaderSourceLifecycleCannotMutateCompatibilityState() throws Exception {
		String backend = Files.readString(Path.of("src/main/java/net/vulkanic/backends/vulkan/VulkanBackend.java"));
		String[] operations = {"uploadShaderSource", "compileShader"};
		for (String operation : operations) {
			int method = backend.indexOf("public void " + operation + "(");
			assertTrue(method >= 0, "missing Java Vulkan shader lifecycle method: " + operation);
			String body = backend.substring(method, Math.min(backend.length(), method + 500));
			assertTrue(body.contains("rejectJavaWholeFramePass(\"" + operation + "\")"),
				"Java Vulkan shader lifecycle must fail closed: " + operation);
		}
	}

	@Test
	void wholeFrameJavaVulkanLogicOpCannotMutateCompatibilityState() throws Exception {
		String backend = Files.readString(Path.of("src/main/java/net/vulkanic/backends/vulkan/VulkanBackend.java"));
		int method = backend.indexOf("public void setLogicOp(CommandContext ctx");
		assertTrue(method >= 0);
		String body = backend.substring(method, Math.min(backend.length(), method + 260));
		assertTrue(body.contains("rejectJavaWholeFramePass(\"setLogicOp\")"));
	}

	@Test
	void wholeFrameJavaVulkanExplicitResourceSubmissionCannotRun() throws Exception {
		String backend = Files.readString(Path.of("src/main/java/net/vulkanic/backends/vulkan/VulkanBackend.java"));
		for (String operation : new String[] {"bindPipelineResources", "applyResourceBarriers"}) {
			int method = backend.indexOf("public void " + operation + "(");
			assertTrue(method >= 0, "missing Java Vulkan explicit-resource method: " + operation);
			String body = backend.substring(method, Math.min(backend.length(), method + 360));
			assertTrue(body.contains("rejectJavaWholeFramePass(\"" + operation + "\")"),
				"Java Vulkan explicit-resource submission must fail closed: " + operation);
		}
	}

	@Test
	void wholeFrameJavaVulkanMultiDrawCannotBypassRustOwnership() throws Exception {
		String backend = Files.readString(Path.of("src/main/java/net/vulkanic/backends/vulkan/VulkanBackend.java"));
		assertTrue(backend.contains("public void multiDrawElementsBaseVertex(CommandContext ctx, int mode"));
		assertTrue(backend.contains("public void multiDrawElementsBaseVertex(CommandContext ctx, VulkanicPrimitiveMode mode"));
		String guard = "rejectJavaWholeFramePass(\"multiDrawElementsBaseVertex\")";
		int firstGuard = backend.indexOf(guard);
		assertTrue(firstGuard >= 0);
		assertTrue(backend.indexOf(guard, firstGuard + guard.length()) >= 0);
	}

	@Test
	void wholeFrameJavaVulkanPixelReadbackCannotReopenCommandRecording() throws Exception {
		String backend = Files.readString(Path.of("src/main/java/net/vulkanic/backends/vulkan/VulkanBackend.java"));
		int method = backend.indexOf("public void readPixels(CommandContext ctx");
		assertTrue(method >= 0);
		String guard = "rejectJavaWholeFramePass(\"readPixels\")";
		int first = backend.indexOf(guard, method);
		assertTrue(first >= 0);
		assertTrue(backend.indexOf(guard, first + guard.length()) >= 0);
	}

	@Test
	void wholeFrameJavaVulkanRenderPassObjectsCannotSurviveOwnershipTransition() throws Exception {
		String backend = Files.readString(Path.of("src/main/java/net/vulkanic/backends/vulkan/VulkanBackend.java"));
		int pass = backend.indexOf("private static final class VulkanBackedRenderPass");
		assertTrue(pass >= 0);
		int ensure = backend.indexOf("private void ensureOpen(String operation)", pass);
		int close = backend.indexOf("public void close()", ensure);
		assertTrue(ensure >= 0 && close >= 0);
		assertTrue(backend.substring(ensure, Math.min(backend.length(), ensure + 260))
			.contains("rejectJavaWholeFramePass(\"renderPass.\" + operation)"));
		assertTrue(backend.substring(close, Math.min(backend.length(), close + 180))
			.contains("rejectJavaWholeFramePass(\"renderPass.close\")"));
	}

	@Test
	void wholeFrameJavaVulkanSwapchainLifecycleCannotMutateJavaState() throws Exception {
		String backend = Files.readString(Path.of("src/main/java/net/vulkanic/backends/vulkan/VulkanBackend.java"));
		String[][] operations = {
			{"recreateVulkanSwapchain", "public void recreateVulkanSwapchain()"},
			{"recreateVulkanSwapchainIfNeeded", "public boolean recreateVulkanSwapchainIfNeeded()"}
		};
		for (String[] operation : operations) {
			int method = backend.indexOf(operation[1]);
			assertTrue(method >= 0, "missing Java Vulkan swapchain method: " + operation);
			String body = backend.substring(method, Math.min(backend.length(), method + 300));
			assertTrue(body.contains("rejectJavaWholeFramePass(\"" + operation[0] + "\")"),
				"Java Vulkan swapchain lifecycle must fail closed: " + operation[0]);
		}
	}

	@Test
	void wholeFrameJavaVulkanDiagnosticsCannotPublishNativeHandles() throws Exception {
		String backend = Files.readString(Path.of("src/main/java/net/vulkanic/backends/vulkan/VulkanBackend.java"));
		assertTrue(backend.contains("Java Vulkan execution-context handles are unavailable while Rust owns whole-frame presentation."));
		assertTrue(backend.contains("Java Vulkan surface/swapchain handles are unavailable while Rust owns whole-frame presentation."));
	}

	@Test
	void selectedVulkanDiagnosticsDoNotFallThroughToJavaBringUp() throws Exception {
		String api = Files.readString(Path.of("src/main/java/net/vulkanic/VulkanicAPI.java"));
		String backend = Files.readString(Path.of("src/main/java/net/vulkanic/backends/vulkan/VulkanBackend.java"));
		assertTrue(api.contains("|| isVulkanBackendSelected()")
			&& api.contains("Rust whole-frame Vulkan owns execution context; Java native handles are unavailable")
			&& api.contains("Rust whole-frame Vulkan owns surface and swapchain presentation; Java native handles are unavailable"),
			"selected Vulkan diagnostics must remain explicit and Java-handle-free before Rust presentation admission");
		int bringUp = backend.indexOf("private void attemptNativeBringUp()");
		assertTrue(bringUp >= 0
			&& backend.indexOf("isVulkanBackendSelected()", bringUp) > bringUp
			&& backend.indexOf("nativeBringUpAttempted = true", bringUp) > backend.indexOf("isVulkanBackendSelected()", bringUp),
			"selected Vulkan must stop Java native bring-up before attempting a compatibility context");
	}

	@Test
	void selectedVulkanCannotInstallScopedJavaBackendOverride() throws Exception {
		String api = Files.readString(Path.of("src/main/java/net/vulkanic/VulkanicAPI.java"));
		int override = api.indexOf("public static <T> T withScopedBackendOverride");
		assertTrue(override >= 0
			&& api.indexOf("if (isVulkanBackendSelected())", override) > override
			&& api.indexOf("Selected Vulkan does not permit a scoped Java backend override", override)
			> api.indexOf("if (isVulkanBackendSelected())", override),
			"selected Vulkan must not install a scoped Java backend escape hatch");
	}

	@Test
	void selectedVulkanCannotInstallOrReadJavaCommandContextScope() throws Exception {
		String api = Files.readString(Path.of("src/main/java/net/vulkanic/VulkanicAPI.java"));
		int get = api.indexOf("public static CommandContext getCommandContext()");
		int push = api.indexOf("public static void pushCommandContext(CommandContext ctx)");
		assertTrue(get >= 0 && api.indexOf("isVulkanBackendSelected()", get) > get
			&& push > get && api.indexOf("Selected Vulkan cannot install a Java command context scope", push) > push,
			"selected Vulkan must not expose or install Java command-context state");
	}

	@Test
	void selectedVulkanCannotReplaceSemanticDeviceWithJavaGlDevice() throws Exception {
		String api = Files.readString(Path.of("src/main/java/net/vulkanic/VulkanicAPI.java"));
		int setter = api.indexOf("public static void setDevice(GpuDevice gpuDevice)");
		assertTrue(setter >= 0
			&& api.indexOf("isVulkanBackendSelected()", setter) > setter
			&& api.indexOf("instanceof net.vulkanic.backends.vulkan.VulkanWholeFrameSemanticGpuDevice", setter) > setter
			&& api.indexOf("Selected Vulkan accepts only the Rust Vulkan semantic device", setter) > setter,
			"selected Vulkan must keep every non-semantic Java GPU device out of the device ownership seam");
	}

	@Test
	void wholeFrameJavaVulkanEncoderCannotBorrowIrisPipelineOverrides() throws Exception {
		String encoder = Files.readString(Path.of("src/main/java/net/vulkanic/backends/vulkan/VulkanNativeCommandEncoder.java"));
		int resolver = encoder.indexOf("private static GlProgram resolveIrisOverrideProgram(");
		int guard = encoder.indexOf("do not\n            // inspect Iris' live pipeline manager", resolver);
		int manager = encoder.indexOf("Iris.getPipelineManager()", resolver);
		assertTrue(resolver >= 0 && guard > resolver && manager > guard,
			"Java Vulkan pipeline binding must fence Iris override lookup before touching the pipeline manager");
	}

	@Test
	void wholeFrameCommandEncoderCannotUseBootstrapJavaCompatibilityDevice() throws Exception {
		String api = Files.readString(Path.of("src/main/java/net/vulkanic/VulkanicAPI.java"));
		String backend = Files.readString(Path.of("src/main/java/net/vulkanic/backends/vulkan/VulkanBackend.java"));
		int create = api.indexOf("public static CommandEncoder createCommandEncoder()");
		int guard = api.indexOf("RustGalVulkanWholeFrameMode.enabled()", create);
		int compatibilityBranch = api.indexOf("getDevice().createCommandEncoder()", create);
		assertTrue(create >= 0 && guard > create, "command encoder creation must fail closed from Rust whole-frame selection");
		assertTrue(compatibilityBranch < 0 || compatibilityBranch < guard,
			"whole-frame command encoder creation must not fall through to the Java compatibility device");
		assertTrue(backend.contains("Java Vulkan compatibility backend access is unavailable while Rust owns whole-frame presentation"),
			"the public compatibility wrapper must not become a whole-frame Java rendering escape hatch");
		assertTrue(api.contains("|| isVulkanBackendSelected()")
			&& backend.contains("Java Vulkan command encoder creation is unavailable on the selected Rust Vulkan route")
			&& backend.contains("Java Vulkan terrain command encoder creation is unavailable on the selected Rust Vulkan route"),
			"selected Vulkan must reject both general and terrain Java encoder creation before native state access");
		assertTrue(api.contains("rejectJavaVulkanWholeFrameOperation(\"command-buffer submission\")")
			&& api.contains("rejectJavaVulkanWholeFrameOperation(\"GPU completion fence creation\")"),
			"stale Java command buffers and GPU fences must not execute after Rust takes whole-frame ownership");
	}

	@Test
	void wholeFrameTaczPipelinePrecompileCannotReopenJavaVulkan() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/special/TaczGlock17SpecialRenderer.java"));
		int helper = source.indexOf("private static void ensureImmediatePipelineReady(");
		int guard = source.indexOf("RustGalVulkanWholeFrameMode.enabled()", helper);
		int precompile = source.indexOf("VulkanicAPI.precompileRenderPipeline", helper);
		assertTrue(helper >= 0 && guard > helper && (precompile < 0 || precompile > guard),
			"TACZ pipeline precompilation must not enter Java Vulkan during Rust whole-frame rendering");
		int immediate = source.indexOf("private static void drawMeshImmediate(");
		assertTrue(source.indexOf("VulkanicAPI.isVulkanBackendSelected()", immediate) > immediate,
			"selected Vulkan must reject TACZ Java immediate mesh rendering before pipeline setup");
	}

	@Test
	void wholeFrameDynamicUniformResetNeverUsesCompatibilityBackendWrapper() throws Exception {
		String api = Files.readString(Path.of("src/main/java/net/vulkanic/VulkanicAPI.java"));
		int reset = api.indexOf("public static void resetDynamicUniforms()");
		int guard = api.indexOf("RustGalVulkanWholeFrameMode.enabled()", reset);
		int wrapper = api.indexOf("rawVulkanBackend.withCompatibilityBackend", reset);
		assertTrue(reset >= 0 && guard > reset, "dynamic uniform reset must select semantic CPU state in whole-frame mode");
		assertTrue(wrapper < 0 || wrapper < guard,
			"whole-frame dynamic uniform reset must not enter the Java compatibility backend wrapper");
	}

	@Test
	void wholeFrameDynamicUniformInitializationDoesNotAllocateJavaRing() throws Exception {
		String api = Files.readString(Path.of("src/main/java/net/vulkanic/VulkanicAPI.java"));
		int initialize = api.indexOf("public static void initializeDynamicUniforms()");
		int guard = api.indexOf("RustGalVulkanWholeFrameMode.enabled()", initialize);
		int absent = api.indexOf("dynamicUniforms = null", guard);
		int branchEnd = api.indexOf("\n        }", absent);
		int allocate = api.indexOf("dynamicUniforms = new DynamicUniforms()", guard);
		assertTrue(initialize >= 0 && guard > initialize && absent > guard,
			"whole-frame dynamic uniform initialization must leave the Java ring absent");
		assertTrue(allocate < 0 || allocate < guard || allocate > branchEnd,
			"whole-frame dynamic uniform initialization must not construct a Java GPU ring");
	}

	@Test
	void wholeFrameDynamicUniformStorageCannotBypassRingOwnershipGuard() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/DynamicUniformStorage.java"));
		int constructor = source.indexOf("public DynamicUniformStorage(");
		int guard = source.indexOf("RustGalVulkanWholeFrameMode.enabled()", constructor);
		int ring = source.indexOf("new MappableRingBuffer", constructor);
		assertTrue(constructor >= 0 && guard > constructor,
			"dynamic uniform storage must reject whole-frame Vulkan explicitly");
		assertTrue(ring < 0 || ring > guard,
			"dynamic uniform storage must not allocate a Java ring before its ownership guard");
	}

	@Test
	void wholeFrameProjectionBufferFamiliesDoNotAllocateJavaUbos() throws Exception {
		for (String name : new String[] {
			"PerspectiveProjectionMatrixBuffer.java",
			"CachedPerspectiveProjectionMatrixBuffer.java",
			"CachedOrthoProjectionMatrixBuffer.java"
		}) {
			String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer", name));
			int constructor = source.indexOf("public " + name.substring(0, name.length() - 5) + "(");
			int guard = source.indexOf("RustGalVulkanWholeFrameMode.enabled()", constructor);
			int nullBuffer = source.indexOf("this.buffer = null", guard);
			int create = source.indexOf("VulkanicAPI.createBuffer", guard);
			assertTrue(constructor >= 0 && guard > constructor && nullBuffer > guard,
				name + " must leave its Java UBO absent for Rust whole-frame presentation");
			assertTrue(create < 0 || create > nullBuffer,
				name + " must not allocate its Java UBO before the whole-frame guard");
		}
	}

	@Test
	void wholeFrameSodiumChunkRendererDoesNotAllocateJavaTerrainBuffers() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/sodium/client/render/chunk/DefaultChunkRenderer.java"));
		int constructor = source.indexOf("public DefaultChunkRenderer(");
		int guard = source.indexOf("RustGalVulkanWholeFrameMode.enabled()", constructor);
		int nullIndex = source.indexOf("this.sharedIndexBuffer = null", guard);
		int commandList = source.indexOf("device.createCommandList()", guard);
		int createBuffer = source.indexOf("VulkanicAPI.createBuffer", guard);
		assertTrue(constructor >= 0 && guard > constructor && nullIndex > guard,
			"Sodium terrain renderer must retain no Java GPU resources for Rust whole-frame Vulkan");
		assertTrue(commandList < 0 || commandList > nullIndex,
			"whole-frame Sodium construction must not create a Java command list");
		assertTrue(createBuffer < 0 || createBuffer > nullIndex,
			"whole-frame Sodium construction must not create a Java terrain UBO");
	}

	@Test
	void wholeFrameSodiumChunkArenasDoNotReserveJavaGpuMemory() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/sodium/client/render/chunk/buffer/GpuChunkBufferArena.java"));
		int constructor = source.indexOf("public GpuChunkBufferArena(");
		assertTrue(source.contains("VulkanicAPI.isVulkanBackendSelected()")
			&& source.contains("Selected Vulkan Sodium chunk arena is unavailable")
			&& source.contains("Selected Vulkan Sodium chunk uploads are unavailable"),
			"selected Vulkan Sodium arenas and uploads must fail closed before Java buffers");
		int guard = source.indexOf("RustGalVulkanWholeFrameMode.enabled()", constructor);
		int nullBuffer = source.indexOf("this.arenaBuffer = null", guard);
		int create = source.indexOf("createBuffer(this.label", guard);
		int disabledBranch = source.indexOf("if (!net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled())", constructor);
		assertTrue(constructor >= 0 && guard > constructor && nullBuffer > guard,
			"whole-frame Sodium arenas must keep Java GPU memory unallocated");
		assertTrue(disabledBranch > constructor && create > disabledBranch && create < nullBuffer,
			"Java Sodium arena allocation must remain exclusively in the non-whole-frame branch");
	}

	@Test
	void wholeFrameDynamicTexturesRemainCpuOnly() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/texture/DynamicTexture.java"));
		int supplier = source.indexOf("private void createTexture(Supplier<String> supplier)");
		int supplierGuard = source.indexOf("RustGalVulkanWholeFrameMode.enabled()", supplier);
		int supplierCreate = source.indexOf("VulkanicAPI.createTexture", supplier);
		int string = source.indexOf("private void createTexture(String string)");
		int stringGuard = source.indexOf("RustGalVulkanWholeFrameMode.enabled()", string);
		int stringCreate = source.indexOf("VulkanicAPI.createTexture", string);
		assertTrue(supplier >= 0 && supplierGuard > supplier && supplierCreate > supplierGuard,
			"supplier dynamic textures must guard Java GPU creation for Rust whole-frame Vulkan");
		assertTrue(string >= 0 && stringGuard > string && stringCreate > stringGuard,
			"named dynamic textures must guard Java GPU creation for Rust whole-frame Vulkan");
	}

	@Test
	void selectedVulkanDynamicTexturesRemainSemanticAndRegisteredByIdentity() throws Exception {
		String dynamic = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/texture/DynamicTexture.java"));
		String manager = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/texture/TextureManager.java"));
		assertTrue(dynamic.contains("net.vulkanic.VulkanicAPI.isVulkanBackendSelected()")
			&& dynamic.contains("releaseJavaGpuTextureForSemanticRoute()")
			&& dynamic.contains("stageDynamicTexture(this)")
			&& dynamic.indexOf("releaseJavaGpuTextureForSemanticRoute()") < dynamic.indexOf("stageDynamicTexture(this)"),
			"selected Vulkan dynamic images must stage CPU pixels instead of uploading Java textures");
		assertTrue(manager.contains("net.vulkanic.VulkanicAPI.isVulkanBackendSelected()")
			&& manager.contains("registerDynamicTexture(resourceLocation, dynamicTexture)")
			&& manager.contains("unregisterDynamicTexture(resourceLocation, dynamicTexture)"),
			"selected Vulkan dynamic images must be registered and retired by semantic identity");
	}

	@Test
	void worldDynamicAndParticleImportsDoNotPublishGuiRawImages() throws Exception {
		String world = Files.readString(Path.of(
			"src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		assertTrue(world.contains("semanticSnapshotUnstaged(identity)")
			&& world.contains("semanticSnapshotUnstaged(atlasLocation)")
			&& !world.contains("RustGalGuiRawImageAssets.semanticSnapshot(identity)")
			&& !world.contains("RustGalGuiRawImageAssets.semanticSnapshot(atlasLocation)"),
			"world dynamic and particle imports must not publish through the GUI raw-image queue");
	}

	@Test
	void firstPersonMapTexturePublishesOnlyAfterGuiMeshAdmission() throws Exception {
		String world = Files.readString(Path.of(
			"src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		int method = world.indexOf("enqueueFirstPersonGuiTexturedQuad(");
		int resolve = world.indexOf("resolveSemanticImage(textureIdentity)", method);
		int enqueue = world.indexOf("enqueueGuiMeshItemRequest(", resolve);
		int stage = world.indexOf("stageSemanticImage(textureIdentity)", enqueue);
		assertTrue(method >= 0 && resolve > method && enqueue > resolve && stage > enqueue,
			"first-person map textures must publish only after explicit GUI mesh admission");
		assertTrue(world.indexOf("if (!finiteMatrix(transform))", method) > method
			&& world.indexOf("requires finite vertex coordinates", method) > method
			&& world.indexOf("requires finite UV coordinates", method) > method,
			"first-person map admission must reject non-finite transforms, vertices, and UVs before projection");
	}

	@Test
	void selectedVulkanOverlayPublishesLateBeforeWorldExtraction() throws Exception {
		String overlay = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/texture/OverlayTexture.java"));
		String gameRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
		assertTrue(overlay.contains("ensureSemanticAsset()")
			&& overlay.contains("semanticPublished")
			&& overlay.contains("registerDynamicTexture(SEMANTIC_IDENTITY, this.texture)"),
			"overlay texture must support late semantic publication after Vulkan selection");
		int frameStart = gameRenderer.indexOf("renderRustVulkanWholeFrameShell");
		int reset = gameRenderer.indexOf("RustGalGuiRenderer.beginWholeFrameVulkanFrame()", frameStart);
		int ensure = gameRenderer.indexOf("this.overlayTexture.ensureSemanticAsset()", reset);
		assertTrue(frameStart >= 0 && reset >= 0 && ensure > reset,
			"Rust frame startup must publish the overlay before semantic extraction");
	}

	@Test
	void persistentMapPlayerSkinPublishesSynthesizedImageSemantically() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/voxelmap/persistent/GuiPersistentMap.java"));
		assertTrue(source.contains("RustGalGuiRawImageAssets.registerDynamicTexture(voxelmapSkinLocation, texture)"),
			"runtime-synthesized persistent-map skin must be published as a semantic CPU image");
		assertTrue(source.contains("RustGalVulkanWholeFrameMode.enabled()"),
			"persistent-map skin publication must honor the Rust whole-frame ownership signal");
	}

	@Test
	void debugSemanticCollectorsHonorPresenterShellOwnership() throws Exception {
		Path debugRoot = Path.of("src/main/java/net/minecraft/client/renderer/debug");
		try (var files = Files.list(debugRoot)) {
			for (Path path : files.filter(path -> path.getFileName().toString().endsWith(".java")).toList()) {
				String source = Files.readString(path);
				if (source.contains("collectRustSemantics")) {
					assertFalse(source.matches("(?s).*if \\(\\s*!net\\.vulkanic\\.VulkanicAPI\\.isVulkanBackendSelected\\(\\)\\s*\\|\\| !net\\.vulkanic\\.world\\.WorldRenderRoutePolicy.*"),
						"debug semantic collector must not skip the Rust presenter shell: " + path);
				}
			}
		}
	}

	@Test
	void selectedVulkanLightmapReleasesPreSelectionJavaResources() throws Exception {
		String light = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/LightTexture.java"));
		String gameRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
		assertTrue(light.contains("ensureRustSemanticRoute()")
			&& light.contains("this.texture = null")
			&& light.contains("this.textureView = null")
			&& light.contains("this.ubo = null"),
			"selected Vulkan lightmap must release any Java resources created before backend selection");
		int frameStart = gameRenderer.indexOf("renderRustVulkanWholeFrameShell");
		int ensure = gameRenderer.indexOf("this.lightTexture().ensureRustSemanticRoute()", frameStart);
		assertTrue(frameStart >= 0 && ensure > frameStart,
			"Rust frame startup must release stale Java lightmap resources before extraction");
	}

	@Test
	void selectedVulkanWorldBorderReleasesPreSelectionJavaBuffer() throws Exception {
		String border = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/WorldBorderRenderer.java"));
		String gameRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
		assertTrue(border.contains("ensureRustSemanticRoute()")
			&& border.contains("this.worldBorderBuffer.close()")
			&& border.contains("this.worldBorderBuffer = null"),
			"selected Vulkan world-border rendering must release a pre-selection Java vertex buffer");
		int frameStart = gameRenderer.indexOf("renderRustVulkanWholeFrameShell");
		int ensure = gameRenderer.indexOf("getWorldBorderRenderer().ensureRustSemanticRoute()", frameStart);
		assertTrue(frameStart >= 0 && ensure > frameStart,
			"Rust frame startup must release the world-border compatibility buffer");
	}

	@Test
	void selectedVulkanMainTargetDestroysPreSelectionAttachments() throws Exception {
		String target = Files.readString(Path.of("src/main/java/net/blaze3d/pipeline/RenderTarget.java"));
		String gameRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
		assertTrue(target.contains("ensureRustSemanticRoute()")
			&& target.contains("this.destroyBuffers()")
			&& target.contains("this.colorTexture = null")
			&& target.contains("this.depthTexture = null"),
			"selected Vulkan targets must destroy pre-selection Java attachments");
		int frameStart = gameRenderer.indexOf("renderRustVulkanWholeFrameShell");
		int ensure = gameRenderer.indexOf("getMainRenderTarget().ensureRustSemanticRoute()", frameStart);
		assertTrue(frameStart >= 0 && ensure > frameStart,
			"Rust frame startup must hand the main target to the Rust presenter before extraction");
	}

	@Test
	void selectedVulkanFrameGraphTargetsReleasePooledJavaAttachments() throws Exception {
		String descriptor = Files.readString(Path.of("src/main/java/net/blaze3d/resource/RenderTargetDescriptor.java"));
		assertTrue(descriptor.contains("renderTarget.ensureRustSemanticRoute()")
			&& descriptor.indexOf("renderTarget.ensureRustSemanticRoute()")
				< descriptor.indexOf("return;"),
			"Rust frame-graph target preparation must release pooled Java attachments before returning");
	}

	@Test
	void selectedVulkanShaderManagerReleasesCachedJavaPostResources() throws Exception {
		String manager = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/ShaderManager.java"));
		String projection = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/CachedOrthoProjectionMatrixBuffer.java"));
		String gameRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
		assertTrue(manager.contains("ensureRustSemanticRoute()")
			&& manager.contains("this.compilationCache.close()")
			&& manager.contains("postChainProjectionMatrixBuffer.ensureRustSemanticRoute()"),
			"selected Vulkan shader manager must retire cached Java post-chain resources");
		assertTrue(projection.contains("ensureRustSemanticRoute()")
			&& projection.contains("this.buffer = null")
			&& projection.contains("this.bufferSlice = null"),
			"cached post projection UBO must be relinquishable after Vulkan selection");
		int frameStart = gameRenderer.indexOf("renderRustVulkanWholeFrameShell");
		int ensure = gameRenderer.indexOf("getShaderManager().ensureRustSemanticRoute()", frameStart);
		assertTrue(frameStart >= 0 && ensure > frameStart,
			"Rust frame startup must retire cached Java post resources before extraction");
	}

	@Test
	void selectedVulkanSkyReleasesPreSelectionCelestialBuffers() throws Exception {
		String sky = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/SkyRenderer.java"));
		String gameRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
		assertTrue(sky.contains("ensureRustSemanticRoute()")
			&& sky.contains("this.starBuffer = null")
			&& sky.contains("this.sunBuffer = null")
			&& sky.contains("this.endFlashBuffer = null"),
			"selected Vulkan sky rendering must release pre-selection celestial buffers");
		assertTrue(sky.contains("if (Minecraft.getInstance().player == null)")
			&& sky.contains("return false;"),
			"semantic sky extraction must remain unavailable until a camera player exists");
		int frameStart = gameRenderer.indexOf("renderRustVulkanWholeFrameShell");
		int ensure = gameRenderer.indexOf("getSkyRenderer().ensureRustSemanticRoute()", frameStart);
		assertTrue(frameStart >= 0 && ensure > frameStart,
			"Rust frame startup must release Java sky buffers before semantic sky extraction");
	}

	@Test
	void selectedVulkanCloudsReleasePreSelectionJavaUniforms() throws Exception {
		String clouds = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/CloudRenderer.java"));
		String gameRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
		assertTrue(clouds.contains("ensureRustSemanticRoute()")
			&& clouds.contains("this.ubo = null")
			&& clouds.contains("this.utb = null"),
			"selected Vulkan cloud rendering must release pre-selection Java uniform buffers");
		int frameStart = gameRenderer.indexOf("renderRustVulkanWholeFrameShell");
		int ensure = gameRenderer.indexOf("getCloudRenderer().ensureRustSemanticRoute()", frameStart);
		assertTrue(frameStart >= 0 && ensure > frameStart,
			"Rust frame startup must release Java cloud uniforms before semantic cloud extraction");
	}

	@Test
	void selectedVulkanParticleFeatureCacheReleasesUsedAndAvailableBuffers() throws Exception {
		String particles = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/feature/ParticleFeatureRenderer.java"));
		String gameRenderer = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
		assertTrue(particles.contains("ensureRustSemanticRoute()")
			&& particles.contains("this.availableBuffers.clear()")
			&& particles.contains("this.usedBuffers.clear()"),
			"selected Vulkan particle cache must release both idle and in-use Java buffers");
		int frameStart = gameRenderer.indexOf("renderRustVulkanWholeFrameShell");
		int ensure = gameRenderer.indexOf("particleFeatureRenderer.ensureRustSemanticRoute()", frameStart);
		assertTrue(frameStart >= 0 && ensure > frameStart,
			"Rust frame startup must retire Java particle buffers before semantic extraction");
	}

	@Test
	void wholeFrameRenderTargetsRetainDimensionsWithoutJavaAttachments() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/blaze3d/pipeline/RenderTarget.java"));
		int create = source.indexOf("public void createBuffers(int i, int j)");
		int guard = source.indexOf("RustGalVulkanWholeFrameMode.enabled()", create);
		int returnIndex = source.indexOf("return;", guard);
		int depthCreate = source.indexOf("VulkanicAPI.createTexture", guard);
		assertTrue(create >= 0 && guard > create && returnIndex > guard,
			"whole-frame RenderTarget creation must retain layout dimensions and return before Java attachments");
		assertTrue(depthCreate < 0 || depthCreate > returnIndex,
			"whole-frame RenderTarget creation must not allocate Java color/depth textures");
	}

	@Test
	void wholeFrameIrisFullscreenQuadDoesNotAllocateJavaVertexBuffer() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/irisshaders/iris/pathways/FullScreenQuadRenderer.java"));
		int constructor = source.indexOf("private FullScreenQuadRenderer()");
		int guard = source.indexOf("RustGalVulkanWholeFrameMode.enabled()", constructor);
		int nullQuad = source.indexOf("this.quad = null", guard);
		int create = source.indexOf("VulkanicAPI.createBuffer", guard);
		assertTrue(constructor >= 0 && guard > constructor && nullQuad > guard,
			"Iris fullscreen quad must remain absent in Rust whole-frame Vulkan");
		assertTrue(create < 0 || create > nullQuad,
			"whole-frame Iris fullscreen quad construction must not allocate a Java vertex buffer");
		assertTrue(source.contains("VulkanicAPI.isVulkanBackendSelected()")
			&& source.contains("Java Iris fullscreen quad is unavailable on the Rust Vulkan route"),
			"selected Vulkan must not construct or expose the Java fullscreen quad");
	}

	@Test
	void wholeFrameFontTexturesKeepOnlySemanticAtlasPixels() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/font/FontTexture.java"));
		int constructor = source.indexOf("public FontTexture(");
		int guard = source.indexOf("RustGalVulkanWholeFrameMode.enabled()", constructor);
		int create = source.indexOf("VulkanicAPI.createTexture", guard);
		int glyphGuard = source.indexOf("RustGalVulkanWholeFrameMode.enabled()", source.indexOf("GpuTextureView glyphTextureView"));
		assertTrue(constructor >= 0 && guard > constructor && create > guard,
			"font atlas Java texture creation must remain outside Rust whole-frame construction");
		assertTrue(glyphGuard > 0 && source.contains("? this.textureView"),
			"whole-frame glyphs must retain semantic atlas identity without requiring a Java texture view");
	}

	@Test
	void wholeFrameAtlasGlyphsDoNotRequireJavaAtlasViews() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/font/AtlasGlyphProvider.java"));
		int construction = source.indexOf("new AtlasGlyphProvider.Instance(");
		int guard = source.indexOf("RustGalVulkanWholeFrameMode.enabled()", construction);
		int atlasView = source.indexOf("atlas.getTextureView()", guard);
		assertTrue(construction >= 0 && guard > construction && atlasView > guard,
			"resource-pack glyphs must carry a nullable Java view while preserving semantic atlas identity");
		assertTrue(source.contains("@Nullable GpuTextureView textureView"),
			"atlas glyph compatibility views must be nullable on the semantic route");
	}

	@Test
	void wholeFrameOverlayTextureStagesPixelsWithoutJavaTextureOperations() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/texture/OverlayTexture.java"));
		int constructor = source.indexOf("public OverlayTexture()");
		int guard = source.indexOf("RustGalVulkanWholeFrameMode.enabled()", constructor);
		int stage = source.indexOf("ensureSemanticAsset()", guard);
		int clamp = source.indexOf("this.texture.setClamp(true)", guard);
		assertTrue(constructor >= 0 && guard > constructor && stage > guard,
			"whole-frame overlay must stage its completed CPU pixels into Rust semantic assets");
		assertTrue(clamp > stage,
			"Java overlay texture operations must remain in the non-whole-frame branch");
	}

	@Test
	void selectedVulkanOverlayUsesARegisteredSemanticIdentity() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/texture/OverlayTexture.java"));
		assertTrue(source.contains("VulkanicAPI.isVulkanBackendSelected()")
			&& source.contains("registerDynamicTexture(SEMANTIC_IDENTITY, this.texture)")
			&& source.contains("unregisterDynamicTexture(SEMANTIC_IDENTITY, this.texture)"),
			"selected Vulkan overlay pixels must be staged under a bounded semantic identity");
		assertTrue(source.contains("ensureSemanticAsset()"),
			"selected Vulkan overlay construction must publish its CPU pixels to Rust");
	}

	@Test
	void wholeFramePlayerGlyphsDoNotDereferenceSkinTextureViews() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/font/PlayerGlyphProvider.java"));
		int method = source.indexOf("public GpuTextureView textureView()");
		int guard = source.indexOf("RustGalVulkanWholeFrameMode.enabled()", method);
		int skinView = source.indexOf("skin.get()).textureView()", guard);
		assertTrue(method >= 0 && guard > method && skinView > guard,
			"whole-frame player glyphs must not dereference Java skin texture views");
		assertTrue(source.contains("@Nullable\n\t\tpublic GpuTextureView textureView()"),
			"player glyph compatibility views must be nullable on the semantic route");
	}

	@Test
	void selectedVulkanPlayerSkinCacheKeepsTextureViewsSemantic() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/PlayerSkinRenderCache.java"));
		int textureView = source.indexOf("public GpuTextureView textureView()");
		assertTrue(textureView >= 0
			&& source.indexOf("net.vulkanic.VulkanicAPI.isVulkanBackendSelected()", textureView) > textureView
			&& source.indexOf("textureManager.getTexture", textureView) > textureView,
			"selected Vulkan skin cache entries must not dereference Java texture views");
	}

	@Test
	void wholeFramePlayerSkinCacheDoesNotMaterializeJavaTextureViews() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/PlayerSkinRenderCache.java"));
		int method = source.indexOf("public GpuTextureView textureView()");
		int guard = source.indexOf("RustGalVulkanWholeFrameMode.enabled()", method);
		int manager = source.indexOf("textureManager.getTexture", guard);
		assertTrue(method >= 0 && guard > method && manager > guard,
			"whole-frame skin cache must avoid Java texture materialization");
		assertTrue(source.contains("@Nullable\n\t\tpublic GpuTextureView textureView()"),
			"player skin cache compatibility views must be nullable on the semantic route");
	}

	@Test
	void wholeFrameIrisWidgetBindingDoesNotMaterializeJavaTextureViews() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/irisshaders/iris/gui/GuiUtil.java"));
		int method = source.indexOf("public static void bindIrisWidgetsTexture()");
		int guard = source.indexOf("RustGalVulkanWholeFrameMode.enabled()", method);
		int textureView = source.indexOf("getTexture(IRIS_WIDGETS_TEX).getTextureView()", guard);
		assertTrue(method >= 0 && guard > method && textureView > guard,
			"whole-frame Iris widgets must not acquire a Java texture view");
		assertTrue(source.indexOf("VulkanicAPI.isVulkanBackendSelected()", guard) > guard,
			"selected Vulkan Iris widgets must not acquire a Java texture view before whole-frame admission");
	}

	@Test
	void wholeFrameSodiumTerrainAtlasViewIsUnavailable() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/chunk/ChunkSectionLayer.java"));
		int method = source.indexOf("public GpuTextureView textureView()");
		int guard = source.indexOf("RustGalVulkanWholeFrameMode.enabled()", method);
		int manager = source.indexOf("textureManager.getTexture", guard);
		assertTrue(method >= 0 && guard > method && manager > guard,
			"whole-frame Sodium terrain must not materialize the Java atlas view");
		assertTrue(source.contains("@Nullable\n\tpublic GpuTextureView textureView()"),
			"Sodium terrain compatibility atlas views must be nullable");
		assertTrue(source.contains("isVulkanBackendSelected()"),
			"selected Vulkan Sodium terrain must not materialize a Java atlas view");
	}

	@Test
	void endPortalScreensCannotFallBackToJavaTextureViewsOnWholeFrame() throws Exception {
		String loading = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/screens/LevelLoadingScreen.java"));
		int loadingCase = loading.indexOf("case END_PORTAL:");
		int loadingGuard = loading.indexOf("RustGalVulkanWholeFrameMode.enabled()", loadingCase);
		int loadingView = loading.indexOf("getTextureView()", loadingGuard);
		assertTrue(loadingCase >= 0 && loadingGuard > loadingCase && loadingView > loadingGuard,
			"whole-frame level loading must reject end-portal Java texture views before acquisition");

		String win = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/screens/WinScreen.java"));
		int poem = win.indexOf("if (this.poem)");
		int winGuard = win.indexOf("RustGalVulkanWholeFrameMode.enabled()", poem);
		int winView = win.indexOf("getTextureView()", winGuard);
		assertTrue(poem >= 0 && winGuard > poem && winView > winGuard,
			"whole-frame win screen must reject end-portal Java texture views before acquisition");
	}

	@Test
	void wholeFrameVoxelMapBlitsUseSemanticResourceIdentity() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/voxelmap/util/VoxelMapGuiGraphics.java"));
		int method = source.indexOf("blitFloatGradient(GuiGraphics graphics, RenderPipeline pipeline, ResourceLocation texture");
		int guard = source.indexOf("isWholeFrameVulkanEnabled()", method);
		int semantic = source.indexOf("submitRustSemanticBlit", guard);
		int javaView = source.indexOf("getTexture(texture).getTextureView()", guard);
		assertTrue(method >= 0 && guard > method && semantic > guard && javaView > semantic,
			"whole-frame VoxelMap resource blits must carry semantic texture identity before Java view acquisition");
	}

	@Test
	void wholeFrameVoxelMapOverlayUsesExplicitSemanticBlits() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/voxelmap/Map.java"));
		int minimap = source.indexOf("submitRustVoxelMapMask");
		int minimapBlit = source.indexOf("semanticMapTexture");
		int waypoint = source.indexOf("private void drawWaypointSemantic");
		int waypointBlit = source.indexOf("submitRustSemanticBlit", waypoint);
		int arrow = source.indexOf("private void drawArrow");
		int arrowBlit = source.indexOf("submitRustSemanticBlit", arrow);
		int frame = source.indexOf("private void drawMapFrame");
		int frameBlit = source.indexOf("submitRustSemanticBlit", frame);
		assertTrue(minimap >= 0 && minimapBlit >= 0
			&& waypoint >= 0 && waypointBlit > waypoint
			&& arrow >= 0 && arrowBlit > arrow
			&& frame >= 0 && frameBlit > frame,
			"Rust VoxelMap minimap, frame, arrow, and waypoint icons must use explicit semantic blits");
		int mapRender = source.indexOf("private void renderMap");
		int mapSquare = source.indexOf("VoxelMapGuiGraphics.blitSquareMap", mapRender);
		int mapCircular = source.indexOf("VoxelMapGuiGraphics.blitCircular", mapRender);
		assertTrue(mapRender >= 0 && mapSquare > mapRender && mapCircular > mapSquare,
			"Rust VoxelMap minimap masks must use copied semantic map meshes");
		assertFalse(source.substring(mapSquare, mapCircular).contains("getTextureView()"),
			"Rust VoxelMap square minimap must not acquire a Java texture view");
		assertFalse(source.substring(mapCircular, source.indexOf("double guiScale", mapCircular)).contains("getTextureView()"),
			"Rust VoxelMap circular minimap must not acquire a Java texture view");
	}

	@Test
	void wholeFrameCannotConvertJavaTexturesToLegacyNativeHandles() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/vulkanic/VulkanicCoreAPI.java"));
		int method = source.indexOf("public static int textureId(GpuTexture texture)");
		int viewMethod = source.indexOf("public static int textureId(GpuTextureView textureView)");
		assertTrue(viewMethod >= 0 && source.indexOf("textureView == null", viewMethod) > viewMethod,
			"typed texture-view handle conversion must reject a null view before dereference");
		int guard = source.indexOf("RustGalVulkanWholeFrameMode.enabled()", method);
		int handle = source.indexOf("VulkanicAPI.getTextureHandle(texture)", guard);
		assertTrue(method >= 0 && guard > method && handle > guard,
			"whole-frame Vulkan must reject Java texture-to-native-handle conversion");
		assertTrue(source.substring(guard, handle).contains("isVulkanBackendSelected()"),
			"selected Vulkan must reject legacy texture handles even before whole-frame shell activation");
	}

	@Test
	void wholeFrameCoreApiCannotBecomeASecondPresenter() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/vulkanic/VulkanicCoreAPI.java"));
		int method = source.indexOf("public static void presentTextureToScreen(CommandContext ctx, GpuTextureView textureView)");
		int guard = source.indexOf("RustGalVulkanWholeFrameMode.enabled()", method);
		int delegate = source.indexOf("VulkanicAPI.presentTextureToScreen(ctx, textureView)", guard);
		assertTrue(method >= 0 && guard > method && delegate > guard,
			"the typed core presenter must fail closed before delegating while Rust owns whole-frame presentation");
		assertTrue(source.substring(guard, delegate).contains("isVulkanBackendSelected()"),
			"selected Vulkan must reject the typed Java presenter before whole-frame shell activation");
	}

	@Test
	void wholeFrameTargetAuditCannotDescribeAJavaDiagnosticShell() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/vulkanic/gui/RustGalFrameCoordinator.java"));
		int method = source.indexOf("private static void auditWholeFrameTarget");
		int guard = source.indexOf("Rust Vulkan whole-frame target audit requires a consumed semantic primitive frame", method);
		assertTrue(method >= 0 && guard > method,
			"selected Rust Vulkan presentation must fail closed if its semantic primitive frame disappears before target audit");
	}

	@Test
	void wholeFrameCannotConvertJavaBuffersToLegacyNativeHandles() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/vulkanic/VulkanicCoreAPI.java"));
		int method = source.indexOf("public static int bufferId(net.blaze3d.buffers.GpuBuffer buffer)");
		int guard = source.indexOf("RustGalVulkanWholeFrameMode.enabled()", method);
		int handle = source.indexOf("VulkanicAPI.getBufferHandle(buffer)", guard);
		assertTrue(method >= 0 && guard > method && handle > guard,
			"whole-frame Vulkan must reject Java buffer-to-native-handle conversion");
		assertTrue(source.substring(guard, handle).contains("isVulkanBackendSelected()"),
			"selected Vulkan must reject legacy buffer handles even before whole-frame shell activation");
	}

	@Test
	void wholeFrameVoxelMapUploadsDoNotOpenJavaCommandContexts() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/voxelmap/Map.java"));
		int firstUpload = source.indexOf("this.mapImages[this.zoom].upload()");
		int firstGuard = source.indexOf("RustGalVulkanWholeFrameMode.enabled()", firstUpload);
		int firstBarrier = source.indexOf("applyResourceBarriers(VulkanicAPI.getCommandContext()", firstGuard);
		int secondUpload = source.indexOf("this.mapImages[this.zoom].upload()", firstUpload + 1);
		int secondGuard = source.indexOf("RustGalVulkanWholeFrameMode.enabled()", secondUpload);
		int secondBarrier = source.indexOf("applyResourceBarriers(VulkanicAPI.getCommandContext()", secondGuard);
		assertTrue(firstUpload >= 0 && firstGuard > firstUpload && firstBarrier > firstGuard,
			"whole-frame VoxelMap mini-map uploads must not reopen Java command contexts");
		assertTrue(secondUpload >= 0 && secondGuard > secondUpload && secondBarrier > secondGuard,
			"whole-frame VoxelMap full-map uploads must not reopen Java command contexts");
	}

	@Test
	void wholeFrameSodiumCloudOptionDoesNotClearJavaRenderTargets() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/sodium/client/gui/SodiumGameOptionPages.java"));
		int binding = source.indexOf("opts.cloudStatus().set(value)");
		int guard = source.indexOf("RustGalVulkanWholeFrameMode.enabled()", binding);
		int clear = source.indexOf("createCommandEncoder().clearColorAndDepthTextures", guard);
		assertTrue(binding >= 0 && guard > binding && clear > guard,
			"whole-frame Sodium cloud setting changes must not clear Java render targets");
	}

	@Test
	void wholeFrameImmediateVertexUploadsCannotReuseThroughJavaEncoder() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/blaze3d/vertex/VertexFormat.java"));
		int upload = source.indexOf("private static GpuBuffer uploadToBuffer(");
		int guard = source.indexOf("RustGalVulkanWholeFrameMode.enabled()", upload);
		int create = source.indexOf("VulkanicAPI.createBuffer", guard);
		int encoder = source.indexOf("VulkanicAPI.createCommandEncoder()", upload);
		assertTrue(upload >= 0 && guard > upload && create > guard,
			"whole-frame immediate uploads must select fresh semantic buffers");
		assertTrue(encoder > create, "Java command-encoder reuse must remain outside the whole-frame fresh-buffer branch");
	}

	@Test
	void frameAbiPayloadsUseConfinedPerSubmitArenas() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/vulkanic/bridge/VulkanicGalBridge.java"));
		int wholeFrame = source.indexOf("private WholeFrameSubmitResult submitWorldFrame(");
		int wholeFrameArena = source.indexOf("Arena frameArena = Arena.ofConfined()", wholeFrame);
		int guiFrame = source.indexOf("List<GuiMeshBatchRecord> meshBatches");
		int guiFrameArena = source.indexOf("Arena frameArena = Arena.ofConfined()", guiFrame);
		assertTrue(wholeFrame >= 0 && wholeFrameArena > wholeFrame,
			"whole-frame ABI payloads must not accumulate in the context arena");
		assertTrue(guiFrame >= 0 && guiFrameArena > guiFrame,
			"GUI ABI payloads must not accumulate in the context arena");
	}

	@Test
	void selectedVulkanShapeGeometryCannotFallThroughToJavaVertexConsumers() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/ShapeRenderer.java"));
		int guard = source.indexOf("private static void rejectSelectedVulkanJavaGeometry()");
		int selectedGate = source.indexOf("VulkanicAPI.isVulkanBackendSelected()", guard);
		int wholeFrameGate = source.indexOf("RustGalVulkanWholeFrameMode.enabled()", selectedGate);
		int rejection = source.indexOf("Java ShapeRenderer geometry is unavailable", wholeFrameGate);
		assertTrue(guard >= 0 && selectedGate > guard && wholeFrameGate > selectedGate && rejection > wholeFrameGate,
			"selected Vulkan shape geometry must fail closed before Java vertex consumers");
		assertFalse(source.substring(selectedGate, rejection).contains("&&"),
			"selected Vulkan shape rejection must not require whole-frame activation as a second condition");
		int face = source.indexOf("public static void renderFace(");
		int faceGuard = source.indexOf("rejectSelectedVulkanJavaGeometry();", face);
		int vector = source.indexOf("public static void renderVector(");
		int vectorGuard = source.indexOf("rejectSelectedVulkanJavaGeometry();", vector);
		assertTrue(face >= 0 && faceGuard > face && vector >= 0 && vectorGuard > vector,
			"all public ShapeRenderer geometry families must cross the Vulkan ownership gate");
	}

	@Test
	void selectedVulkanBlockAndFluidTessellationCannotFallThroughToJavaConsumers() throws Exception {
		String liquid = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/block/LiquidBlockRenderer.java"));
		int liquidEntry = liquid.indexOf("public void tesselate(");
		int liquidGate = liquid.indexOf("isVulkanBackendSelected()", liquidEntry);
		int liquidEmit = liquid.indexOf("boolean bl = fluidState.is", liquidEntry);
		assertTrue(liquidEntry >= 0 && liquidGate > liquidEntry && liquidEmit > liquidGate,
			"selected Vulkan fluids must not enter Java liquid tessellation");

		String model = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/block/ModelBlockRenderer.java"));
		int helper = model.indexOf("rejectSelectedVulkanJavaTessellation");
		int blockEntry = model.indexOf("public void tesselateBlock(");
		int blockGate = model.indexOf("rejectSelectedVulkanJavaTessellation();", blockEntry);
		int aoEntry = model.indexOf("public void tesselateWithAO(");
		int aoGate = model.indexOf("rejectSelectedVulkanJavaTessellation();", aoEntry);
		int noAoEntry = model.indexOf("public void tesselateWithoutAO(");
		int noAoGate = model.indexOf("rejectSelectedVulkanJavaTessellation();", noAoEntry);
		int staticEntry = model.indexOf("public static void renderModel(");
		int staticGate = model.indexOf("rejectSelectedVulkanJavaTessellation();", staticEntry);
		assertTrue(helper >= 0 && blockGate > blockEntry && aoGate > aoEntry
			&& noAoGate > noAoEntry && staticGate > staticEntry,
			"all public block-model tessellation entrypoints must cross the Rust ownership gate");
	}

	@Test
	void selectedVulkanTextureAtlasSpritesCannotUploadJavaGpuFrames() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/texture/TextureAtlasSprite.java"));
		int helper = source.indexOf("rejectSelectedVulkanJavaUpload");
		int firstFrame = source.indexOf("public void uploadFirstFrame(");
		int firstFrameGate = source.indexOf("rejectSelectedVulkanJavaUpload();", firstFrame);
		int ticker = source.indexOf("public void tickAndUpload(GpuTexture gpuTexture)");
		int tickerGate = source.indexOf("rejectSelectedVulkanJavaUpload();", ticker);
		int wrap = source.indexOf("public VertexConsumer wrap(VertexConsumer vertexConsumer)");
		int wrapGate = source.indexOf("rejectSelectedVulkanJavaUpload();", wrap);
		assertTrue(helper >= 0 && firstFrameGate > firstFrame && tickerGate > ticker && wrapGate > wrap,
			"selected Vulkan atlas sprites must not reopen Java GPU frame uploads");
	}

	@Test
	void selectedVulkanAbstractTexturesCannotExposeOrMutateJavaGpuState() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/texture/AbstractTexture.java"));
		int helper = source.indexOf("private static void ensureJavaTextureAccessAvailable");
		assertTrue(helper >= 0
			&& source.contains("ensureJavaTextureAccessAvailable(\"set clamp\")")
			&& source.contains("ensureJavaTextureAccessAvailable(\"set filter\")")
			&& source.contains("ensureJavaTextureAccessAvailable(\"set mipmaps\")")
			&& source.contains("ensureJavaTextureAccessAvailable(\"get texture\")")
			&& source.contains("ensureJavaTextureAccessAvailable(\"get texture view\")"),
			"selected Vulkan must fence every common Java texture getter and sampler mutation");
		assertTrue(source.contains("Java texture \" + operation + \" is unavailable while Rust owns Vulkan rendering"),
			"the common texture boundary must fail closed with an explicit ownership diagnostic");
	}

	@Test
	void selectedVulkanAtlasDiagnosticsDoNotReadJavaGpuTextures() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/texture/TextureAtlas.java"));
		int dump = source.indexOf("public void dumpContents(");
		int guard = source.indexOf("isVulkanBackendSelected()", dump);
		int names = source.indexOf("dumpSpriteNames(path, string, this.texturesByName)", guard);
		int gpuRead = source.indexOf("TextureUtil.writeAsPNG", dump);
		assertTrue(dump >= 0 && guard > dump && names > guard && gpuRead > names,
			"selected Vulkan atlas diagnostics must remain CPU-only before the Java GPU readback");
	}

	@Test
	void selectedVulkanVoxelMapSkyColorUsesSemanticFogWithoutIrisPublication() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/voxelmap/Map.java"));
		int helper = source.indexOf("private int getSkyColor()");
		int route = source.indexOf("boolean rustVulkan", helper);
		int semantic = source.indexOf("computeFogColorSemantic", route);
		int legacy = source.indexOf("computeFogColor(minecraft.gameRenderer", route);
		assertTrue(helper >= 0 && route > helper && semantic > route && legacy > semantic,
			"selected Vulkan VoxelMap sky color must use the Iris-free semantic fog path");
	}

	@Test
	void selectedVulkanVoxelMapEntityImageRendererCannotAllocateJavaFbos() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/voxelmap/entityrender/EntityMapImageManager.java"));
		int constructor = source.indexOf("public EntityMapImageManager()");
		int selected = source.indexOf("isVulkanBackendSelected()", constructor);
		int fbo = source.indexOf("createTexture(\"voxelmap-radarfbotexture\"", constructor);
		assertTrue(constructor >= 0 && selected > constructor && fbo > selected,
			"selected Vulkan must reject VoxelMap entity-image Java FBO construction");
	}

	@Test
	void selectedVulkanDebugEntriesDoNotEagerlyConstructIrisRuntime() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/components/debug/DebugScreenEntries.java"));
		assertTrue(source.contains("new IrisCompatibilityDebugEntry(false)")
			&& source.contains("new IrisCompatibilityDebugEntry(true)"),
			"Iris debug entries must be lazy compatibility wrappers");
		int wrapper = source.indexOf("private static final class IrisCompatibilityDebugEntry");
		int guard = source.indexOf("RustGalVulkanWholeFrameMode.enabled()", wrapper);
		int irisConstruction = source.indexOf("new net.irisshaders.iris.gui.debug", wrapper);
		assertTrue(wrapper >= 0 && guard > wrapper && irisConstruction > guard,
			"Rust Vulkan debug pages must return before constructing Iris debug runtime objects");
	}

	@Test
	void semanticBlockEntityScopesRestoreNestedIdentityAndRejectUnderflow() {
		assertEquals(-1, VulkanicGalBridge.activeSemanticBlockEntityId());
		VulkanicGalBridge.beginSemanticBlockEntity(17);
		try {
			assertEquals(17, VulkanicGalBridge.activeSemanticBlockEntityId());
			VulkanicGalBridge.beginSemanticBlockEntity(29);
			assertEquals(29, VulkanicGalBridge.activeSemanticBlockEntityId());
			VulkanicGalBridge.endSemanticBlockEntity();
			assertEquals(17, VulkanicGalBridge.activeSemanticBlockEntityId());
		} finally {
			if (VulkanicGalBridge.activeSemanticBlockEntityId() != -1) {
				VulkanicGalBridge.endSemanticBlockEntity();
			}
		}
		assertEquals(-1, VulkanicGalBridge.activeSemanticBlockEntityId());
		assertThrows(IllegalStateException.class, VulkanicGalBridge::endSemanticBlockEntity);
	}

	@Test
	void terrainPlacementRetainsDoublePrecisionAndRejectsAmbiguousOwnership() {
		net.minecraft.SharedConstants.tryDetectVersion();
		net.minecraft.server.Bootstrap.bootStrap();
		var placement = new VulkanicGalBridge.TerrainSectionPlacement(112, 80, 528, 150.5, 101.62, 530.5);
		assertEquals(101.62, placement.cameraY());
		assertNotEquals((double)(float)101.62, placement.cameraY());
		var original = new VulkanicGalBridge.WorldMeshInstanceRecord(
			60, 41L, 2L, -1, 0, 0, 0, 0xffffffff, new float[16], 640, 480, 0, 0, 0, 0, -1);
		var semantic = original.withTerrainPlacement(placement);
		assertEquals(placement, semantic.terrainPlacement());
		assertNull(original.terrainPlacement());
		assertArrayEquals(new float[] {1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1}, semantic.transform());
		float[] detached = semantic.transform();
		detached[12] = 99;
		assertEquals(0, semantic.transform()[12]);
		var frame = new RustGalWorldPrimitiveRenderer.PrimitiveFrame(640, 480,
			semantic.transform(), semantic.transform(), new VulkanicGalBridge.WorldBackgroundRecord(false, 0, 0, 0, 0, 640, 480),
			List.of(), List.of(), List.of(), List.of(), List.of(semantic));
		var resized = RustGalWorldPrimitiveRenderer.withViewport(frame, 1280, 720);
		assertEquals(placement, resized.meshInstances().getFirst().terrainPlacement());
		assertEquals(1280, resized.meshInstances().getFirst().viewportWidth());
		assertArrayEquals(semantic.transform(), resized.meshInstances().getFirst().transform());
		assertThrows(IllegalArgumentException.class, () -> new VulkanicGalBridge.TerrainSectionPlacement(1, 0, 0, 0, 0, 0));
		assertThrows(IllegalArgumentException.class, () -> new VulkanicGalBridge.TerrainSectionPlacement(0, 0, 0, Double.NaN, 0, 0));
		assertThrows(IllegalArgumentException.class, () -> new VulkanicGalBridge.WorldMeshInstanceRecord(
			60, 41L, 2L, -1, 0, 0, 0, 0xffffffff, detached, 640, 480, 0, 0, 0, 0, -1, placement));
		assertThrows(IllegalArgumentException.class, () -> new VulkanicGalBridge.WorldMeshInstanceRecord(
			67, 41L, 2L, -1, 0, 0, 0, 0xffffffff, semantic.transform(), 640, 480, 0, 0, 0, 0, -1, placement));
	}

	@Test
	void viewportResizePreservesWorldAndFirstPersonFoilSemantics() {
		net.minecraft.SharedConstants.tryDetectVersion();
		net.minecraft.server.Bootstrap.bootStrap();
		float[] transform = new org.joml.Matrix4f().translation(0.3f, -0.2f, 0.7f).get(new float[16]);
		var plain = new VulkanicGalBridge.WorldMeshInstanceRecord(
			67, 41L, 2L, -1, 0, 0, 0, 0xffffffff, transform, 640, 480);
		var foil = plain.withItemFoil(new VulkanicGalBridge.StandardItemFoilRecord(12345L, 0.25, 0.5f));
		var frame = new RustGalWorldPrimitiveRenderer.PrimitiveFrame(640, 480, transform, transform,
			new VulkanicGalBridge.WorldBackgroundRecord(false, 0, 0, 0, 0, 640, 480),
			List.of(), List.of(), List.of(), List.of(), List.of(), List.of(plain, foil), List.of(),
			VulkanicGalBridge.WorldVoxelVolumeFrameRecord.disabled(),
			VulkanicGalBridge.WorldShaderEnvironmentFrameRecord.disabled(),
			VulkanicGalBridge.WorldFeatureCoverageRecord.empty(), List.of(),
			VulkanicGalBridge.WorldLodRenderFrameRecord.disabled(), 0,
			VulkanicGalBridge.WorldFirstPersonFrameRecord.disabled(), List.of(plain, foil));
		assertSame(frame, RustGalWorldPrimitiveRenderer.withViewport(frame, 640, 480));
		var resized = RustGalWorldPrimitiveRenderer.withViewport(frame, 1280, 720);
		for (var instances : List.of(resized.meshInstances(), resized.firstPersonMeshInstances())) {
			assertNull(instances.getFirst().itemFoil());
			assertEquals(foil.itemFoil(), instances.get(1).itemFoil());
			for (var instance : instances) {
				assertEquals(1280, instance.viewportWidth());
				assertEquals(720, instance.viewportHeight());
				assertArrayEquals(transform, instance.transform());
				assertEquals(plain.meshKey(), instance.meshKey());
				assertEquals(plain.meshGeneration(), instance.meshGeneration());
			}
		}
		assertEquals(640, frame.firstPersonMeshInstances().get(1).viewportWidth());
		assertEquals(foil.itemFoil(), frame.firstPersonMeshInstances().get(1).itemFoil());
	}

	@Test
	void firstPersonMeshConvenienceRecordUsesExplicitNoScopeIdentity() {
		assertEquals(67, VulkanicGalBridge.WORLD_MESH_ENTITY_STRATUM);
		assertEquals(71, VulkanicGalBridge.WORLD_MESH_ORDINARY_BLOCK_STRATUM);
		VulkanicGalBridge.WorldMeshInstanceRecord firstPerson = new VulkanicGalBridge.WorldMeshInstanceRecord(
			VulkanicGalBridge.WORLD_MESH_ENTITY_STRATUM, 41L, 2L, 0,
			0, 0, 0, 0xffffffff, new float[16], 640, 480
		);
		assertEquals(0, firstPerson.flags());
		assertEquals(-1, firstPerson.blockEntityId());
		assertThrows(IllegalArgumentException.class, () -> new VulkanicGalBridge.WorldMeshInstanceRecord(
			VulkanicGalBridge.WORLD_MESH_ENTITY_STRATUM, 41L, 2L, 0,
			0, 0, 0, 0xffffffff, new float[16], 640, 480,
			0, 0, 0, -1
		));
		assertThrows(IllegalArgumentException.class, () -> new VulkanicGalBridge.WorldTextQuadRecord(
			1L, 1L, 1L, false, 0, 0, 0xffffffff, 0.0, new float[16],
			new float[12], new float[8], -2
		));
	}

	@Test
	void ffiContextAdmissionPrecedesBackendConstruction() throws Exception {
		String source = Files.readString(Path.of("src/main/rust/render/vulkanic/ffi/context.rs"));
		for (String constructor : new String[] {
			"mattmc_vulkanic_gal_context_create(",
			"mattmc_vulkanic_gal_context_create_borrowed_opengl(",
			"mattmc_vulkanic_gal_context_create_windowed_vulkan("
		}) {
			int start = source.indexOf(constructor);
			int next = source.indexOf("#[no_mangle]", start + constructor.length());
			String body = source.substring(start, next < 0 ? source.length() : next);
			assertTrue(body.indexOf("ensure_context_capacity()?") < body.indexOf("let backend ="),
				constructor + " must enforce the live-context bound before backend construction");
		}
	}

	@Test
	void skullSemanticCallsitesAlwaysProvideTextureIdentity() throws Exception {
		String customHead = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/entity/layers/CustomHeadLayer.java"));
		assertTrue(customHead.contains("resolveSkullSemanticTexture")
			&& customHead.contains("playerSkin().body().texturePath()")
			&& customHead.contains("SkullBlockRenderer.defaultTexture(type)"),
			"worn skulls must pass a copied semantic texture identity for both player and vanilla skulls");
		String skullSpecial = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/special/SkullSpecialRenderer.java"));
		assertTrue(skullSpecial.contains("this.texture);"),
			"special skull items must forward their baked semantic texture identity");
		String playerHeadSpecial = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/special/PlayerHeadSpecialRenderer.java"));
		assertTrue(playerHeadSpecial.contains("renderInfo.playerSkin().body().texturePath()")
			&& playerHeadSpecial.contains("defaultTexture(Types.PLAYER)"),
			"player-head items must provide a resolved or default semantic texture identity");
	}
}
