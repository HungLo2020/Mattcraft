package net.vulkanic.gui;

import static org.junit.jupiter.api.Assertions.assertFalse;

import java.util.List;
import net.vulkanic.bridge.VulkanicGalBridge;
import org.junit.jupiter.api.Test;

import static org.junit.jupiter.api.Assertions.assertArrayEquals;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

class GuiItemMeshSemanticCollectorTest {
	@Test
	void nativeBakedFaceAndFoilAreCopiedWithoutReplacingTheSourceNormal() {
		float[] position={1,2,3};
		var vertex=new VulkanicGalBridge.GuiMeshVertexRecord(position,new float[]{0,0},new float[]{0,0},
			-1,0x005a005a,5,1);
		position[0]=99;
		assertEquals(1,vertex.position()[0]);
		assertEquals(0x005a005a,vertex.normalPacked());
		assertEquals(5,vertex.sourceFace());
		assertEquals(1,vertex.sourceFoilType());
		var layer=new GuiItemMeshSemanticCollector.GuiItemMeshLayer(
			GuiItemMeshSemanticCollector.MaterialMode.CUTOUT,true,identityMatrix(),List.of(),null,1);
		assertEquals(1,layer.sourceFoilType());
		org.junit.jupiter.api.Assertions.assertNull(layer.itemFoil());
		for (int face : new int[]{-1,0,7}) {
			assertThrows(IllegalArgumentException.class, () -> new VulkanicGalBridge.GuiMeshVertexRecord(
				new float[3],new float[2],new float[2],-1,1,face,1));
		}
		assertThrows(IllegalArgumentException.class, () -> new VulkanicGalBridge.GuiMeshVertexRecord(
			new float[3],new float[2],new float[2],-1,1,5,2));
	}

	@Test
	void nativeBlockMeshCarriesBoundsWithoutJavaRasterSetup() {
		double[] bounds = {-0.5,-0.5,-0.5,0.5,0.5,0.5};
		var raster = new VulkanicGalBridge.GuiBlockItemRasterRecord(3,bounds);
		var mesh = new GuiItemMeshSemanticCollector.GuiItemMesh("minecraft:stone",0,0,0,0,16,16,
			new float[]{1,0,0,1,0,0},List.of(),List.of(),raster);
		bounds[0] = -99;
		assertEquals(-0.5,mesh.blockItemRaster().modelBounds()[0]);
		assertEquals(3,mesh.blockItemRaster().guiScale());
		var fields=java.util.Arrays.stream(GuiItemMeshSemanticCollector.GuiItemMesh.class.getRecordComponents())
			.map(java.lang.reflect.RecordComponent::getName).toList();
		for (String forbidden : List.of("renderWidth","renderHeight","guardPixels","offscreenModelTransform"))
			assertFalse(fields.contains(forbidden),"semantic block mesh cannot express Java raster setup");
		assertThrows(IllegalArgumentException.class, () -> new GuiItemMeshSemanticCollector.GuiItemMesh(
			"minecraft:stone",0,0,0,0,16,16,new float[6],List.of(),List.of(),null));
	}

	@Test
	void specialFoilRejectsBeforeClientResourceAccessOrPartialDrawPublication() throws Exception {
		var layer = new net.minecraft.client.renderer.item.ItemStackRenderState.SemanticLayer(
			List.of(), new int[0], null,
			net.minecraft.client.renderer.item.ItemStackRenderState.FoilType.SPECIAL,
			true, false, true, identityMatrix());
		var output = new java.util.ArrayList<GuiItemMeshSemanticCollector.GuiItemMeshLayer>();
		var sources = new java.util.ArrayList<GuiItemTextureSource>();
		var append = GuiItemMeshSemanticCollector.class.getDeclaredMethod("appendLayer",
			net.minecraft.client.renderer.item.ItemStackRenderState.SemanticLayer.class,
			List.class, List.class);
		append.setAccessible(true);
		assertEquals("special-foil-native-contract-unavailable",
			append.invoke(null, layer, output, sources));
		assertTrue(output.isEmpty());
		assertTrue(sources.isEmpty());
	}

	@Test
	void standardFoilCopiesOriginalUvsWithoutAClientOrTextureTransform() throws Exception {
		float[] positions = {0,0,0, 1,0,0, 1,1,0, 0,1,0};
		float[] atlas = {0.2F,0.3F, 0.4F,0.3F, 0.4F,0.8F, 0.2F,0.8F};
		float[] local = {0,0, 1,0, 1,1, 0,1};
		var source = new GuiItemMeshSemanticCollector.GuiItemMeshQuad(7,"test",positions,atlas,local,
			new int[]{1,2,3,4},new int[]{5,6,7,8},2,true);
		var copy = GuiItemMeshSemanticCollector.class.getDeclaredMethod("glintQuad",
			GuiItemMeshSemanticCollector.GuiItemMeshQuad.class,long.class);
		copy.setAccessible(true);
		var foil = (GuiItemMeshSemanticCollector.GuiItemMeshQuad)copy.invoke(null,source,99L);
		assertEquals(99L,foil.assetId());
		assertArrayEquals(positions,foil.positions());
		assertArrayEquals(atlas,foil.atlasUvs());
		assertArrayEquals(local,foil.localUvs());
		assertArrayEquals(new int[]{-1,-1,-1,-1},foil.colorsArgb());
		atlas[0]=0.99F;
		assertEquals(0.2F,foil.atlasUvs()[0]);
	}

