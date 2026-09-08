package net.vulkanic.gui;

import java.util.List;
import net.blaze3d.vertex.PoseStack;
import net.vulkanic.bridge.VulkanicGalBridge;
import net.sodium.api.math.MatrixHelper;
import org.junit.jupiter.api.Test;
import org.joml.Matrix4f;

import static org.junit.jupiter.api.Assertions.assertArrayEquals;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

class GuiItemMeshSemanticCollectorTest {
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
		assertTrue(source.contains("List<RustGalGuiRawImageAssets.Asset> assets"),
			"the collector must return copied assets for post-admission staging");
		assertTrue(source.contains("resolveAssetId(quad.assetId())"),
			"mesh staging must preserve the exact copied asset identity, including animated frames");
		assertTrue(source.contains("ENCHANTED_GLINT_ITEM"));
		assertTrue(source.contains("specialFoilQuad"));
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
	void copiedStandard3dTargetDoesNotRetainMutableTransforms() {
		float[] guiPose = new float[] {1.0F, 0.0F, 0.0F, 1.0F, 4.0F, 8.0F};
		float[] offscreen = identityMatrix();
		GuiItemMeshSemanticCollector.GuiItemMesh mesh = new GuiItemMeshSemanticCollector.GuiItemMesh(
			"minecraft:stone", 4, 8, 4, 8, 20, 24, guiPose, 34, 34, 1, offscreen, List.of(), List.of()
		);
		guiPose[0] = 7.0F;
		offscreen[5] = 9.0F;
		float[] copied = mesh.offscreenModelTransform();
		copied[10] = 11.0F;

		assertArrayEquals(new float[] {1.0F, 0.0F, 0.0F, 1.0F, 4.0F, 8.0F}, mesh.guiPose());
		assertArrayEquals(identityMatrix(), mesh.offscreenModelTransform());
		assertThrows(IllegalArgumentException.class, () -> new GuiItemMeshSemanticCollector.GuiItemMesh(
			"minecraft:stone", 0, 0, 0, 0, 16, 16, new float[6], 2, 2, 1, identityMatrix(), List.of(), List.of()
		));
	}

	@Test
	void copiedNormalsMatchTheJavaItemPoseNormalMatrix() {
		Matrix4f transform = new Matrix4f().rotateX((float)(Math.PI / 2.0)).scale(1.0F, -1.0F, -1.0F);
		PoseStack poseStack = new PoseStack();
		poseStack.mulPose(transform);
		int rawNormal = 0x007f0000;

		assertEquals(
			MatrixHelper.transformNormal(poseStack.last().normal(), poseStack.last().trustedNormals, rawNormal),
			GuiItemMeshSemanticCollector.transformGuiNormal(transform, rawNormal),
			"the copied normal must match Java's item encoder before Rust receives it"
		);
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
