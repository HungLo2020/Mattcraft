package net.vulkanic.world;

import net.sodium.client.render.chunk.vertex.format.NativeSectionMeshBuilder;
import net.sodium.client.render.chunk.RenderSection;
import net.sodium.client.render.chunk.data.BuiltSectionInfo;
import net.vulkanic.bridge.VulkanicGalBridge;
import org.junit.jupiter.api.Test;

import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import java.nio.file.Files;
import static org.junit.jupiter.api.Assertions.assertFalse;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertArrayEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

public class RustGalTerrainRendererLightingContractTest {
	@Test
	void specularResourceSuffixPreservesResourceLocationExtensions() {
		assertEquals("block/sand_s", RustGalTerrainRenderer.appendPbrSuffix("block/sand", "_s"));
		assertEquals("optifine/cit/metal_s.png", RustGalTerrainRenderer.appendPbrSuffix("optifine/cit/metal.png", "_s"));
		assertEquals("custom/stone_s.png", RustGalTerrainRenderer.appendPbrSuffix("custom/stone.png", "_s"));
	}

	@Test
	void preservesIrisPackedBlockMaterialSemantics() {
		int packed = (10200 + 1 << 1) | 1;
		assertEquals(10200, RustGalTerrainRenderer.decodeIrisShaderBlockId(packed));
		assertEquals(1, RustGalTerrainRenderer.decodeIrisShaderRenderType(packed));
	}

	@Test
	void preservesSodiumCompactTerrainMaterialByte() {
		assertEquals(0b101, RustGalTerrainRenderer.decodeTerrainMaterialBits(0x0005_0000));
		assertEquals(0xff, RustGalTerrainRenderer.decodeTerrainMaterialBits(0x00ff_0000));
		assertEquals(0, RustGalTerrainRenderer.decodeTerrainMaterialBits(0xff00_0000));
	}