	@Test
	void semanticMaterialModesIncludeExplicitTranslucentItems() {
		assertEquals(
			List.of(
				GuiItemMeshSemanticCollector.MaterialMode.OPAQUE,
				GuiItemMeshSemanticCollector.MaterialMode.CUTOUT,
				GuiItemMeshSemanticCollector.MaterialMode.TRANSLUCENT,
				GuiItemMeshSemanticCollector.MaterialMode.GLINT
			),
			List.of(GuiItemMeshSemanticCollector.MaterialMode.values())
		);
	}

	@Test
	void bridgeBatchAdmitsTheTranslucentMaterialMode() {
		VulkanicGalBridge.GuiMeshVertexRecord vertex = new VulkanicGalBridge.GuiMeshVertexRecord(
			new float[] {0.0F, 0.0F, 0.0F}, new float[] {0.0F, 0.0F}, new float[] {0.0F, 0.0F},
			0xffffffff, 0
		);
		new VulkanicGalBridge.GuiMeshBatchRecord(
			1, 0, 3, 1, 7L, 0L, 0.0F, identityMatrix(),
			new float[] {1.0F, 0.0F, 0.0F, 1.0F, 0.0F, 0.0F},
			0, 0, 16, 16, 32, 32, 18, 18, 1,
			List.of(vertex, vertex, vertex), List.of(0, 1, 2)
		);
	}

	@Test
	void foilCollectionOwnsAnExplicitGlintLayerAndStagesItsCopiedAsset() throws Exception {
		String source = java.nio.file.Files.readString(java.nio.file.Path.of(
			"src/main/java/net/vulkanic/gui/GuiItemMeshSemanticCollector.java"
		));
		assertTrue(source.contains("MaterialMode.GLINT"));
		assertTrue(source.contains("List<GuiItemTextureSource> sources"),
			"the collector must return typed immutable resources for post-admission staging");
		assertTrue(source.contains("new GuiItemTextureSource.Atlas("));
		assertTrue(source.contains("new GuiItemTextureSource.Raw(asset)"));
		assertFalse(source.contains("resolveAnimatedSprite"), "animation frame selection belongs to the native atlas");
		assertTrue(source.contains("RustGalGuiItemRenderer.itemLocalUv("),
			"owned atlas UVs must be copied exactly, not clamped into a different mapping");
		assertTrue(source.contains("resolveAssetId(quad.assetId())"),
			"mesh staging must preserve the exact copied raw asset identity, including foil");
		assertTrue(source.contains("ENCHANTED_GLINT_ITEM"));
		assertFalse(source.contains("specialFoilQuad"));
		assertTrue(source.contains("special-foil-native-contract-unavailable"));
		assertFalse(source.contains("SPECIAL_FOIL_TEXTURE_SCALE"));
	}

	@Test
	void dynamicItemFallbackPreparesWithoutPublishingBeforeAdmission() throws Exception {
		String collector = java.nio.file.Files.readString(java.nio.file.Path.of(
			"src/main/java/net/vulkanic/gui/GuiItemMeshSemanticCollector.java"));
		int fallback = collector.indexOf("instanceof net.minecraft.client.renderer.texture.DynamicTexture dynamic");
		assertTrue(fallback >= 0);
		assertTrue(collector.indexOf("registerDynamicTextureUnstaged(spriteIdentity, dynamic)", fallback) > fallback,
			"item collection must bind dynamic sources without publishing frame pixels");
		assertTrue(collector.indexOf("prepareDynamicTexture(dynamic)", fallback) > fallback,
			"item collection must privately prepare dynamic pixels before mesh admission");
		assertTrue(collector.indexOf("registerDynamicTexture(spriteIdentity, dynamic)", fallback) < 0,
			"item collection must not use the publishing lifecycle API");
	}

	@Test
	void standard3dVertexColorFollowsTheActiveItemEncoderPolicy() {
		int bakedVertexColor = 0xff4080c0;
		int tint = 0xff80a040;

		assertEquals(
			tint,
			GuiItemMeshSemanticCollector.standard3dVertexColor(bakedVertexColor, tint, false),
			"the current Fabric item fast path deliberately ignores baked vertex color"
		);
		assertEquals(
			0xff605010,
			GuiItemMeshSemanticCollector.standard3dVertexColor(bakedVertexColor, tint, true),
			"platforms that enable Java's baked-vertex multiplication retain it exactly once"
		);
	}

