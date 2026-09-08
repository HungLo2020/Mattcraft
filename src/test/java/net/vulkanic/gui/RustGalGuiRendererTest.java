package net.vulkanic.gui;

import net.minecraft.client.gui.font.TextGlyphQuad;
import org.joml.Matrix3x2f;
import org.joml.Matrix4f;
import org.junit.jupiter.api.Test;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import javax.imageio.ImageIO;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

class RustGalGuiRendererTest {
	@Test
	void shippedDefaultPanoramaHasSixRealDimensionMatchedFaces() throws Exception {
		Path root = Path.of("src/main/resources/assets/minecraft/textures/gui/title/background/caves");
		int width = -1;
		int height = -1;
		for (int face = 0; face < 6; face++) {
			var image = ImageIO.read(root.resolve("panorama_" + face + ".png").toFile());
			assertTrue(image != null && image.getWidth() > 1 && image.getHeight() > 1,
				"default panorama face must be a real image, not a placeholder");
			if (face == 0) {
				width = image.getWidth();
				height = image.getHeight();
			} else {
				assertEquals(width, image.getWidth(), "panorama faces must share one width");
				assertEquals(height, image.getHeight(), "panorama faces must share one height");
			}
		}
	}

	@Test
	void semanticBlitPreservesBlitRenderStateCoordinateFieldOrder() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/GuiGraphics.java"));
		int method = source.indexOf("public void submitRustSemanticBlit(");
		int end = source.indexOf("\n\t}\n", method);
		assertTrue(method >= 0 && end > method, "semantic blit callsite must remain present");
		String callsite = source.substring(method, end);
		int u0 = callsite.indexOf("\n\t\t\tu0,");
		int u1 = callsite.indexOf("\n\t\t\tu1,", u0);
		int v0 = callsite.indexOf("\n\t\t\tv0,", u1);
		int v1 = callsite.indexOf("\n\t\t\tv1,", v0);
		assertTrue(u0 >= 0 && u1 > u0 && v0 > u1 && v1 > v0,
			"BlitRenderState requires u0,u1,v0,v1 even though the public semantic API is u0,v0,u1,v1");
	}

	@Test
	void panoramaCubeUvMatchesFrozenTransposedModelViewRotation() {
		float[] identity = RustGalPanoramaRenderer.cubeUv(new Matrix4f(), 0.0F, 0.0F, 1.0F, 1.0F);
		assertEquals(5.5F / 6.0F, identity[1], 0.000001F);

		float[] quarterTurn = RustGalPanoramaRenderer.cubeUv(
			new Matrix4f().rotationY((float)(Math.PI * 0.5)), 0.0F, 0.0F, 1.0F, 1.0F
		);
		assertEquals(0.5F / 6.0F, quarterTurn[1], 0.000001F,
			"Frozen panorama.vsh applies transpose(mat3(ModelViewMat)) before the shared face selector");
	}

	@Test
	void panoramaUsesDedicatedNoCullNoDepthSemanticMaterial() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/vulkanic/gui/RustGalPanoramaRenderer.java"));
		assertTrue(source.contains("GUI_PANORAMA.order(), 0, VulkanicGalBridge.GUI_MESH_MATERIAL_PANORAMA, 1"),
			"Frozen's panorama pipeline has no culling or depth test, so it must not use the generic opaque-item material");
	}

	@Test
	void panoramaFragmentSamplingRetainsFrozenCubeFaceEdgeFiltering() throws Exception {
		String source = Files.readString(Path.of("src/main/rust/render/vulkanic/gui_mesh_frontend.rs"));
		String frozenEdgeSampling = "vec2 atlas_uv = vec2(clamp(u, 0.0, 1.0), (face + clamp(v, 0.0, 1.0)) / 6.0);";
		assertEquals(2, occurrences(source, frozenEdgeSampling),
			"both Rust panorama backends must retain Frozen's continuous cube-face edge sampling");
		assertFalse(source.contains("0.5 / float(size.x)"),
			"a half-texel inset changes Frozen's panorama sampling contract at cube-face boundaries");
	}

	private static int occurrences(String source, String needle) {
		int count = 0;
		int from = 0;
		while ((from = source.indexOf(needle, from)) >= 0) {
			count++;
			from += needle.length();
		}
		return count;
	}

	@Test
	void regularBlitRepeatIntervalsSplitIntoBoundedUnitUvs() {
		List<float[]> segments = RustGalGuiRenderer.wrappedUnitIntervalSegments(0.0F, 26.6875F);
		assertEquals(27, segments.size(), "a 32-pixel separator stretched to 854 pixels needs bounded repeated-image segments");
		assertEquals(0.0F, segments.getFirst()[2], 0.000001F);
		assertEquals(1.0F, segments.getFirst()[3], 0.000001F);
		assertEquals(0.6875F, segments.getLast()[3] - segments.getLast()[2], 0.000001F);
	}

	@Test
	void wrappedRegularBlitsPreserveSourceLayerSchedulingAndAtomicAdmission() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"));
		int helper = source.indexOf("private static List<RustGalGuiElementRenderState> enqueueWrappedAffineAsset(");
		int preflight = source.indexOf("// Fully preflight all expanded records", helper);
		int batch = source.indexOf("enqueueGuiAffineQuadRequests(\n\t\t\trequests, dynamicLayerId(dynamicLayerOrder), semanticLayerOrder", helper);
		int stage = source.indexOf("RustGalGuiRawImageAssets.stage(asset)", batch);
		assertTrue(helper >= 0 && preflight > helper && batch > preflight && stage > batch,
			"a wrapped BlitRenderState must retain its source-layer order and stage its asset only after complete batched admission");
	}

	@Test
	void tiledBlitsAlsoUseOnePreflightedSemanticBatch() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"));
		int tiled = source.indexOf("public static List<RustGalGuiElementRenderState> tryEnqueueTiledCopiedBlit(");
		int preflight = source.indexOf("new VulkanicGalBridge.GuiTiledQuadRecord(", tiled);
		int batch = source.indexOf("enqueueGuiTiledQuadRequest(", tiled);
		int stage = source.indexOf("RustGalGuiRawImageAssets.stage(asset)", batch);
		assertTrue(tiled >= 0 && preflight > tiled && batch > preflight && stage > batch,
			"the complete immutable tile parent must be queued before its Rust-owned image is staged");
	}

	@Test
	void panoramaAdmissionBoundsInputsBeforeBuildingSemanticMesh() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/vulkanic/gui/RustGalPanoramaRenderer.java"));
		int method = source.indexOf("public static boolean enqueue(");
		int guard = source.indexOf("Float.isFinite(pitchDegrees)", method);
		int mesh = source.indexOf("List.of(", method);
		assertTrue(method >= 0 && guard > method && mesh > guard,
			"Rust panorama admission must reject non-finite camera inputs before semantic mesh construction");
	}

	@Test
	void panoramaUsesFrozenFullscreenTriangleCameraRaysRatherThanATessellatedApproximation() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/vulkanic/gui/RustGalPanoramaRenderer.java"));
		assertTrue(source.contains("List.of(0, 1, 2)"));
		assertTrue(source.contains("panoramaVertex(") && source.contains("cubeRay("));
		assertTrue(source.contains("new float[] {0.0F, 0.0F}, new float[] {ray[1], ray[2]}"),
			"the Rust mesh ABI consumes local UVs, so panorama ray Y/Z must not be placed in diagnostic atlas UVs");
		assertFalse(source.contains("private static final int GRID"),
			"camera rays must replace per-frame panorama tessellation");
	}

	@Test
	void panoramaAttachesItsSemanticTokenToTheActiveGuiRenderStateBeforeStagingAssets() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/vulkanic/gui/RustGalPanoramaRenderer.java"));
		int enqueue = source.indexOf("enqueueGuiMeshItemRequest(");
		int attach = source.indexOf("renderState.submitGuiElement(new RustGalGuiElementRenderState(", enqueue);
		int stage = source.indexOf("RustGalGuiRawImageAssets.stageCubeMap(image)", attach);
		assertTrue(enqueue >= 0 && attach > enqueue && stage > attach,
			"a panorama request must enter the current GuiRenderState so whole-frame Vulkan consumes it exactly once per frame");

		String callsite = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/PanoramaRenderer.java"));
		assertTrue(callsite.contains("this.cubeMap, 10.0F, -this.spin, i, j,")
			&& callsite.contains("guiGraphics.guiRenderState\n\t\t))")
			&& callsite.contains("new net.vulkanic.bridge.VulkanicGalBridge.GuiProjectionRecord("),
			"the title-screen callsite must provide its active GuiRenderState to panorama admission");
	}

	@Test
	void wholeFrameVulkanRetiresCopiedGuiStateAfterSubmission() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/vulkanic/gui/RustGalFrameCoordinator.java"));
		int execute = source.indexOf("executeFrameBatches(window, requests, true");
		int reset = source.indexOf("renderState.reset();", execute);
		int capture = source.indexOf("DeterministicCameraCapture.afterRender", reset);
		assertTrue(execute >= 0 && reset > execute && capture > reset,
			"whole-frame Vulkan must reset copied GUI semantic state after execution and before the next capture boundary");
		String minecraft = Files.readString(Path.of("src/main/java/net/minecraft/client/Minecraft.java"));
		int shell = minecraft.indexOf("if (rustWholeFrameShell)");
		int legacyClear = minecraft.indexOf("command.recording.clear", shell);
		assertTrue(shell >= 0 && legacyClear > shell);
		assertFalse(minecraft.substring(shell, legacyClear).contains("DeterministicCameraCapture.afterRender("),
			"the shell must not double-count the coordinator's post-present capture hook");
	}

	@Test
	void guiRenderStateResetClearsFrameLocalBoundsNavigation() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/render/state/GuiRenderState.java"));
		int reset = source.indexOf("public void reset()");
		int bounds = source.indexOf("this.lastElementBounds = null;", reset);
		int next = source.indexOf("this.nextStratum();", bounds);
		assertTrue(reset >= 0 && bounds > reset && next > bounds,
			"reset must discard the prior frame's bounds before creating the new semantic stratum");
	}

	@Test
	void textQuadUsesMinecraftCornerOrderToBuildTheAffineBasis() {
		TextGlyphQuad quad = new TextGlyphQuad(
			"minecraft:font/default/0", false,
			10.0F, 20.0F,
			10.0F, 30.0F,
			18.0F, 30.0F,
			18.0F, 20.0F,
			0.0F, 0.1F, 0.2F, 0.4F, 0.7F, 0xFFFFFFFF
		);

		var request = RustGalGuiRenderer.transformTextQuad(quad, new Matrix3x2f(), 41L, 320, 180, null);

		assertEquals(10.0F, request.x0());
		assertEquals(20.0F, request.y0());
		assertEquals(18.0F, request.x1(), "U axis must end at the top-right glyph corner");
		assertEquals(20.0F, request.y1());
		assertEquals(10.0F, request.x3(), "V axis must end at the bottom-left glyph corner");
		assertEquals(30.0F, request.y3());
	}

	@Test
	void flatItemFoilUsesTheCopiedGlintAssetAndProjectedUvTransform() throws Exception {
		String source = java.nio.file.Files.readString(java.nio.file.Path.of(
			"src/main/java/net/vulkanic/gui/RustGalGuiItemRenderer.java"
		));
		assertTrue(source.contains("layer.foilType() == ItemStackRenderState.FoilType.STANDARD"));
		assertTrue(source.contains("quad.source().stage()"),
			"flat-item quads must stage their explicitly typed texture source");
		assertTrue(source.contains("new GuiItemTextureSource.Raw(glint)"),
			"glint must remain an explicit copied image source");
		String textureSource = java.nio.file.Files.readString(java.nio.file.Path.of(
			"src/main/java/net/vulkanic/gui/GuiItemTextureSource.java"));
		assertTrue(textureSource.contains("RustGalGuiRawImageAssets.stage(asset)"),
			"typed raw sources must still publish copied pixels");
		assertTrue(textureSource.contains("RustGalFrameCoordinator.stageGuiAtlasReference(region)"),
			"atlas sources must publish declarations, not fabricated copied images");
		assertTrue(source.contains("source.atlasU0()"));
		assertTrue(source.contains("ENCHANTED_GLINT_ITEM"));
		assertTrue(source.contains("specialFoilQuad"));
		assertTrue(source.contains("SPECIAL_FOIL_TEXTURE_SCALE"));
	}

	@Test
	void specialGuiItemsUseOnlyBoundedVanillaModelCopiers() throws Exception {
		String itemSource = java.nio.file.Files.readString(java.nio.file.Path.of(
			"src/main/java/net/vulkanic/gui/RustGalGuiItemRenderer.java"
		));
		String stateSource = java.nio.file.Files.readString(java.nio.file.Path.of(
			"src/main/java/net/minecraft/client/renderer/item/ItemStackRenderState.java"
		));
		assertTrue(itemSource.contains("tryEnqueueSpecialItem"));
		assertTrue(itemSource.contains("TridentSpecialRenderer"));
		assertTrue(itemSource.contains("ChestSpecialRenderer"));
		assertTrue(itemSource.contains("chest.material().texture()"));
		assertTrue(itemSource.contains("HangingSignSpecialRenderer"));
		assertTrue(itemSource.contains("StandingSignSpecialRenderer"));
		assertTrue(itemSource.contains("CopperGolemStatueSpecialRenderer"));
		assertTrue(itemSource.contains("BedSpecialRenderer"));
		assertTrue(itemSource.contains("bed.headModel()"));
		assertTrue(itemSource.contains("ShieldSpecialRenderer"));
		assertTrue(itemSource.contains("Sheets.getShieldMaterial(layer.pattern())"));
		assertTrue(itemSource.contains("BannerSpecialRenderer"));
		assertTrue(itemSource.contains("renderer.standingModel()"));
		assertTrue(itemSource.contains("renderer.standingFlagModel()"));
		assertTrue(itemSource.contains("Sheets.getBannerMaterial(layer.pattern())"));
		assertTrue(itemSource.contains("SkullSpecialRenderer"));
		assertTrue(itemSource.contains("skull.texture()"));
		assertTrue(itemSource.contains("PlayerHeadSpecialRenderer"));
		assertTrue(itemSource.contains("info.playerSkin().body().texturePath()"));
		assertTrue(itemSource.contains("ShulkerBoxSpecialRenderer"));
		assertTrue(itemSource.contains("shulker.material().texture()"));
		assertTrue(itemSource.contains("shulker.orientation().getRotation()"));
		assertTrue(itemSource.contains("tryEnqueueModelPartPip"));
		assertTrue(itemSource.contains("DecoratedPotSpecialRenderer"));
		assertTrue(itemSource.contains("renderer.sideMaterial(decorations.front())"));
		assertTrue(itemSource.contains("special-renderer-unavailable"));
		assertTrue(itemSource.contains("ItemRenderer.ENCHANTED_GLINT_ITEM"));
		assertTrue(itemSource.contains("0xffffffff, 4"));
		assertTrue(itemSource.contains("shield.model(), ItemRenderer.ENCHANTED_GLINT_ITEM"));
		assertTrue(itemSource.contains("flagModel, ItemRenderer.ENCHANTED_GLINT_ITEM"));
		assertTrue(itemSource.contains("skull.model(), ItemRenderer.ENCHANTED_GLINT_ITEM"));
		assertTrue(itemSource.contains("playerHead.model(), ItemRenderer.ENCHANTED_GLINT_ITEM"));
		assertTrue(itemSource.contains("model, ItemRenderer.ENCHANTED_GLINT_ITEM"));
		assertTrue(itemSource.contains("hangingSign.model(), ItemRenderer.ENCHANTED_GLINT_ITEM"));
		assertTrue(itemSource.contains("standingSign.model(), ItemRenderer.ENCHANTED_GLINT_ITEM"));
		assertTrue(itemSource.contains("copperGolem.model(), ItemRenderer.ENCHANTED_GLINT_ITEM"));
		assertTrue(itemSource.contains("new Model.Simple(conduit.model(), RenderType::entitySolid)"));
		assertTrue(itemSource.contains("new Model.Simple(part, RenderType::entitySolid), ItemRenderer.ENCHANTED_GLINT_ITEM"));
		assertTrue(itemSource.contains("foot ? bed.footModel() : bed.headModel(), ItemRenderer.ENCHANTED_GLINT_ITEM"));
		assertTrue(itemSource.contains("model, ItemRenderer.ENCHANTED_GLINT_ITEM"));
		assertTrue(stateSource.contains("forEachSpecialRenderer"));
	}

	@Test
	void crosshairBlitsUseTheExplicitInvertAffineRoute() throws Exception {
		String source = java.nio.file.Files.readString(java.nio.file.Path.of(
			"src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"
		));
		assertTrue(source.contains("blit.pipeline() != RenderPipelines.CROSSHAIR"));
		assertTrue(source.contains("boolean invertBlend = blit.pipeline() == RenderPipelines.CROSSHAIR"));
		assertTrue(source.contains("invertBlend ? GuiRenderStratum.GUI_CROSSHAIR.order()"));
	}

	@Test
	void premultipliedGuiBlitsUseAnExplicitBlendStratum() throws Exception {
		String source = java.nio.file.Files.readString(java.nio.file.Path.of(
			"src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"
		));
		assertTrue(source.contains("blit.pipeline() != RenderPipelines.GUI_TEXTURED_PREMULTIPLIED_ALPHA"));
		assertTrue(source.contains("boolean premultipliedBlend = blit.pipeline() == RenderPipelines.GUI_TEXTURED_PREMULTIPLIED_ALPHA"));
		assertTrue(source.contains("GuiRenderStratum.GUI_PREMULTIPLIED_BLIT.order()"));
	}

	@Test
	void tiledPremultipliedBlitsShareTheRustOwnedBlendStratum() throws Exception {
		String source = Files.readString(Path.of(
			"src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"));
		int tiled = source.indexOf("public static List<RustGalGuiElementRenderState> tryEnqueueTiledCopiedBlit");
		int whitelist = source.indexOf("blit.pipeline() != RenderPipelines.GUI_TEXTURED_PREMULTIPLIED_ALPHA", tiled);
		int stratum = source.indexOf("stratum = GuiRenderStratum.GUI_PREMULTIPLIED_BLIT.order()", tiled);
		assertTrue(tiled >= 0 && whitelist > tiled && stratum > whitelist,
			"tiled premultiplied GUI blits must use the explicit Rust blend stratum");
	}

	@Test
	void tiledAdditiveBlitsShareTheRustOwnedBlendStratum() throws Exception {
		String source = Files.readString(Path.of(
			"src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"));
		int tiled = source.indexOf("public static List<RustGalGuiElementRenderState> tryEnqueueTiledCopiedBlit");
		int whitelist = source.indexOf("blit.pipeline() != RenderPipelines.GUI_NAUSEA_OVERLAY", tiled);
		int stratum = source.indexOf("stratum = GuiRenderStratum.GUI_ADDITIVE_BLIT.order()", tiled);
		assertTrue(tiled >= 0 && whitelist > tiled && stratum > whitelist,
			"tiled additive GUI blits must use the explicit Rust blend stratum");
	}

	@Test
	void tiledGuiBlitsRejectNullSemanticStateBeforeFieldAccess() throws Exception {
		String source = Files.readString(Path.of(
			"src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"));
		int tiled = source.indexOf("public static List<RustGalGuiElementRenderState> tryEnqueueTiledCopiedBlit");
		int guard = source.indexOf("if (blit == null) return null;", tiled);
		int field = source.indexOf("blit.x1()", guard);
		assertTrue(tiled >= 0 && guard > tiled && field > guard,
			"Rust tiled GUI admission must reject null copied state before reading semantic fields");
	}

	@Test
	void tiledGuiBlitsRejectNonFiniteAffinePoseBeforeGeometryAdmission() throws Exception {
		String source = Files.readString(Path.of(
			"src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"));
		int tiled = source.indexOf("public static List<RustGalGuiElementRenderState> tryEnqueueTiledCopiedBlit");
		int poseGuard = source.indexOf("!finiteAffinePose(blit.pose())", tiled);
		int field = source.indexOf("return enqueueTypedTiledBlit(", tiled);
		assertTrue(tiled >= 0 && poseGuard > tiled && field > poseGuard,
			"Rust tiled GUI admission must reject non-finite affine poses before enqueueing geometry");
	}

	@Test
	void copiedGuiBlitsRejectNullStateBeforePipelineOrPoseAccess() throws Exception {
		String source = Files.readString(Path.of(
			"src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"));
		int copied = source.indexOf("public static List<RustGalGuiElementRenderState> tryEnqueueCopiedBlit");
		int guard = source.indexOf("blit == null || blit.pose() == null", copied);
		int field = source.indexOf("blit.pipeline()", guard);
		assertTrue(copied >= 0 && guard > copied && field > guard,
			"Rust copied GUI blits must reject null semantic state before pipeline access");
	}

	@Test
	void affineGuiRoutesRejectNonFinitePoseCoefficients() throws Exception {
		String source = Files.readString(Path.of(
			"src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"));
		int helper = source.indexOf("private static boolean finiteAffinePose(");
		int gradient = source.indexOf("tryEnqueueGradientBlit(");
		int copied = source.indexOf("tryEnqueueCopiedBlit(");
		assertTrue(helper >= 0 && gradient >= 0 && copied >= 0
				&& source.indexOf("!finiteAffinePose(pose)", gradient) > gradient
				&& source.indexOf("!finiteAffinePose(blit.pose())", copied) > copied,
			"Rust affine GUI routes must reject non-finite copied pose matrices");
	}

	@Test
	void voxelMapMaskRejectsDerivedUvScaleOverflow() throws Exception {
		String source = Files.readString(Path.of(
			"src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"));
		int method = source.indexOf("public static List<RustGalGuiElementRenderState> tryEnqueueVoxelMapMask");
		int scale = source.indexOf("float sourceScale = sourceWidth * 0.5F / radius;", method);
		int guard = source.indexOf("!Float.isFinite(radius * mapScale)", method);
		int derivedGuard = source.indexOf("!Float.isFinite(sourceScale)", scale);
		int mesh = source.indexOf("List<VulkanicGalBridge.GuiMeshVertexRecord>", scale);
		assertTrue(method >= 0 && scale > method && guard > method && derivedGuard > scale && mesh > derivedGuard,
			"VoxelMap semantic admission must reject derived UV-scale overflow before mesh construction");
	}

	@Test
	void voxelMapMeshUsesOneGuardedTranslationIntoItsCompactTarget() throws Exception {
		String source = Files.readString(Path.of(
			"src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"));
		int method = source.indexOf("public static List<RustGalGuiElementRenderState> tryEnqueueVoxelMapMask");
		int translation = source.indexOf("position[0] -= left - 1.0F", method);
		int batch = source.indexOf("size + 2, size + 2, 1, 0, 0, 0, 0, 0, localVertices, indices", method);
		assertTrue(method >= 0 && translation > method && batch > translation,
			"VoxelMap mesh vertices must be translated once into the guarded compact Rust target");
	}

	@Test
	void voxelMapMeshUsesVulkanFrontFacingGuiWinding() throws Exception {
		String source = Files.readString(Path.of(
			"src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"));
		int method = source.indexOf("public static List<RustGalGuiElementRenderState> tryEnqueueVoxelMapMask");
		int clockwise = source.indexOf(
			"float[][] corners = {{left, top}, {right, top}, {right, bottom}, {left, bottom}};",
			method);
		assertTrue(method >= 0 && clockwise > method,
			"VoxelMap semantic quads must use clockwise top-left GUI winding so the Vulkan GUI Y reflection keeps them front-facing");
	}

	@Test
	void voxelMapFrameResourceRetainsTransparentCenter() throws Exception {
		try (var stream = RustGalGuiRendererTest.class.getResourceAsStream("/assets/voxelmap/images/squaremap.png")) {
			assertTrue(stream != null);
			var image = javax.imageio.ImageIO.read(stream);
			assertEquals(256, image.getWidth());
			assertEquals(256, image.getHeight());
			assertEquals(0, (image.getRGB(128, 128) >>> 24) & 0xff);
			assertEquals(255, (image.getRGB(10, 128) >>> 24) & 0xff);
		}
	}

	@Test
	void voxelMapOverlaySubmitsItsSemanticMeshElementsToGuiState() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/voxelmap/Map.java"));
		int method = source.indexOf("public boolean renderRustSemanticOverlay");
		int helper = source.indexOf("drawContext.submitRustVoxelMapMask(", method);
		String graphics = Files.readString(Path.of("src/main/java/net/minecraft/client/gui/GuiGraphics.java"));
		int submit = graphics.indexOf("this.guiRenderState.submitGuiElement(element)",
			graphics.indexOf("submitRustVoxelMapMask("));
		assertTrue(method >= 0 && helper > method && submit >= 0,
			"VoxelMap must use the GuiGraphics semantic helper that attaches mesh tokens to GuiRenderState");
	}

	@Test
	void endPortalRejectsNonFiniteAnimatedUvsBeforeBatchAdmission() throws Exception {
		String source = Files.readString(Path.of(
			"src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"));
		int method = source.indexOf("public static List<RustGalGuiElementRenderState> tryEnqueueEndPortal");
		int loop = source.indexOf("for (int vertex = 0; vertex < 4; vertex++)", method);
		int guard = source.indexOf("if (!Float.isFinite(value)) return null;", loop);
		int batch = source.indexOf("batches.add(endPortalBatch", loop);
		assertTrue(method >= 0 && loop > method && guard > loop && batch > guard,
			"End Portal semantic admission must reject animated UV overflow before adding a batch");
	}

	@Test
	void gradientBlitRejectsNonFiniteDerivedCoordinatesBeforeMeshConstruction() throws Exception {
		String source = Files.readString(Path.of(
			"src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"));
		int method = source.indexOf("public static List<RustGalGuiElementRenderState> tryEnqueueGradientBlit");
		int derived = source.indexOf("float x2 = x1 + x3 - x0, y2 = y1 + y3 - y0;", method);
		int guard = source.indexOf("!Float.isFinite(x0)", derived);
		int mesh = source.indexOf("List<VulkanicGalBridge.GuiMeshVertexRecord>", derived);
		assertTrue(method >= 0 && derived > method && guard > derived && mesh > guard,
			"gradient semantic admission must reject affine overflow before mesh construction");
	}

	@Test
	void gradientBlitUsesTheSharedBoundedUvContract() throws Exception {
		String source = Files.readString(Path.of(
			"src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"));
		int method = source.indexOf("public static List<RustGalGuiElementRenderState> tryEnqueueGradientBlit");
		int uv = source.indexOf("u0 < -GUI_UV_OVERLAP_LIMIT", method);
		int asset = source.indexOf("RustGalGuiRawImageAssets.Asset asset", method);
		assertTrue(method >= 0 && uv > method && asset > uv,
			"gradient semantic admission must bound UVs before resolving or staging assets");
	}

	@Test
	void rectangleRoutesRejectNullAndInvalidSemanticStateBeforeExtraction() throws Exception {
		String source = Files.readString(Path.of(
			"src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"));
		int uniform = source.indexOf("tryEnqueueUniformRectangle(\n\t\tColoredRectangleRenderState rectangle");
		int guard = source.indexOf("rectangle == null || guiWidth <= 0", uniform);
		int gradient = source.indexOf("tryEnqueueVerticalGradientRectangle(", uniform);
		int gradientGuard = source.indexOf("rectangle == null || guiWidth <= 0", gradient);
		assertTrue(uniform >= 0 && guard > uniform && gradient >= 0 && gradientGuard > gradient,
			"Rust GUI rectangle routes must reject null/invalid semantic state before extraction");
	}

	@Test
	void nauseaOverlayBlitsUseTheExplicitAdditiveRoute() throws Exception {
		String source = java.nio.file.Files.readString(java.nio.file.Path.of(
			"src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"
		));
		assertTrue(source.contains("blit.pipeline() != RenderPipelines.GUI_NAUSEA_OVERLAY"));
		assertTrue(source.contains("boolean additiveBlend = blit.pipeline() == RenderPipelines.GUI_NAUSEA_OVERLAY"));
		assertTrue(source.contains("additiveBlend ? GuiRenderStratum.GUI_ADDITIVE_BLIT.order()"));
	}

	@Test
	void namedGuiBlendStrataMatchTheRustFrontendOrders() throws Exception {
		assertEquals(790, GuiRenderStratum.GUI_PREMULTIPLIED_BLIT.order());
		assertEquals(795, GuiRenderStratum.GUI_ADDITIVE_BLIT.order());
		String rust = Files.readString(Path.of("src/main/rust/render/vulkanic/gui_frontend.rs"));
		assertTrue(rust.contains("GUI_PREMULTIPLIED_BLIT_STRATUM: u32 = 790")
			&& rust.contains("GUI_ADDITIVE_BLIT_STRATUM: u32 = 795"));
	}

	@Test
	void voxelMapLequalBlitsUseAnExplicitDepthTestedStratum() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"));
		String rust = Files.readString(Path.of("src/main/rust/render/vulkanic/gui_frontend.rs"));
		assertTrue(source.contains("VoxelMapPipelines.GUI_TEXTURED_LESS_OR_EQUAL_DEPTH_PIPELINE"));
		assertTrue(source.contains("GuiRenderStratum.GUI_LEQUAL_DEPTH_BLIT.order()"));
		assertTrue(rust.contains("GUI_LEQUAL_DEPTH_BLIT_STRATUM: u32 = 805"));
		assertTrue(rust.contains("TextureGroup::DynamicLequalDepth"));
		assertTrue(rust.contains("CompareOp::LessOrEqual"));
	}

	@Test
	void tiledGuiUvIntervalsPreserveRepeatedTextureTurns() throws Exception {
		var method = RustGalGuiRenderer.class.getDeclaredMethod("wrappedUnitIntervalSegments", float.class, float.class);
		method.setAccessible(true);
		@SuppressWarnings("unchecked")
		var segments = (java.util.List<float[]>) method.invoke(null, 0.75F, 2.25F);
		assertEquals(3, segments.size());
		assertEquals(0.75F, segments.get(0)[2], 0.0001F);
		assertEquals(0.0F, segments.get(1)[2], 0.0001F);
		assertEquals(0.0F, segments.get(2)[2], 0.0001F);
		assertEquals(1.0F, segments.get(2)[3], 0.0001F);
	}

	@Test
	void mojangLogoUsesTheExistingExplicitAlphaAffineRoute() throws Exception {
		String source = java.nio.file.Files.readString(java.nio.file.Path.of(
			"src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"
		));
		assertTrue(source.contains("blit.pipeline() != RenderPipelines.MOJANG_LOGO"));
	}

	@Test
	void firstPersonScreenEffectsUseTheCopiedAffineTextureRoute() throws Exception {
		String source = java.nio.file.Files.readString(java.nio.file.Path.of(
			"src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"
		));
		assertTrue(source.contains("blit.pipeline() != RenderPipelines.BLOCK_SCREEN_EFFECT"));
		assertTrue(source.contains("blit.pipeline() != RenderPipelines.FIRE_SCREEN_EFFECT"));
		assertTrue(source.contains("first-person block/fire screen effects are ordinary single-sampler"));
	}

	@Test
	void rawGuiImageStagingHasAJavaAndRustAggregateByteBound() throws Exception {
		String coordinator = java.nio.file.Files.readString(java.nio.file.Path.of(
			"src/main/java/net/vulkanic/gui/RustGalFrameCoordinator.java"
		));
		String rust = java.nio.file.Files.readString(java.nio.file.Path.of(
			"src/main/rust/render/vulkanic/gui_frontend.rs"
		));
		assertTrue(coordinator.contains("MAX_PENDING_RAW_IMAGE_BYTES = 256L * 1024L * 1024L"));
		assertTrue(coordinator.contains("projectedBytes = Math.addExact(projectedBytes, asset.pixels().length)"));
		assertTrue(rust.contains("GUI_MAX_RAW_IMAGE_BYTES_TOTAL: usize = 256 * 1024 * 1024"));
		assertTrue(rust.contains("raw GUI image aggregate bytes"));
	}

	@Test
	void semanticGuiEnqueuesRequireAnAdmittedRustRoute() throws Exception {
		String coordinator = java.nio.file.Files.readString(java.nio.file.Path.of(
			"src/main/java/net/vulkanic/gui/RustGalFrameCoordinator.java"
		));
		assertTrue(coordinator.contains("private static void requireRustGuiRoute()"));
		assertTrue(coordinator.contains("if (!RustGalGuiRenderer.currentExecutionRoute().usesRustGui())"));
		assertTrue(coordinator.contains("requireRustGuiRoute();\n\t\tlong lockStartedNanos"));
		assertTrue(coordinator.contains("requireRustGuiRoute();\n\t\tif (semanticLayerId"));
		assertTrue(coordinator.contains("static void stageGuiRawImage")
			&& coordinator.contains("static void stageGuiRawImage(VulkanicGalBridge.GuiRawImageAssetRecord asset) {\n\t\trequireRustGuiRoute();"));
		String assets = java.nio.file.Files.readString(java.nio.file.Path.of(
			"src/main/java/net/vulkanic/gui/RustGalGuiRawImageAssets.java"
		));
		assertTrue(assets.contains("registerDynamicTextureBinding(ResourceLocation identity, DynamicTexture texture) {\n\t\tif (!RustGalGuiRenderer.currentExecutionRoute().usesRustGui())"));
		assertTrue(assets.contains("public static Asset prepareDynamicTexture(DynamicTexture texture) {\n\t\tif (!RustGalGuiRenderer.currentExecutionRoute().usesRustGui())"));
	}

	@Test
	void textHighlightRectanglesUseTheExplicitAdditiveStratum() throws Exception {
		String source = java.nio.file.Files.readString(java.nio.file.Path.of(
			"src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"
		));
		assertTrue(source.contains("rectangle.pipeline() == RenderPipelines.GUI_TEXT_HIGHLIGHT"));
		assertTrue(source.contains("requestLayerOrder = GuiRenderStratum.GUI_ADDITIVE_BLIT.order()"));
	}

	@Test
	void taczGuiSpecialItemsUseBoundedBedrockQuadAdapter() throws Exception {
		String source = java.nio.file.Files.readString(java.nio.file.Path.of(
			"src/main/java/net/vulkanic/gui/RustGalGuiItemRenderer.java"
		));
		assertTrue(source.contains("instanceof TaczGlock17SpecialRenderer"));
		assertTrue(source.contains("TaczGuiSubmitCollector"));
		assertTrue(source.contains("submitTexturedQuads"));
		assertTrue(source.contains("private static final int MAX_QUADS = 4096"));
		assertTrue(source.contains("TACZ GUI semantic capture encountered arbitrary custom geometry"));
		assertTrue(source.contains("minecraft.gui.tacz-bedrock"));
		assertTrue(source.contains("item.pose().m00()"));
		assertTrue(source.contains("glintAsset != null"));
		assertTrue(source.contains("ItemRenderer.SPECIAL_FOIL_TEXTURE_SCALE"));
		assertTrue(source.contains("new VulkanicGalBridge.GuiMeshBatchRecord(layerOrder, records.size(), 4"));
	}

	@Test
	void taczGuiTextureLoadingDoesNotPublishBeforeMeshAdmission() throws Exception {
		String source = Files.readString(Path.of(
			"src/main/java/net/vulkanic/gui/RustGalGuiItemRenderer.java"));
		int load = source.indexOf("loadVanillaResource(texture");
		int enqueue = source.indexOf("enqueueGuiMeshItemRequest(records", load);
		int stage = source.indexOf("RustGalGuiRawImageAssets.stage(batch.asset())", enqueue);
		assertTrue(load >= 0 && enqueue > load && stage > enqueue,
			"TACZ GUI resources must be loaded privately and published after mesh admission");
	}

	@Test
	void dynamicGuiTexturePublishesOnlyAfterAffineAdmission() throws Exception {
		String source = Files.readString(Path.of(
			"src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"));
		int method = source.indexOf("tryEnqueueDynamicTextureQuad(");
		int prepare = source.indexOf("prepareDynamicTexture(texture)", method);
		int enqueue = source.indexOf("enqueueAffineAsset(asset", prepare);
		int stage = source.indexOf("RustGalGuiRawImageAssets.stage(asset)", enqueue);
		assertTrue(method >= 0 && prepare > method && enqueue > prepare && stage > enqueue,
			"dynamic GUI pixels must be prepared privately and staged after affine admission");
	}

	@Test
	void semanticImageResolutionDoesNotPublishDynamicPixels() throws Exception {
		String source = Files.readString(Path.of(
			"src/main/java/net/vulkanic/gui/RustGalGuiItemRenderer.java"));
		int method = source.indexOf("public static long resolveSemanticImage(ResourceLocation sprite)");
		int fallback = source.indexOf("instanceof DynamicTexture dynamic", method);
		assertTrue(method >= 0 && fallback > method);
		assertTrue(source.indexOf("registerDynamicTextureUnstaged(sprite, dynamic)", fallback) > fallback,
			"semantic resolution must bind dynamic sources without publishing frame pixels");
		assertTrue(source.indexOf("prepareDynamicTexture(dynamic)", fallback) > fallback,
			"semantic resolution must privately prepare copied dynamic pixels");
		assertTrue(source.indexOf("registerDynamicTexture(sprite, dynamic)", fallback) < 0,
			"semantic resolution must not use the publishing lifecycle API");
	}

	@Test
	void modelPipRejectsMalformedSemanticInputsBeforeCollection() throws Exception {
		String source = java.nio.file.Files.readString(java.nio.file.Path.of(
			"src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"
		));
		int method = source.indexOf("public static List<RustGalGuiElementRenderState> tryEnqueueModelPip(");
		int collect = source.indexOf("GuiModelPipSemanticCollector.collect", method);
		assertTrue(method >= 0 && collect > method);
		String body = source.substring(method, collect);
		assertTrue(body.contains("model == null || texture == null || pose == null || setup == null"));
		assertTrue(body.contains("x1 <= x0 || y1 <= y0"));
		assertTrue(body.contains("!Float.isFinite(scale) || scale <= 0.0F"));
	}

	@Test
	void modelPipCollectorRejectsInvalidViewportDimensions() throws Exception {
		String source = java.nio.file.Files.readString(java.nio.file.Path.of(
			"src/main/java/net/vulkanic/gui/GuiModelPipSemanticCollector.java"
		));
		assertTrue(source.contains("guiWidth <= 0 || guiHeight <= 0"));
	}

	@Test
	void modelPipUsesTheModelRenderTypeForDefaultMaterialMode() throws Exception {
		String source = java.nio.file.Files.readString(java.nio.file.Path.of(
			"src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"
		));
		assertTrue(source.contains("guiModelMaterialMode(model, texture)"));
		assertTrue(source.contains("model.renderType(texture)"));
		assertTrue(source.contains("if (name.contains(\"cutout\")) return 2"));
		assertTrue(source.contains("BlendFunction.TRANSLUCENT.equals(blend.get()) ? 3"));
	}

	@Test
	void textRouteRejectsNullStateAndInvalidViewportBeforeExtraction() throws Exception {
		String source = java.nio.file.Files.readString(java.nio.file.Path.of(
			"src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"
		));
		int method = source.indexOf("public static List<RustGalGuiElementRenderState> tryEnqueueText(");
		int extraction = source.indexOf("textState.ensurePrepared()", method);
		assertTrue(method >= 0 && extraction > method);
		String body = source.substring(method, extraction);
		assertTrue(body.contains("textState == null || guiWidth <= 0 || guiHeight <= 0"));
		assertTrue(body.contains("!finiteAffinePose(textState.pose)"));
	}

	@Test
	void textRoutesRejectNonFiniteCopiedQuadsBeforeTransform() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"));
		int helper = source.indexOf("private static boolean finiteTextQuad(");
		int textLoop = source.indexOf("if (!finiteTextQuad(quad) || !isParallelogram(quad))");
		int glyphLoop = source.indexOf("if (!finiteTextQuad(quad) || !isParallelogram(quad))", textLoop + 1);
		assertTrue(helper >= 0 && textLoop >= 0 && glyphLoop > textLoop
			&& source.indexOf("transformTextQuad(quad", textLoop) > textLoop,
			"Rust text routes must reject non-finite copied glyph geometry before affine transformation");
	}

	@Test
	void modelPipCollectorRejectsUnknownMaterialModes() throws Exception {
		String source = java.nio.file.Files.readString(java.nio.file.Path.of(
			"src/main/java/net/vulkanic/gui/GuiModelPipSemanticCollector.java"
		));
		assertTrue(source.contains("materialMode < 1 || materialMode > 4"));
	}

	@Test
	void modelPipStagesAssetsOnlyAfterSuccessfulCapture() throws Exception {
		String collector = Files.readString(Path.of(
			"src/main/java/net/vulkanic/gui/GuiModelPipSemanticCollector.java"));
		String renderer = Files.readString(Path.of(
			"src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"));
		int capture = collector.indexOf("model.renderToBuffer");
		int result = collector.indexOf("List.of(asset)", capture);
		int enqueue = renderer.indexOf("enqueueGuiMeshItemRequest");
		int stage = renderer.indexOf("RustGalGuiRawImageAssets.stage(asset)", enqueue);
		assertTrue(capture >= 0 && result > capture && enqueue >= 0 && stage > enqueue,
			"model PIP assets must be published by the caller only after mesh admission");
	}

	@Test
	void bannerPipPreflightsAllPatternTexturesBeforeCapture() throws Exception {
		String source = Files.readString(Path.of(
			"src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"
		));
		int method = source.indexOf("tryEnqueueBannerPip(");
		int preflight = source.indexOf("texture-preflight", method);
		int capture = source.indexOf("GuiModelPipSemanticCollector.collect(", method);
		assertTrue(method >= 0 && preflight > method && capture > preflight,
			"banner PIP must resolve every copied pattern texture before model capture");
	}

	@Test
	void bannerPipStagesAllCopiedAssetsAfterCombinedMeshAdmission() throws Exception {
		String source = Files.readString(Path.of(
			"src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"));
		int method = source.indexOf("tryEnqueueBannerPip(");
		int collect = source.indexOf("GuiModelPipSemanticCollector.collect(", method);
		int enqueue = source.indexOf("enqueueGuiMeshItemRequest(ordered", collect);
		int stage = source.indexOf("RustGalGuiRawImageAssets.stage(asset)", enqueue);
		assertTrue(method >= 0 && collect > method && enqueue > collect && stage > enqueue,
			"banner PIP assets must publish only after the combined mesh is admitted");
	}

	@Test
	void bakedModelPipUsesCurrentAnimatedSpriteFrame() throws Exception {
		String source = Files.readString(Path.of(
			"src/main/java/net/vulkanic/gui/GuiModelPipSemanticCollector.java"));
		int loop = source.indexOf("for (BakedQuad quad : quads)");
		int resolve = source.indexOf("resolveAnimatedSprite(view.getSprite())", loop);
		assertTrue(loop >= 0 && resolve > loop,
			"baked model PIPs must use the current copied animated-sprite frame");
	}

	@Test
	void bakedQuadGlintStagesAfterQuadValidation() throws Exception {
		String source = Files.readString(Path.of(
			"src/main/java/net/vulkanic/gui/GuiModelPipSemanticCollector.java"
		));
		int glintResolve = source.indexOf("glint = RustGalGuiRawImageAssets.resolve");
		int loop = source.indexOf("for (BakedQuad quad : quads)", glintResolve);
		int result = source.indexOf("List.of(glint)", loop);
		assertTrue(glintResolve >= 0 && loop > glintResolve && result > loop
			&& !source.contains("RustGalGuiRawImageAssets.stage(glint)"),
			"baked-quad glint assets must be returned for post-admission staging");
	}

	@Test
	void entityPipEnergySwirlLayersUseCopiedTextureOffsets() throws Exception {
		String renderer = java.nio.file.Files.readString(java.nio.file.Path.of(
			"src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"
		));
		String collector = java.nio.file.Files.readString(java.nio.file.Path.of(
			"src/main/java/net/vulkanic/gui/GuiModelPipSemanticCollector.java"
		));
		assertTrue(renderer.contains("submitAnimatedModelSemanticTexture")
			&& renderer.contains("collectAnimated")
			&& renderer.contains("layerModel.uvOffsetU()"),
			"entity PIP energy-swirl layers must enter the semantic animated-model collector");
		assertTrue(collector.contains("uvOffsetU") && collector.contains("this.u=u + uvOffsetU"),
			"animated PIP UV offsets must be copied into Rust mesh vertices rather than relying on a Java texture matrix");
	}

	@Test
	void entityPipAdmissionRequiresMatchingLivingRendererState() throws Exception {
		String renderer = java.nio.file.Files.readString(java.nio.file.Path.of(
			"src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"
		));
		assertTrue(renderer.contains("!(renderer instanceof LivingEntityRenderer<?, ?, ?>)")
			&& renderer.contains("!(entityPip.renderState() instanceof LivingEntityRenderState)"),
			"entity PIP extraction must reject renderer/state mismatches before semantic collection");
	}

	@Test
	void entityPipCaptureCannotLeakBlockSubmitsIntoWorldStorage() throws Exception {
		String renderer = java.nio.file.Files.readString(java.nio.file.Path.of(
			"src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"
		));
		int capture = renderer.indexOf("private static final class EntityPipLayerCapture");
		assertTrue(capture >= 0);
		String body = renderer.substring(capture);
		assertTrue(body.contains("public void submitBlock(PoseStack poseStack")
			&& body.contains("public void submitBlockDisplay(PoseStack poseStack")
			&& body.contains("public void submitBlockModel(PoseStack poseStack")
			&& body.contains("public void submitMovingBlock(PoseStack poseStack")
			&& body.contains("public void submitHitbox(PoseStack poseStack")
			&& body.contains("public void submitNameTag(PoseStack poseStack")
			&& body.contains("public void submitText(PoseStack poseStack")
			&& body.contains("public void submitFlame(PoseStack poseStack")
			&& body.contains("public void submitLeash(PoseStack poseStack")
			&& body.contains("public void submitParticleGroup(SubmitNodeCollector.ParticleGroupRenderer"),
			"GUI entity capture must reject world-only block families instead of inheriting world storage");
	}

	@Test
	void modelPipUsesMinecraftNoOverlayConstant() throws Exception {
		String source = java.nio.file.Files.readString(java.nio.file.Path.of(
			"src/main/java/net/vulkanic/gui/GuiModelPipSemanticCollector.java"
		));
		assertTrue(source.contains("OverlayTexture.NO_OVERLAY"));
		assertTrue(!source.contains("OverlayTextureNoOverlay"));
	}

	@Test
	void endPortalStagesAssetsOnlyAfterMeshAcceptance() throws Exception {
		String source = Files.readString(Path.of(
			"src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"
		));
		int method = source.indexOf("tryEnqueueEndPortal(");
		int enqueue = source.indexOf("enqueueGuiMeshItemRequest(", method);
		int stageSky = source.indexOf("RustGalGuiRawImageAssets.stage(sky)", enqueue);
		int stagePortal = source.indexOf("RustGalGuiRawImageAssets.stage(portal)", enqueue);
		assertTrue(method >= 0 && enqueue > method && stageSky > enqueue && stagePortal > enqueue,
			"End Portal assets must stage only after its complete mesh request is accepted");
	}

	@Test
	void panoramaStagesCubeMapOnlyAfterMeshAcceptance() throws Exception {
		String source = Files.readString(Path.of(
			"src/main/java/net/vulkanic/gui/RustGalPanoramaRenderer.java"
		));
		int enqueue = source.indexOf("enqueueGuiMeshItemRequest(");
		int stage = source.indexOf("RustGalGuiRawImageAssets.stageCubeMap(image)");
		assertTrue(enqueue >= 0 && stage > enqueue,
			"panorama cube-map assets must stage only after the bounded mesh request is accepted");
	}

	@Test
	void panoramaFaceResolutionRetainsBoundedVanillaClasspathFallback() throws Exception {
		String source = Files.readString(Path.of("src/main/java/net/vulkanic/gui/RustGalGuiRawImageAssets.java"));
		int method = source.indexOf("private static Asset resolveCubeFace");
		int nextMethod = source.indexOf("\n\t/**", method + 1);
		String body = source.substring(method, nextMethod < 0 ? source.length() : nextMethod);
		assertTrue(body.contains("getResourceAsStream(classpathName)"));
		assertTrue(body.contains("\"minecraft\".equals(source.getNamespace())"));
		assertTrue(body.contains("MAX_DECODED_PIXELS / 6"));
	}

	@Test
	void guiRawImageInvalidationPublishesAnEmptyReplacementGeneration() throws Exception {
		String assets = Files.readString(Path.of(
			"src/main/java/net/vulkanic/gui/RustGalGuiRawImageAssets.java"));
		String coordinator = Files.readString(Path.of(
			"src/main/java/net/vulkanic/gui/RustGalFrameCoordinator.java"));
		assertTrue(assets.contains("RustGalFrameCoordinator.invalidateGuiRawImages()"));
		int clear = coordinator.indexOf("pendingRawImages.clear()");
		int generation = coordinator.indexOf("rawImageGeneration++", clear);
		assertTrue(clear >= 0 && generation > clear,
			"GUI raw-image invalidation must clear pending payloads and advance their generation");
	}

	@Test
	void guiImageFastPathCommitsOnlyAfterCoordinatorAdmission() throws Exception {
		String source = Files.readString(Path.of(
			"src/main/java/net/vulkanic/gui/RustGalGuiRawImageAssets.java"));
		int submit = source.indexOf("RustGalFrameCoordinator.stageGuiRawImage(");
		int commit = source.indexOf("STAGED_ASSETS.put(asset.assetId(), asset)", submit);
		assertTrue(submit >= 0 && commit > submit,
			"GUI fast-path staging must commit only after coordinator admission");
		int cube = source.indexOf("static void stageCubeMap(Asset asset)");
		int cubeSubmit = source.indexOf("stage(asset)", cube);
		int cubeCommit = source.indexOf("STAGED_CUBEMAP_ASSETS.add", cubeSubmit);
		assertTrue(cubeSubmit >= 0 && cubeCommit > cubeSubmit,
			"cube-map staging must commit its identity only after image admission");
	}

	@Test
	void resourceAffineGuiAssetsStageOnlyAfterGeometryAdmission() throws Exception {
		String source = Files.readString(Path.of(
			"src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"));
		int helper = source.indexOf("private static List<RustGalGuiElementRenderState> enqueueAffineAsset(");
		int admissible = source.indexOf("if (!admissibleAffineQuad", helper);
		int stage = source.indexOf("RustGalGuiRawImageAssets.stage(asset)", admissible);
		assertTrue(helper >= 0 && admissible > helper && stage > admissible,
			"resource-backed GUI assets must not stage before semantic geometry admission");
	}

	@Test
	void solidGuiRectanglesStageWhiteTexelOnlyAfterMeshAdmission() throws Exception {
		String source = Files.readString(Path.of(
			"src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"));
		int four = source.indexOf("tryEnqueueFourColoredRectangle");
		int fourEnqueue = source.indexOf("enqueueGuiMeshItemRequest(", four);
		int fourStage = source.indexOf("stageGuiRawImage(", fourEnqueue);
		int uniform = source.indexOf("tryEnqueueUniformRectangle(", four + 1);
		int uniformEnqueue = source.indexOf("enqueueGuiAffineQuadRequest(", uniform);
		int uniformStage = source.indexOf("stageGuiRawImage(", uniformEnqueue);
		assertTrue(four >= 0 && fourEnqueue > four && fourStage > fourEnqueue,
			"four-corner GUI meshes must stage the shared texel only after scheduler admission");
		assertTrue(uniform >= 0 && uniformEnqueue > uniform && uniformStage > uniformEnqueue,
			"uniform GUI rectangles must stage the shared texel only after scheduler admission");
	}

	@Test
	void textAssetsStageOnlyAfterAffineBatchAdmission() throws Exception {
		String source = Files.readString(Path.of(
			"src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"));
		int helper = source.indexOf("tryEnqueueText(");
		int enqueue = source.indexOf("enqueueGuiAffineQuadRequests(", helper);
		int atlasStage = source.indexOf("stageTextAtlas(", enqueue);
		int rawStage = source.indexOf("RustGalGuiRawImageAssets.stage(raw)", enqueue);
		assertTrue(helper >= 0 && enqueue > helper && atlasStage > enqueue && rawStage > enqueue,
			"text atlas and raw-image assets must commit only after the semantic text batch is admitted");
	}

	@Test
	void taczGuiImagesStageOnlyAfterMeshAdmission() throws Exception {
		String source = Files.readString(Path.of(
			"src/main/java/net/vulkanic/gui/RustGalGuiItemRenderer.java"));
		int enqueue = source.indexOf("enqueueGuiMeshItemRequest(records");
		int batchStage = source.indexOf("RustGalGuiRawImageAssets.stage(batch.asset())", enqueue);
		int glintStage = source.indexOf("RustGalGuiRawImageAssets.stage(glintAsset)", enqueue);
		assertTrue(enqueue >= 0 && batchStage > enqueue && glintStage > enqueue,
			"TACZ GUI images must commit only after its complete semantic mesh is admitted");
	}

	@Test
	void affineGuiAssetsDeclineNegativeSemanticLayerIds() throws Exception {
		String source = Files.readString(Path.of(
			"src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"));
		int helper = source.indexOf("private static List<RustGalGuiElementRenderState> enqueueAffineAsset(");
		int guard = source.indexOf("asset == null || dynamicLayerOrder < 0", helper);
		int order = source.indexOf("dynamicLayerOrder(dynamicLayerOrder)", guard);
		assertTrue(helper >= 0 && guard > helper && order > guard,
			"affine GUI asset admission must decline negative semantic layers before ordering");
	}

	@Test
	void borrowedOpenGlRustGuiMarkerAcquiresTheGuiTargetWithoutJavaDrawing() throws Exception {
		String source = Files.readString(Path.of(
			"src/main/java/net/minecraft/client/gui/render/GuiRenderer.java"));
		int marker = source.indexOf("if (step instanceof GuiRenderer.RustGalDraw)");
		int coordinator = source.indexOf("RustGalFrameCoordinator.executeGuiFrame", marker);
		int nextRange = source.indexOf("rustGalFrameExecuted.setTrue()", coordinator);
		String rustRange = source.substring(marker, nextRange < 0 ? source.length() : nextRange);
		assertTrue(coordinator > marker);
		assertTrue(rustRange.contains("Rust GAL GUI borrowed OpenGL target scope"),
			"the borrowed OpenGL route must acquire the main GUI attachment before Rust samples current GL state");
		assertTrue(rustRange.contains("renderTarget.getColorTextureView()"));
		assertTrue(rustRange.contains("renderTarget.useDepth ? renderTarget.getDepthTextureView() : null"));
		assertTrue(rustRange.contains("RustGalFrameCoordinator.executeGuiFrame(minecraft, rustGalDrawGroup)"));
		assertFalse(rustRange.contains("targetScope.setPipeline(") || rustRange.contains("targetScope.draw("),
			"the target scope must not become a hidden Java GUI draw path");
	}

	@Test
	void loadingScreenUsesRustSemanticGridAndProgressRectanglesOnBorrowedOpenGl() throws Exception {
		String loading = Files.readString(Path.of(
			"src/main/java/net/minecraft/client/gui/screens/LevelLoadingScreen.java"));
		String renderer = Files.readString(Path.of(
			"src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"));
		assertTrue(loading.contains("currentExecutionRoute().usesRustGui()"),
			"the loading-screen progress bar must select the admitted Rust GUI route");
		assertTrue(loading.contains("Rust OpenGL loading progress semantic admission failed"));
		assertTrue(loading.contains("tryEnqueueUniformRectangle(")
			&& loading.contains("filledWidth == 0")
			&& loading.contains("Mth.clamp(f, 0.0F, 1.0F)"),
			"the loading bar must carry bounded semantic background/fill rectangles and omit a degenerate zero-width fill");
		assertTrue(renderer.contains("if (!currentExecutionRoute().usesRustGui()")
			&& renderer.contains("LOADING_GRID_PRODUCER"),
			"the chunk-status grid must admit the borrowed OpenGL route through Rust semantic rendering");
		assertTrue(renderer.contains("if (currentExecutionRoute().usesRustGui())")
			&& renderer.contains("enqueueGuiAffineQuadRequests(")
			&& renderer.contains("ARGB.opaque(colors[row * gridSize + column])")
			&& renderer.contains("SOLID_WHITE_ASSET_ID"),
			"Rust OpenGL and Rust Vulkan loading grids must preserve row-major semantic status colors through the Rust affine path rather than an unavailable storage-buffer mesh shader or an opaque image shortcut");
	}

	@Test
	void loadingGridPackedImageHasABoundedEdgeBeforeAllocation() throws Exception {
		String renderer = Files.readString(Path.of(
			"src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"));
		int bound = renderer.indexOf("MAX_LOADING_GRID_TEXTURE_EDGE");
		int extent = renderer.indexOf("extent > MAX_LOADING_GRID_TEXTURE_EDGE", bound);
		int allocation = renderer.indexOf("new byte[Math.multiplyExact(Math.multiplyExact(imageWidth, imageHeight), 4)]", extent);
		assertTrue(bound >= 0 && extent > bound && allocation > extent,
			"loading-grid packed images must reject oversized extents before allocating RGBA storage");
	}

	@Test
	void loadingGridUsesExplicitPerCellGeometryByDefault() throws Exception {
		String renderer = Files.readString(Path.of(
			"src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"));
		assertTrue(renderer.contains("System.getProperty(\"mattmc.dev.rustGalGui.loadingGridTexture\", \"false\")"),
			"the retained packed-image experiment must remain opt-in only");
		int packedPath = renderer.indexOf("mattmc.dev.rustGalGui.loadingGridTexture");
		int explicitAffine = renderer.indexOf("if (currentExecutionRoute().usesRustGui())");
		assertTrue(explicitAffine >= 0 && explicitAffine < packedPath,
			"the default loading-grid route must use bounded explicit per-cell affine geometry before any optional sampled-image experiment");
	}

	@Test
	void loadingGridResidencyIsInvalidatedWithGuiFrontendLifecycle() throws Exception {
		String renderer = Files.readString(Path.of(
			"src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"));
		String coordinator = Files.readString(Path.of(
			"src/main/java/net/vulkanic/gui/RustGalFrameCoordinator.java"));
		int invalidator = renderer.indexOf("public static void invalidateLoadingGridAsset()");
		int residentReset = renderer.indexOf("loadingGridAssetResident = false", invalidator);
		int reload = coordinator.indexOf("public static void reload(ResourceManager resourceManager)");
		int reloadCall = coordinator.indexOf("RustGalGuiRenderer.invalidateLoadingGridAsset();", reload);
		int shutdown = coordinator.indexOf("public static void shutdown()");
		int shutdownCall = coordinator.indexOf("RustGalGuiRenderer.invalidateLoadingGridAsset();", shutdown);
		assertTrue(invalidator >= 0 && residentReset > invalidator
			&& reloadCall > reload && shutdownCall > shutdown,
			"loading-grid residency must be invalidated whenever the Rust GUI frontend is rebuilt");
	}

	@Test
	void loadingGridContentChangesStageOnTheFirstChangedFrame() throws Exception {
		String renderer = Files.readString(Path.of(
			"src/main/java/net/vulkanic/gui/RustGalGuiRenderer.java"));
		int resident = renderer.indexOf("if (!loadingGridAssetResident || loadingGridAssetHash != assetHash");
		assertTrue(resident >= 0,
			"loading-grid content changes must invalidate the packed asset immediately");
		assertTrue(!renderer.substring(resident, Math.min(renderer.length(), resident + 240))
			.contains("framesSinceUpload >= 120"),
			"loading-grid status updates must not wait an arbitrary 120-frame delay");
	}

	@Test
	void failedGuiAssetGenerationsRemainRetryable() throws Exception {
		String source = Files.readString(Path.of(
			"src/main/java/net/vulkanic/gui/RustGalFrameCoordinator.java"));
		int atlasCatch = source.indexOf("assetUpdateFailures++");
		int atlasRetry = source.indexOf("attemptedAssetGeneration = uploadedAssetGeneration", atlasCatch);
		int rawCatch = source.indexOf("assetUpdateFailures++", atlasRetry);
		int rawRetry = source.indexOf("attemptedRawImageGeneration = uploadedRawImageGeneration", rawCatch);
		assertTrue(atlasCatch >= 0 && atlasRetry > atlasCatch,
			"failed GUI atlas updates must remain retryable");
		assertTrue(rawCatch >= 0 && rawRetry > rawCatch,
			"failed GUI raw-image updates must remain retryable");
	}
}