	@Test
	void shaderPackDepthFarUsesEffectiveRenderDistanceScalar() {
		String source;
		try {
			source = Files.readString(Path.of(
				"src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		} catch (java.io.IOException error) {
			throw new AssertionError("unable to read Rust world renderer source", error);
		}
		assertTrue(source.contains("shaderPackDepthFarForRenderDistance("));
		assertTrue(source.contains("Math.max(1.0F, effectiveRenderDistance * 16.0F)"));
	}

	@Test
	public void compactTerrainColorConvertsSodiumAbgrToSemanticArgb() {
		int compactAbgr = 0x80402010;

		assertEquals(0x80102040, RustGalTerrainRenderer.decodeCompactTerrainColorForRust(compactAbgr, false));
	}

	@Test
	public void separateAoTerrainColorConsumesAlphaAsAmbientOcclusion() {
		int compactAbgr = 0x80402010;

		assertEquals(0xff081020, RustGalTerrainRenderer.decodeCompactTerrainColorForRust(compactAbgr, true));
	}

	@Test
	public void fullSeparateAoLeavesOpaqueWhiteVertexWhite() {
		int compactAbgr = 0xffffffff;

		assertEquals(0xffffffff, RustGalTerrainRenderer.decodeCompactTerrainColorForRust(compactAbgr, true));
	}

	@Test
	void rustWholeFrameCompactTerrainUsesFrozenBakedColorContract() throws Exception {
		String sectionManager = Files.readString(Path.of(
			"src/main/java/net/sodium/client/render/chunk/RenderSectionManager.java"
		));
		String wholeFrameSource = Files.readString(Path.of(
			"src/main/java/net/vulkanic/world/RustGalWholeFrameTerrainSource.java"
		));
		String terrainRenderer = Files.readString(Path.of(
			"src/main/java/net/vulkanic/world/RustGalTerrainRenderer.java"
		));

		int rustOwnedBuilder = sectionManager.indexOf("this.builder = rustVulkanOwned");
		assertTrue(rustOwnedBuilder >= 0);
		int builderEnd = sectionManager.indexOf(": new ChunkBuilder(level, vertexType);", rustOwnedBuilder);
		String builderBranch = sectionManager.substring(rustOwnedBuilder, builderEnd);
		assertTrue(builderBranch.contains("new ChunkBuilder(level, vertexType, false, 1)"),
			"the direct vanilla Rust route must bake AO and directional face shade into compact RGB");
		assertTrue(!builderBranch.contains("new ChunkBuilder(level, vertexType, true, 1)"));
		assertTrue(wholeFrameSource.contains(
			"new ChunkBuilder(level, ChunkMeshFormats.COMPACT, false, semanticMeshWorkerCount())"
		), "the independent Rust whole-frame terrain producer must use the same baked-RGB compact contract");
		assertTrue(!wholeFrameSource.contains(
			"new ChunkBuilder(level, ChunkMeshFormats.COMPACT, true, semanticMeshWorkerCount())"
		), "separate-AO compact meshes are incompatible with the direct vanilla Rust terrain consumer");

		int compactLayout = terrainRenderer.indexOf("private static TerrainMeshLayout compact()");
		assertTrue(compactLayout >= 0);
		String compactLayoutBody = terrainRenderer.substring(compactLayout,
			terrainRenderer.indexOf("private static TerrainMeshLayout activeIrisCompatible()", compactLayout));
		assertTrue(compactLayoutBody.contains("new TerrainMeshLayout(COMPACT_PREFIX_STRIDE, false, 0, 0)"));
	}

	@Test
	void wholeFrameTerrainSnapshotDeduplicatesCanonicalSectionPositions() {
		RenderSection built = new RenderSection(null, 3, 4, 5);
		built.setInfo(BuiltSectionInfo.EMPTY);
		RenderSection duplicatePosition = new RenderSection(null, 3, 4, 5);
		duplicatePosition.setInfo(BuiltSectionInfo.EMPTY);
		RenderSection unbuilt = new RenderSection(null, 9, 9, 9);

		List<RenderSection> snapshot = RustGalTerrainRenderer.snapshotBuiltTerrainSections(
			Arrays.asList(null, built, duplicatePosition, built, unbuilt, null)
		);

		assertEquals(List.of(built), snapshot,
			"semantic terrain extraction must submit each canonical section position once");
		assertEquals(List.of(), RustGalTerrainRenderer.snapshotBuiltTerrainSections(null));
	}

	@Test
	void wholeFrameTerrainSnapshotRejectsMoreThanRustResidencyBound() {
		List<RenderSection> sections = new ArrayList<>(4097);
		for (int index = 0; index < 4097; index++) {
			RenderSection section = new RenderSection(null, index, 0, 0);
			section.setInfo(BuiltSectionInfo.EMPTY);
			sections.add(section);
		}

		assertThrows(IllegalStateException.class,
			() -> RustGalTerrainRenderer.snapshotBuiltTerrainSections(sections),
			"semantic terrain extraction must fail closed before exceeding Rust section residency");
	}

	@Test
	public void compactTerrainAtlasCoordinatesRemainTopOriginForCopiedAtlas() {
		int copiedAtlasV = Math.round(0.710938F * (1 << 15));
		float decoded = RustGalTerrainRenderer.decodeTexture(copiedAtlasV);

		assertEquals(0.710938F, decoded, 1.0F / (1 << 15));
		assertTrue(Math.abs(decoded - (1.0F - 0.710938F)) > 0.2F,
			"a vertically mirrored compact V coordinate selects an unrelated copied-atlas row");
	}

	@Test
	public void compactTerrainAtlasCoordinatesPreserveSodiumSubTexelDirection() {
		int atlasExtent = 1024;
		int lowDirection = 16_384;
		int highDirection = lowDirection | 0x8000;
		float shrink = (1.0F / (1 << 15)) - (1.0F / (atlasExtent * 256.0F));

		assertEquals(RustGalTerrainRenderer.decodeTexture(lowDirection) - shrink,
			RustGalTerrainRenderer.decodeTextureForCopiedAtlas(lowDirection, atlasExtent), 1.0e-7F);
		assertEquals(RustGalTerrainRenderer.decodeTexture(highDirection) + shrink,
			RustGalTerrainRenderer.decodeTextureForCopiedAtlas(highDirection, atlasExtent), 1.0e-7F);
	}

	@Test
	public void copiedAtlasTracksSemanticAnimationGenerationAndSelectedFrame() throws Exception {
		String source = java.nio.file.Files.readString(java.nio.file.Path.of(
			"src/main/java/net/vulkanic/world/RustGalTerrainRenderer.java"
		));
		int method = source.indexOf("private static void ensureAtlasPayload()");
		int nextMethod = source.indexOf("\n\tprivate static FluidSpriteAsset buildFluidSpriteAsset", method + 1);
		String body = source.substring(method, nextMethod < 0 ? source.length() : nextMethod);
		assertTrue(body.contains("semanticRawSnapshot()"));
		assertTrue(body.contains("semanticSnapshotFrameKey()"));
		assertTrue(body.contains("copiedAtlasSemanticGeneration == semanticGeneration"));
		assertTrue(body.contains("copiedAtlasSemanticFrameKey == semanticFrameKey"));
		assertTrue(body.contains("snapshotFrameKey"));
		int copySprite = source.indexOf("private static void copySprite");
		String copyBody = source.substring(copySprite, source.indexOf("\n\tprivate static long rgbaHash", copySprite));
		assertTrue(copyBody.contains("contents.semanticFrameIndex()"));
		assertTrue(copyBody.contains("getFrameX(frame) * contents.width()"));
		assertTrue(copyBody.contains("getFrameY(frame) * contents.height()"));
	}

	@Test
	public void copiedPbrAtlasIsBoundedBeforeBaseAndDerivedImageAllocation() throws Exception {
		String source = java.nio.file.Files.readString(java.nio.file.Path.of(
			"src/main/java/net/vulkanic/world/RustGalTerrainRenderer.java"
		));
		assertTrue(source.contains("MAX_RUST_ATLAS_PIXELS = 16_777_216L"));
		int ensure = source.indexOf("private static void ensureAtlasPayload()");
		int ensureImage = source.indexOf("new BufferedImage(atlas.width, atlas.height", ensure);
		assertTrue(ensure >= 0 && ensureImage > ensure);
		assertTrue(source.substring(ensure, ensureImage).contains("atlas pixel bound exceeded"));
		int pbr = source.indexOf("private static byte[] buildPbrAtlasPayload");
		int pbrImage = source.indexOf("new BufferedImage(atlas.width, atlas.height", pbr);
		assertTrue(pbr >= 0 && pbrImage > pbr);
		assertTrue(source.substring(pbr, pbrImage).contains("Rust PBR atlas pixel bound exceeded"));
		assertTrue(source.substring(ensure, pbr).contains("hasPbrResources(atlas, \"_n\")"));
		assertTrue(source.substring(ensure, pbr).contains("hasPbrResources(atlas, \"_s\")"));
	}

	@Test
	public void wholeFrameShaderEnvironmentUsesFreshVanillaFogInsteadOfSodiumHookCache() throws Exception {
		String source = java.nio.file.Files.readString(java.nio.file.Path.of(
			"src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"
		));
		int method = source.indexOf("private static FogRenderer.RustFogParameters shaderPackFogParameters(ClientLevel level, Camera camera)");
		assertTrue(method >= 0, "the semantic fog bridge must remain a private Rust-owned record builder");
		int nextMethod = source.indexOf("\n\tprivate static", method + 1);
		String body = source.substring(method, nextMethod < 0 ? source.length() : nextMethod);

		assertTrue(body.contains("collectFogParametersForRust("));
		assertTrue(body.contains("getDeltaTracker()"));
		assertTrue(!body.contains("sodium$getFogParameters()"));
		assertTrue(!body.contains("instanceof FogStorage"));
	}

	@Test
	public void wholeFrameShaderEnvironmentUsesBlockDistanceForDistantHorizonsFog() throws Exception {
		String source = java.nio.file.Files.readString(java.nio.file.Path.of(
			"src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"
		));
		int method = source.indexOf("private static int shaderPackDistantHorizonsRenderDistance()");
		int nextMethod = source.indexOf("\n\tprivate static", method + 1);
		String body = source.substring(method, nextMethod < 0 ? source.length() : nextMethod);

		assertTrue(body.contains("DhApi.Delayed.configs.graphics().chunkRenderDistance().getValue() * 16"));
		assertTrue(body.contains("options.getEffectiveRenderDistance() * 16"));
		assertTrue(body.contains("Math.max(vanillaRenderDistance, distantHorizonsRenderDistance)"));
		assertTrue(!source.contains("DHCompat.getRenderDistance()"));
	}

	@Test
	public void visibleTerrainDoesNotClassifyPendingRustUploadsAsStaleGenerations() throws Exception {
		String source = Files.readString(Path.of(
			"src/main/java/net/vulkanic/world/RustGalTerrainRenderer.java"
		));
		int enqueue = source.indexOf("private static boolean enqueueSectionLayer");
		int submission = source.indexOf("boolean submitted = RustGalWorldPrimitiveRenderer.enqueueStaticTerrainMeshInstance", enqueue);
		assertTrue(enqueue >= 0 && submission > enqueue);
		String body = source.substring(enqueue, submission);
		assertTrue(body.contains("isStaticTerrainMeshGenerationUploaded"));
		assertTrue(body.contains("asset-upload-pending"));
		assertTrue(body.contains("!\"stale-generation\".equals(activeFault())"));
	}

	@Test
	public void staticTerrainPreservesFrozenSodiumBackFaceCullingContract() throws Exception {
		String source = Files.readString(Path.of(
			"src/main/java/net/vulkanic/world/RustGalTerrainRenderer.java"
		));
		int enqueue = source.indexOf("private static boolean enqueueSectionLayer");
		int submission = source.indexOf("boolean submitted = RustGalWorldPrimitiveRenderer.enqueueStaticTerrainMeshInstance", enqueue);
		assertTrue(enqueue >= 0 && submission > enqueue);
		String setup = source.substring(enqueue, submission + 900);
		assertTrue(setup.contains("RustGalWorldPrimitiveRenderer.CULL_BACK"),
				"Frozen Sodium terrain pipelines retain their default back-face culling for every terrain layer");
		assertTrue(!setup.contains("RustGalWorldPrimitiveRenderer.CULL_NONE"),
				"the semantic terrain route must not accumulate both sides of copied glass or fluid faces");

		int assetBuild = source.indexOf("new VulkanicGalBridge.WorldMeshSectionRecord(");
		assertTrue(assetBuild >= 0);
		String firstAssetSection = source.substring(assetBuild, source.indexOf(");", assetBuild) + 2);
		assertTrue(firstAssetSection.contains("RustGalWorldPrimitiveRenderer.CULL_BACK"));
	}

	@Test
	public void staticTerrainDecoderEmitsOneCompleteIndexRangePerVertexSegment() throws Exception {
		String source = Files.readString(Path.of(
			"src/main/java/net/vulkanic/world/RustGalTerrainRenderer.java"
		));
		int decoder = source.indexOf("private static TerrainSectionAsset decodeMesh(");
		int finish = source.indexOf("if (cursor != vertexCount)", decoder);
		assertTrue(decoder >= 0 && finish > decoder);
		String body = source.substring(decoder, finish);
		int quadLoop = body.indexOf("for (int quadBase = cursor");
		int sectionRecord = body.indexOf("sections.add(new VulkanicGalBridge.WorldMeshSectionRecord", quadLoop);
		int cursorAdvance = body.indexOf("cursor += segmentVertexCount", quadLoop);
		assertTrue(quadLoop >= 0 && sectionRecord > quadLoop && cursorAdvance > sectionRecord,
			"all quads in a segment must be indexed before its one material range and cursor advance");
		assertEquals(cursorAdvance, body.lastIndexOf("cursor += segmentVertexCount"),
			"the segment cursor must advance once, not once per quad");
	}

	@Test
	public void uploadedTerrainCanEnqueueAfterCpuPayloadRelease() throws Exception {
		String source = Files.readString(Path.of(
			"src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"
		));
		int method = source.indexOf("public static boolean enqueueStaticTerrainMeshInstance(");
		int lock = source.indexOf("synchronized (LOCK)", method);
		int validation = source.indexOf("boolean generationMatches", lock);
		assertTrue(method >= 0 && lock > method && validation > lock);
		String body = source.substring(lock, source.indexOf("if (!generationMatches)", validation));
		assertTrue(body.contains("STATIC_TERRAIN_MESH_RESIDENCY.get(meshKey)"));
		assertTrue(body.contains("residency.meshGeneration() == meshGeneration"));
	}

	@Test
	public void primitiveMetadataRestoresCompactTerrainShaderSemantics() {
		List<VulkanicGalBridge.WorldMeshVertexRecord> vertices = new ArrayList<>();
		for (int index = 0; index < 4; index++) {
			vertices.add(new VulkanicGalBridge.WorldMeshVertexRecord(
				2.0F + index * 0.25F,
				3.5F,
				4.25F,
				0.0F,
				0.0F,
				0.0F,
				0.0F,
				-1,
				-1,
				0,
				0xffffffff,
				0,
				0,
				0
			));
		}
		int[] metadata = primitiveMetadata(NativeSectionMeshBuilder.PRIMITIVE_KIND_UNKNOWN);
		metadata[2] = 12;
		metadata[3] = 2;
		metadata[4] = 3;
		metadata[5] = 4;
		metadata[6] = 0;
		metadata[9] = 9;

		RustGalTerrainRenderer.applyPrimitiveSemanticFallback(metadata, vertices, 4, false, false);

		for (VulkanicGalBridge.WorldMeshVertexRecord vertex : vertices) {
			assertEquals(12, vertex.shaderBlockId());
			assertEquals(0, vertex.shaderMaterialType());
		}
		assertEquals(0x09100020, vertices.get(0).midBlockPacked());
	}

	@Test
	public void primitiveMetadataOverridesPrivatePackedBlockIdentity() {
		int[] metadata = primitiveMetadata(NativeSectionMeshBuilder.PRIMITIVE_KIND_UNKNOWN);
		metadata[2] = 12;
		metadata[6] = 0;
		List<VulkanicGalBridge.WorldMeshVertexRecord> vertices = testVertices(1);
		for (int index = 0; index < vertices.size(); index++) {
			VulkanicGalBridge.WorldMeshVertexRecord vertex = vertices.get(index);
			vertices.set(index, new VulkanicGalBridge.WorldMeshVertexRecord(
				vertex.x(), vertex.y(), vertex.z(), vertex.u(), vertex.v(), vertex.atlasU(), vertex.atlasV(),
				32000, vertex.shaderMaterialType(), vertex.terrainMaterialBits(), vertex.colorArgb(), vertex.normalPacked(), vertex.light(), vertex.midBlockPacked()
			));
		}

		RustGalTerrainRenderer.applyPrimitiveSemanticFallback(metadata, vertices, 4, true, false);

		for (VulkanicGalBridge.WorldMeshVertexRecord vertex : vertices) {
			assertEquals(12, vertex.shaderBlockId());
			assertEquals(0, vertex.shaderMaterialType());
		}
	}

	@Test
	public void primitiveMetadataOverridesPrivatePackedRenderType() {
		int[] metadata = primitiveMetadata(NativeSectionMeshBuilder.PRIMITIVE_KIND_UNKNOWN);
		metadata[2] = 12;
		metadata[6] = 1;
		List<VulkanicGalBridge.WorldMeshVertexRecord> vertices = testVertices(1);
		for (int index = 0; index < vertices.size(); index++) {
			VulkanicGalBridge.WorldMeshVertexRecord vertex = vertices.get(index);
			vertices.set(index, new VulkanicGalBridge.WorldMeshVertexRecord(
				vertex.x(), vertex.y(), vertex.z(), vertex.u(), vertex.v(), vertex.atlasU(), vertex.atlasV(),
				32000, 2, vertex.terrainMaterialBits(), vertex.colorArgb(), vertex.normalPacked(), vertex.light(), vertex.midBlockPacked()
			));
		}

		RustGalTerrainRenderer.applyPrimitiveSemanticFallback(metadata, vertices, 4, true, false);

		for (VulkanicGalBridge.WorldMeshVertexRecord vertex : vertices) {
			assertEquals(12, vertex.shaderBlockId());
			assertEquals(1, vertex.shaderMaterialType());
		}
	}

	@Test
	public void translucentPrimitiveMetadataBuildsOrderedMixedMaterialRanges() {
		byte[] sorted = sortedQuads(0, 1, 2);
		int[] metadata = primitiveMetadata(
			NativeSectionMeshBuilder.PRIMITIVE_KIND_BUILTIN_WATER,
			NativeSectionMeshBuilder.PRIMITIVE_KIND_NON_FLUID_TRANSLUCENT,
			NativeSectionMeshBuilder.PRIMITIVE_KIND_BUILTIN_WATER
		);
		RustGalTerrainRenderer.installTestingFluidSpriteAssetsForUnitTests();

		RustGalTerrainRenderer.OrderedTranslucentMesh mesh =
			RustGalTerrainRenderer.buildOrderedTranslucentMesh(sorted, metadata, testVertices(3), 12);
		List<VulkanicGalBridge.WorldMeshSectionRecord> sections = mesh.sections();

		assertEquals(sorted.length, mesh.indexBytes().length);
		assertEquals(3, mesh.sourcePrimitiveCount());
		assertEquals(1, mesh.nonFluidPrimitiveCount());
		assertEquals(2, mesh.waterPrimitiveCount());
		assertEquals(0, mesh.unsupportedPrimitiveCount());
		assertEquals(3, mesh.retainedPrimitiveCount());
		assertEquals(0, mesh.omittedPrimitiveCount());
		assertEquals(18, mesh.sourceIndexCount());
		assertEquals(18, mesh.retainedIndexCount());
		assertEquals(3, sections.size(), "water/glass/water sorted order must not be grouped by material");
		assertEquals(2, mesh.materialSwitchCount());
		assertEquals(RustGalWorldPrimitiveRenderer.MATERIAL_ID_WATER_TRANSLUCENT, sections.get(0).materialId());
		assertEquals(RustGalWorldPrimitiveRenderer.MATERIAL_TEXTURE_WATER_STILL, sections.get(0).textureId());
		assertEquals(0, sections.get(0).indexOffset());
		assertEquals(6, sections.get(0).indexCount());
		assertEquals(RustGalWorldPrimitiveRenderer.MATERIAL_ID_TRANSLUCENT_TEXTURED, sections.get(1).materialId());
		assertEquals(RustGalWorldPrimitiveRenderer.MATERIAL_TEXTURE_TERRAIN_BLOCK_ATLAS, sections.get(1).textureId());
		assertEquals(24, sections.get(1).indexOffset());
		assertEquals(6, sections.get(1).indexCount());
		assertEquals(RustGalWorldPrimitiveRenderer.MATERIAL_ID_WATER_TRANSLUCENT, sections.get(2).materialId());
		assertEquals(RustGalWorldPrimitiveRenderer.MATERIAL_TEXTURE_WATER_FLOW, sections.get(2).textureId());
		assertEquals(48, sections.get(2).indexOffset());
		assertEquals(6, sections.get(2).indexCount());
	}

	@Test
	public void normalVulkanWaterBindingDoesNotDependOnPrivateAnimationAdmission() throws Exception {
		String property = "mattmc.dev.rustGalAtlasAnimation";
		String previous = System.getProperty(property);
		try {
			for (String value : new String[] { "false", "true" }) {
				System.setProperty(property, value);
				for (var route : WorldRenderRoutePolicy.Route.values()) {
					assertEquals(route.usesRustWholeFrameVulkan()
						? RustGalTerrainRenderer.WaterTextureBinding.BLOCK_ATLAS
						: RustGalTerrainRenderer.WaterTextureBinding.SEPARATE_SHEETS,
						RustGalTerrainRenderer.waterTextureBinding(route));
				}
			}
			System.clearProperty(property);
			assertEquals(RustGalTerrainRenderer.WaterTextureBinding.BLOCK_ATLAS,
				RustGalTerrainRenderer.waterTextureBinding(WorldRenderRoutePolicy.Route.RUST_VULKAN_WHOLE_FRAME));
			String source = Files.readString(Path.of("src/main/java/net/vulkanic/world/RustGalTerrainRenderer.java"));
			assertTrue(source.contains("waterTextureBinding(WorldRenderRoutePolicy.currentStaticTerrainRoute())"));
			assertFalse(source.contains("AtlasAnimationResource.privateTickDeliveryEnabled()"));
		} finally {
			if (previous == null) System.clearProperty(property); else System.setProperty(property, previous);
		}
	}

	@Test
	public void atlasWaterPreservesSortedGeometryAndMaterialWithoutSheetUvRemapping() {
		RustGalTerrainRenderer.installTestingFluidSpriteAssetsForUnitTests();
		List<VulkanicGalBridge.WorldMeshVertexRecord> vertices = testVertices(3);
		List<VulkanicGalBridge.WorldMeshVertexRecord> originals = List.copyOf(vertices);
		byte[] sorted = sortedQuads(2, 1, 0);
		var mesh = RustGalTerrainRenderer.buildOrderedTranslucentMesh(sorted, primitiveMetadata(
			NativeSectionMeshBuilder.PRIMITIVE_KIND_BUILTIN_WATER,
			NativeSectionMeshBuilder.PRIMITIVE_KIND_NON_FLUID_TRANSLUCENT,
			NativeSectionMeshBuilder.PRIMITIVE_KIND_BUILTIN_WATER), vertices, 12,
			RustGalTerrainRenderer.waterTextureBinding(WorldRenderRoutePolicy.Route.RUST_VULKAN_WHOLE_FRAME));
		assertArrayEquals(sorted, mesh.indexBytes());
		assertEquals(3, mesh.sections().size());
		assertEquals(1, mesh.waterStillPrimitiveCount());
		assertEquals(1, mesh.waterFlowPrimitiveCount());
		assertEquals(0, mesh.omittedPrimitiveCount());
		for (int index = 0; index < vertices.size(); index++) {
			var before = originals.get(index);
			var after = vertices.get(index);
			int type = index < 4 ? 1 : index >= 8 ? 2 : before.shaderMaterialType();
			assertEquals(new VulkanicGalBridge.WorldMeshVertexRecord(
				before.x(), before.y(), before.z(), before.u(), before.v(), before.atlasU(), before.atlasV(),
				before.shaderBlockId(), type, before.terrainMaterialBits(), before.colorArgb(),
				before.normalPacked(), before.light(), before.midBlockPacked()), after);
		}
		for (int index = 0; index < 3; index++) {
			var section = mesh.sections().get(index);
			assertEquals(RustGalWorldPrimitiveRenderer.MATERIAL_TEXTURE_TERRAIN_BLOCK_ATLAS, section.textureId());
			assertEquals(index == 1 ? RustGalWorldPrimitiveRenderer.MATERIAL_ID_TRANSLUCENT_TEXTURED
				: RustGalWorldPrimitiveRenderer.MATERIAL_ID_WATER_TRANSLUCENT, section.materialId());
			assertEquals(index * 24, section.indexOffset());
			assertEquals(6, section.indexCount());
			assertEquals(RustGalWorldPrimitiveRenderer.CULL_BACK, section.cullPolicy());
			assertEquals(RustGalWorldPrimitiveRenderer.WORLD_WINDING_CCW, section.winding());
			assertEquals(RustGalWorldPrimitiveRenderer.MATERIAL_MODE_TRANSLUCENT, section.materialMode());
		}
	}

	@Test
	public void adjacentAtlasWaterSpritesShareOneRangeWithoutLosingSpriteIdentity() {
		RustGalTerrainRenderer.installTestingFluidSpriteAssetsForUnitTests();
		List<VulkanicGalBridge.WorldMeshVertexRecord> vertices = testVertices(3);
		// The middle quad occupies the overlay sprite; the others are still/flow.
		vertices.set(4, vertex(0.55F, 0.05F));
		vertices.set(5, vertex(0.60F, 0.05F));
		vertices.set(6, vertex(0.60F, 0.10F));
		vertices.set(7, vertex(0.55F, 0.10F));
		byte[] sorted = sortedQuads(2, 0, 1);
		var mesh = RustGalTerrainRenderer.buildOrderedTranslucentMesh(sorted, primitiveMetadata(
			NativeSectionMeshBuilder.PRIMITIVE_KIND_BUILTIN_WATER,
			NativeSectionMeshBuilder.PRIMITIVE_KIND_BUILTIN_WATER,
			NativeSectionMeshBuilder.PRIMITIVE_KIND_BUILTIN_WATER), vertices, 12,
			RustGalTerrainRenderer.WaterTextureBinding.BLOCK_ATLAS);
		assertArrayEquals(sorted, mesh.indexBytes());
		assertEquals(1, mesh.sections().size());
		assertEquals(18, mesh.sections().getFirst().indexCount());
		assertEquals(1, mesh.waterStillPrimitiveCount());
		assertEquals(1, mesh.waterFlowPrimitiveCount());
		assertEquals(1, mesh.waterOverlayPrimitiveCount());
		assertEquals(3, vertices.get(4).shaderMaterialType());
		assertEquals(0.55F, vertices.get(4).u());
		assertEquals(0, mesh.waterTextureSwitchCount());
	}

	@Test
	public void translucentPrimitiveMetadataRetainsGenericFluidWithAtlasSemanticsWithoutReordering() {
		byte[] sorted = sortedQuads(0, 1, 2);
		int[] metadata = primitiveMetadata(
			NativeSectionMeshBuilder.PRIMITIVE_KIND_NON_FLUID_TRANSLUCENT,
			NativeSectionMeshBuilder.PRIMITIVE_KIND_GENERIC_FLUID,
			NativeSectionMeshBuilder.PRIMITIVE_KIND_BUILTIN_WATER
		);
		RustGalTerrainRenderer.installTestingFluidSpriteAssetsForUnitTests();

		RustGalTerrainRenderer.OrderedTranslucentMesh mesh =
			RustGalTerrainRenderer.buildOrderedTranslucentMesh(sorted, metadata, testVertices(3), 12);
		List<VulkanicGalBridge.WorldMeshSectionRecord> sections = mesh.sections();

		assertEquals(72, mesh.indexBytes().length, "a generic fluid must remain in the sorted payload");
		assertEquals(3, mesh.sourcePrimitiveCount());
		assertEquals(2, mesh.nonFluidPrimitiveCount());
		assertEquals(1, mesh.waterPrimitiveCount());
		assertEquals(0, mesh.unsupportedPrimitiveCount());
		assertEquals(3, mesh.retainedPrimitiveCount());
		assertEquals(0, mesh.omittedPrimitiveCount());
		assertEquals(18, mesh.sourceIndexCount());
		assertEquals(18, mesh.retainedIndexCount());
		assertEquals(0, mesh.omittedIndexCount());
		assertEquals(2, sections.size(), "adjacent generic-fluid and ordinary translucent atlas ranges should coalesce");
		assertEquals(RustGalWorldPrimitiveRenderer.MATERIAL_ID_TRANSLUCENT_TEXTURED, sections.get(0).materialId());
		assertEquals(0, sections.get(0).indexOffset());
		assertEquals(12, sections.get(0).indexCount());
		assertEquals(RustGalWorldPrimitiveRenderer.MATERIAL_ID_WATER_TRANSLUCENT, sections.get(1).materialId());
		assertEquals(RustGalWorldPrimitiveRenderer.MATERIAL_TEXTURE_WATER_FLOW, sections.get(1).textureId());
		assertEquals(48, sections.get(1).indexOffset());
	}

	@Test
	public void translucentPrimitiveMetadataRejectsMalformedSortReferences() {
		int[] metadata = primitiveMetadata(
			NativeSectionMeshBuilder.PRIMITIVE_KIND_NON_FLUID_TRANSLUCENT,
			NativeSectionMeshBuilder.PRIMITIVE_KIND_BUILTIN_WATER
		);
		RustGalTerrainRenderer.installTestingFluidSpriteAssetsForUnitTests();
		List<VulkanicGalBridge.WorldMeshVertexRecord> vertices = testVertices(2);

		assertThrows(IllegalArgumentException.class,
			() -> RustGalTerrainRenderer.buildOrderedTranslucentMesh(sortedQuads(0, 0), metadata, new ArrayList<>(vertices), 8));
		assertThrows(IllegalArgumentException.class,
			() -> RustGalTerrainRenderer.buildOrderedTranslucentMesh(sortedQuads(0), metadata, new ArrayList<>(vertices), 8));

		byte[] interleaved = sortedQuads(0);
		interleaved[4] = 7;
		assertThrows(IllegalArgumentException.class,
			() -> RustGalTerrainRenderer.buildOrderedTranslucentMesh(interleaved, metadata, new ArrayList<>(vertices), 8));
	}

	@Test
	public void facingLocalStaticSortIndicesNormalizeIntoTheCopiedGlobalVertexStream() {
		byte[] facingLocal = sortedQuads(0, 0);

		byte[] normalized = RustGalTerrainRenderer.normalizeTranslucentSortedIndexBytes(
			facingLocal,
			8,
			new int[] { 1, 1 }
		);

		assertArrayEquals(sortedQuads(0, 1), normalized);
		assertArrayEquals(
			sortedQuads(1, 0),
			RustGalTerrainRenderer.normalizeTranslucentSortedIndexBytes(sortedQuads(1, 0), 8, new int[] { 1, 1 }),
			"already-global dynamic or topological sorter payloads must pass through unchanged"
		);
		assertThrows(IllegalArgumentException.class,
			() -> RustGalTerrainRenderer.normalizeTranslucentSortedIndexBytes(sortedQuads(0, 0), 8, new int[] { 2 }));
	}

	@Test
	public void translucentPrimitiveMetadataRepresentsAllUnsupportedPayloadAsAnEmptyFilteredRange() {
		int[] metadata = primitiveMetadata(NativeSectionMeshBuilder.PRIMITIVE_KIND_UNSUPPORTED_FLUID);

		RustGalTerrainRenderer.OrderedTranslucentMesh mesh =
			RustGalTerrainRenderer.buildOrderedTranslucentMesh(sortedQuads(0), metadata, testVertices(1), 4);

		assertEquals(1, mesh.sourcePrimitiveCount());
		assertEquals(1, mesh.unsupportedPrimitiveCount());
		assertEquals(0, mesh.retainedPrimitiveCount());
		assertEquals(1, mesh.omittedPrimitiveCount());
		assertEquals(0, mesh.retainedIndexCount());
		assertEquals(6, mesh.omittedIndexCount());
		assertTrue(mesh.sections().isEmpty());
	}

	@Test
	public void translucentPrimitiveMetadataTreatsUnknownFlatQuadAsNonFluidTranslucent() {
		RustGalTerrainRenderer.OrderedTranslucentMesh mesh =
			RustGalTerrainRenderer.buildOrderedTranslucentMesh(
				sortedQuads(0),
				primitiveMetadata(NativeSectionMeshBuilder.PRIMITIVE_KIND_UNKNOWN),
				testVertices(1), 4);

		assertEquals(1, mesh.nonFluidPrimitiveCount());
		assertEquals(1, mesh.retainedPrimitiveCount());
		assertEquals(0, mesh.omittedPrimitiveCount());
		assertEquals(RustGalWorldPrimitiveRenderer.MATERIAL_ID_TRANSLUCENT_TEXTURED,
			mesh.sections().get(0).materialId());
	}

	@Test
	void terrainSectionIndexPublishesOnlyAfterRustMeshRegistryAdmission() throws Exception {
		String source = Files.readString(Path.of(
			"src/main/java/net/vulkanic/world/RustGalTerrainRenderer.java"));
		int register = source.indexOf("RustGalWorldPrimitiveRenderer.registerStaticTerrainMeshAsset");
		int publish = source.indexOf("SECTION_ASSETS.put(new LayerKey", register);
		assertTrue(register >= 0 && publish > register,
			"terrain section index must publish only after Rust mesh admission");
	}

	@Test
	void atlasGenerationCommitsOnlyAfterRustMeshAdmission() throws Exception {
		String source = Files.readString(Path.of(
			"src/main/java/net/vulkanic/world/RustGalTerrainRenderer.java"));
		int register = source.indexOf("RustGalWorldPrimitiveRenderer.registerStaticTerrainMeshAsset");
		int confirm = source.indexOf("confirmAtlasPayloadRegistered(atlasGenerationForRegistration)", register);
		assertTrue(register >= 0 && confirm > register,
			"atlas generation must commit only after Rust mesh registration succeeds");
	}

	@Test
	void unchangedTerrainTexturePayloadsDoNotRemainDirtyAfterEveryMeshRegistration() throws Exception {
		String source = Files.readString(Path.of(
			"src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		int method = source.indexOf("public static void registerStaticTerrainMeshAsset(");
		int textureLoop = source.indexOf("for (VulkanicGalBridge.WorldMeshTextureAssetRecord texture : textures)", method);
		int compare = source.indexOf("changed |= registerChangedTexture(WORLD_MESH_TEXTURES, DIRTY_WORLD_MESH_TEXTURES, texture)", textureLoop);
		int end = source.indexOf("STATIC_TERRAIN_MESH_RESIDENCY.put", textureLoop);
		assertTrue(method >= 0 && textureLoop > method && compare > textureLoop && compare < end,
			"unchanged static-terrain texture payloads must not be re-dirtied every frame");
	}

	@Test
	void unchangedTerrainMeshPayloadsDoNotAdvanceUploadGeneration() throws Exception {
		String source = Files.readString(Path.of(
			"src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		int method = source.indexOf("public static void registerStaticTerrainMeshAsset(");
		int residency = source.indexOf("STATIC_TERRAIN_MESH_RESIDENCY.put", method);
		int helper = source.indexOf("sameStaticTerrainPayload(previous, asset)", residency);
		int definition = source.indexOf("private static boolean sameStaticTerrainPayload(");
		assertTrue(method >= 0 && residency > method && helper > residency && definition > helper,
			"payload-equivalent terrain rebuilds must retain the existing uploaded generation");
	}

	@Test
	void staleInvalidatedTerrainSectionsLeaveTheReadinessDomain() throws Exception {
		String source = Files.readString(Path.of(
			"src/main/java/net/vulkanic/world/RustGalWholeFrameTerrainSource.java"));
		int method = source.indexOf("private void admitInvalidatedSections");
		int visibility = source.indexOf("!this.isInsideCurrentWindow(section) || !this.isVisible(section, frustum)", method);
		int remove = source.indexOf("iterator.remove()", visibility);
		assertTrue(method >= 0 && visibility > method && remove > visibility,
			"stale terrain invalidations must not keep Rust whole-frame readiness blocked forever");
	}

	private static int[] primitiveMetadata(int... kinds) {
		int stride = NativeSectionMeshBuilder.PRIMITIVE_METADATA_RECORD_INTS;
		int[] metadata = new int[kinds.length * stride];
		for (int index = 0; index < kinds.length; index++) {
			metadata[index * stride] = kinds[index];
		}
		return metadata;
	}

	private static byte[] sortedQuads(int... primitiveIds) {
		byte[] bytes = new byte[primitiveIds.length * 24];
		int cursor = 0;
		for (int primitiveId : primitiveIds) {
			int base = primitiveId * 4;
			for (int value : new int[] { base, base + 1, base + 2, base + 2, base + 3, base }) {
				bytes[cursor++] = (byte)(value & 0xff);
				bytes[cursor++] = (byte)((value >>> 8) & 0xff);
				bytes[cursor++] = (byte)((value >>> 16) & 0xff);
				bytes[cursor++] = (byte)((value >>> 24) & 0xff);
			}
		}
		return bytes;
	}

	private static List<VulkanicGalBridge.WorldMeshVertexRecord> testVertices(int primitiveCount) {
		List<VulkanicGalBridge.WorldMeshVertexRecord> vertices = new ArrayList<>(primitiveCount * 4);
		for (int primitive = 0; primitive < primitiveCount; primitive++) {
			float baseU = switch (primitive) {
				case 0 -> 0.05F;
				case 2 -> 0.30F;
				default -> 0.80F;
			};
			float baseV = 0.05F;
			vertices.add(vertex(baseU, baseV));
			vertices.add(vertex(baseU + 0.05F, baseV));
			vertices.add(vertex(baseU + 0.05F, baseV + 0.05F));
			vertices.add(vertex(baseU, baseV + 0.05F));
		}
		return vertices;
	}

	private static VulkanicGalBridge.WorldMeshVertexRecord vertex(float u, float v) {
		return new VulkanicGalBridge.WorldMeshVertexRecord(
			0.0F,
			0.0F,
			0.0F,
			u,
			v,
			u,
			v,
			0,
			0,
			0,
			0xffffffff,
			0,
			0,
			0
		);
	}
}