	@Test
	void copiedMeshSemanticsDoNotRetainMutableCallerArrays() {
		float[] transform = identityMatrix();
		float[] positions = new float[] {
			0.0F, 0.0F, 0.0F, 1.0F, 0.0F, 0.0F,
			1.0F, 1.0F, 0.0F, 0.0F, 1.0F, 0.0F
		};
		float[] uvs = new float[] {0.0F, 0.0F, 1.0F, 0.0F, 1.0F, 1.0F, 0.0F, 1.0F};
		float[] localUvs = new float[] {0.25F, 0.25F, 0.75F, 0.25F, 0.75F, 0.75F, 0.25F, 0.75F};
		GuiItemMeshSemanticCollector.GuiItemMeshQuad quad = new GuiItemMeshSemanticCollector.GuiItemMeshQuad(
			7L, "minecraft:block/stone", positions, uvs, localUvs, new int[] {1, 2, 3, 4}, new int[] {5, 6, 7, 8}, 2, true
		);
		GuiItemMeshSemanticCollector.GuiItemMeshLayer layer = new GuiItemMeshSemanticCollector.GuiItemMeshLayer(
			GuiItemMeshSemanticCollector.MaterialMode.OPAQUE, true, transform, List.of(quad)
		);

		transform[0] = 7.0F;
		positions[0] = 8.0F;
		float[] copiedTransform = layer.modelTransform();
		copiedTransform[5] = 9.0F;
		float[] copiedPositions = quad.positions();
		copiedPositions[3] = 10.0F;
		float[] copiedLocalUvs = quad.localUvs();
		copiedLocalUvs[0] = 11.0F;

		assertArrayEquals(identityMatrix(), layer.modelTransform());
		assertArrayEquals(new float[] {
			0.0F, 0.0F, 0.0F, 1.0F, 0.0F, 0.0F,
			1.0F, 1.0F, 0.0F, 0.0F, 1.0F, 0.0F
		}, quad.positions());
		assertArrayEquals(localUvs, quad.localUvs());
	}

	@Test
	void copiedMeshSemanticsRejectMalformedGeometry() {
		assertThrows(IllegalArgumentException.class, () -> new GuiItemMeshSemanticCollector.GuiItemMeshQuad(
			7L, "minecraft:block/stone", new float[11], new float[8], new float[8], new int[4], new int[4], 2, true
		));
		float[] nonFinite = new float[8];
		nonFinite[3] = Float.POSITIVE_INFINITY;
		assertThrows(IllegalArgumentException.class, () -> new GuiItemMeshSemanticCollector.GuiItemMeshQuad(
			7L, "minecraft:block/stone", new float[12], nonFinite, new float[8], new int[4], new int[4], 2, true
		));
	}

	@Test
	void copiedGuiSemanticsDoNotRetainMutableTransforms() {
		float[] guiPose = new float[] {1.0F, 0.0F, 0.0F, 1.0F, 4.0F, 8.0F};
		var raster=new VulkanicGalBridge.GuiBlockItemRasterRecord(2,new double[]{-.5,-.5,-.5,.5,.5,.5});
		GuiItemMeshSemanticCollector.GuiItemMesh mesh = new GuiItemMeshSemanticCollector.GuiItemMesh(
			"minecraft:stone", 4, 8, 4, 8, 20, 24, guiPose, List.of(), List.of(),raster
		);
		guiPose[0] = 7.0F;
		mesh.guiPose()[0] = 11.0F;

		assertArrayEquals(new float[] {1.0F, 0.0F, 0.0F, 1.0F, 4.0F, 8.0F}, mesh.guiPose());
		assertThrows(IllegalArgumentException.class, () -> new GuiItemMeshSemanticCollector.GuiItemMesh(
			"minecraft:stone", 0, 0, 0, 0, 16, 16, new float[5], List.of(), List.of(),raster
		));
	}

	@Test
	void collectorHasNoJavaRasterOrNormalTransformRoute() throws Exception {
		String source=java.nio.file.Files.readString(java.nio.file.Path.of(
			"src/main/java/net/vulkanic/gui/GuiItemMeshSemanticCollector.java"));
		for (String forbidden : List.of("rustGalGuiBlockItemLayout","standard3dTarget(","transformGuiNormal(","new PoseStack(","invert().transpose()"))
			assertFalse(source.contains(forbidden),"removed Java rendering policy must not return: "+forbidden);
		assertTrue(source.contains("normals[index] = quad.getAccurateNormal(index)"));
	}

	private static float[] identityMatrix() {
		return new float[] {
			1.0F, 0.0F, 0.0F, 0.0F,
			0.0F, 1.0F, 0.0F, 0.0F,
			0.0F, 0.0F, 1.0F, 0.0F,
			0.0F, 0.0F, 0.0F, 1.0F
		};
	}
}
