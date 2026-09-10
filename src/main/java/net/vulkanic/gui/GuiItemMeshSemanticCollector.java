package net.vulkanic.gui;

import java.util.ArrayList;
import java.util.List;
import net.minecraft.Util;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.render.state.GuiItemRenderState;
import net.minecraft.client.renderer.RenderType;
import net.minecraft.client.renderer.block.model.BakedQuad;
import net.minecraft.client.renderer.entity.ItemRenderer;
import net.minecraft.client.renderer.item.ItemStackRenderState;
import net.minecraft.client.renderer.texture.TextureAtlasSprite;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.util.ARGB;
import net.minecraft.util.Mth;
import net.minecraft.world.phys.AABB;
import net.sodium.client.render.immediate.model.BakedModelEncoder;
import net.sodium.client.model.quad.BakedQuadView;

/**
 * Copies ordinary vanilla GUI item-model semantics before any native packing.
 * It intentionally retains no renderer, model, atlas, or GPU objects. The
 * Rust owns raster layout, normal selection, lighting and GPU execution.
 */
public final class GuiItemMeshSemanticCollector {
	private GuiItemMeshSemanticCollector() {
	}

	/**
	 * Copies original standard-3D GUI item semantics. No offscreen extent,
	 * raster pose, transformed normal, or GPU state is computed here.
	 */
	public static CollectionResult collectStandard3d(GuiItemRenderState item, int guiScale) {
		if (item == null || item.itemStackRenderState() == null) {
			return CollectionResult.rejected("missing-item-state");
		}
		if (item.itemStackRenderState().displayContext() != net.minecraft.world.item.ItemDisplayContext.GUI) {
			return CollectionResult.rejected("display-context");
		}
		if (!item.itemStackRenderState().usesBlockLight()) {
			return CollectionResult.rejected("flat-lighting");
		}
		if (guiScale <= 0) {
			return CollectionResult.rejected("gui-scale");
		}
		AABB modelBounds = item.itemStackRenderState().getModelBoundingBox();
		if (modelBounds == null) return CollectionResult.rejected("model-bounds");
		var blockRaster = new net.vulkanic.bridge.VulkanicGalBridge.GuiBlockItemRasterRecord(
			guiScale, new double[] {modelBounds.minX, modelBounds.minY, modelBounds.minZ,
				modelBounds.maxX, modelBounds.maxY, modelBounds.maxZ},item.itemStackRenderState().isOversizedInGui());
		List<GuiItemMeshLayer> layers = new ArrayList<>();
		List<GuiItemTextureSource> sources = new ArrayList<>();
		String[] rejection = new String[1];
		item.itemStackRenderState().forEachSemanticLayer(layer -> {
			if (rejection[0] != null) {
				return;
			}
			rejection[0] = appendLayer(layer, layers, sources);
		});
		if (rejection[0] != null || layers.isEmpty()) {
			return CollectionResult.rejected(rejection[0] == null ? "empty-mesh" : rejection[0]);
		}
		for (GuiItemMeshLayer layer : layers) for (GuiItemMeshQuad quad : layer.quads()) {
			RustGalGuiRawImageAssets.Asset asset = RustGalGuiRawImageAssets.resolveAssetId(quad.assetId());
			if (asset != null && sources.stream().noneMatch(existing -> existing.assetId() == asset.assetId()))
				sources.add(new GuiItemTextureSource.Raw(asset));
		}
		// Native layout receives the original logical item origin/box; Rust
		// derives oversized placement before applying the copied GUI pose.
		int left = item.x();
		int top = item.y();
		int right = item.x() + 16;
		int bottom = item.y() + 16;
		return CollectionResult.accepted(new GuiItemMesh(
			item.name(), item.x(), item.y(), left, top, right, bottom,
			new float[] {item.pose().m00(), item.pose().m01(), item.pose().m10(), item.pose().m11(), item.pose().m20(), item.pose().m21()},
			layers, sources, blockRaster
		));
	}

	private static String appendLayer(ItemStackRenderState.SemanticLayer layer, List<GuiItemMeshLayer> output,
		List<GuiItemTextureSource> sources) {
		if (layer == null) return "missing-layer";
		if (layer.hasSpecialRenderer()) return "special-renderer";
		if (layer.foilType() == ItemStackRenderState.FoilType.SPECIAL) {
			return "special-foil-native-contract-unavailable";
		}
		if (layer.renderType() == null || layer.quads().isEmpty()) return "empty-or-missing-render-type";
		MaterialMode mode = materialMode(layer.renderType());
		if (mode == null) return "render-type";

		List<GuiItemMeshQuad> quads = new ArrayList<>(layer.quads().size());
		for (BakedQuad quad : layer.quads()) {
			GuiItemMeshQuad copied = copyQuad(quad, layer.tintLayers(), sources);
			if (copied == null) return "unsupported-quad";
			quads.add(copied);
		}
		float[] modelTransform = layer.modelTransform();
		int sourceFoilType = layer.foilType() == ItemStackRenderState.FoilType.STANDARD ? 1 : 0;
		output.add(new GuiItemMeshLayer(mode, layer.usesBlockLight(), modelTransform, quads, null, sourceFoilType));
		if (layer.foilType() == ItemStackRenderState.FoilType.STANDARD) {
			RustGalGuiRawImageAssets.Asset glint = RustGalGuiRawImageAssets.resolve(ItemRenderer.ENCHANTED_GLINT_ITEM);
			if (glint == null) return "glint-texture-unavailable";
			List<GuiItemMeshQuad> glintQuads = new ArrayList<>(quads.size());
			for (GuiItemMeshQuad quad : quads) {
				glintQuads.add(glintQuad(quad, glint.assetId()));
			}
			output.add(new GuiItemMeshLayer(MaterialMode.GLINT, false, modelTransform, glintQuads,
				new net.vulkanic.bridge.VulkanicGalBridge.StandardItemFoilRecord(Util.getMillis(),
					Minecraft.getInstance().options.glintSpeed().get(),
					Minecraft.getInstance().options.glintStrength().get().floatValue()), sourceFoilType));
		}
		return null;
	}

	/** Copies the original geometry/UVs; native material preparation owns foil math. */
	private static GuiItemMeshQuad glintQuad(GuiItemMeshQuad source, long glintAssetId) {
		int[] colors = new int[] {
			0xffffffff, 0xffffffff, 0xffffffff, 0xffffffff
		};
		return new GuiItemMeshQuad(glintAssetId, "minecraft:glint", source.positions(), source.atlasUvs(),
			source.localUvs(), colors, source.packedNormals(), source.lightFace(), false);
	}

	private static MaterialMode materialMode(RenderType renderType) {
		String name = renderType.toString();
		if (name.contains("translucent")) return MaterialMode.TRANSLUCENT;
		if (name.contains("cutout")) return MaterialMode.CUTOUT;
		return name.contains("item") || name.contains("solid") ? MaterialMode.OPAQUE : null;
	}

	private static GuiItemMeshQuad copyQuad(BakedQuad bakedQuad, int[] tintLayers,
		List<GuiItemTextureSource> sources) {
		if (!(bakedQuad instanceof BakedQuadView quad)) return null;
		TextureAtlasSprite sprite = quad.getSprite();
		ResourceLocation spriteIdentity = sprite == null ? null : sprite.contents().name();
		if (sprite == null || spriteIdentity == null) return null;
		long assetId;
		if (net.minecraft.client.renderer.texture.TextureAtlas.LOCATION_BLOCKS.equals(sprite.atlasLocation())) {
			var region = net.vulkanic.world.RustGalTerrainRenderer.requireGuiAtlasSpritePayload(sprite);
			assetId = RustGalGuiRawImageAssets.assetId("gui-atlas-region:"+sprite.atlasLocation()+":"+spriteIdentity);
			if (sources.stream().noneMatch(source -> source.assetId() == assetId)) {
				sources.add(new GuiItemTextureSource.Atlas(new GuiAtlasRegion(assetId,region.texture(),
					region.atlasWidth(),region.atlasHeight(),region.x(),region.y(),region.width(),region.height())));
			}
		} else {
			if (sprite.contents().isAnimated()) return null;
			RustGalGuiRawImageAssets.Asset asset = RustGalGuiRawImageAssets.resolve(spriteIdentity);
			if (asset == null) {
				var texture = Minecraft.getInstance().getTextureManager().getTexture(spriteIdentity);
				if (texture instanceof net.minecraft.client.renderer.texture.DynamicTexture dynamic) {
					RustGalGuiRawImageAssets.registerDynamicTextureUnstaged(spriteIdentity, dynamic);
					RustGalGuiRawImageAssets.prepareDynamicTexture(dynamic);
					asset = RustGalGuiRawImageAssets.resolve(spriteIdentity);
				}
			}
			if (asset == null) return null;
			assetId = asset.assetId();
		}
		float[] positions = new float[12];
		float[] atlasUvs = new float[8];
		float[] localUvs = new float[8];
		int[] colors = new int[4];
		int[] normals = new int[4];
		int tint = itemTint(bakedQuad, tintLayers);
		for (int index = 0; index < 4; index++) {
			float x = quad.getX(index);
			float y = quad.getY(index);
			float z = quad.getZ(index);
			float u = quad.getTexU(index);
			float v = quad.getTexV(index);
			if (!Float.isFinite(x) || !Float.isFinite(y) || !Float.isFinite(z) || !Float.isFinite(u) || !Float.isFinite(v)) {
				return null;
			}
			int position = index * 3;
			positions[position] = x;
			positions[position + 1] = y;
			positions[position + 2] = z;
			int uv = index * 2;
			atlasUvs[uv] = u;
			atlasUvs[uv + 1] = v;
			localUvs[uv] = RustGalGuiItemRenderer.itemLocalUv(u,sprite.getU0(),sprite.getU1());
			localUvs[uv + 1] = RustGalGuiItemRenderer.itemLocalUv(v,sprite.getV0(),sprite.getV1());
			// Keep this aligned with ItemRenderer's Sodium fast path. Fabric's
			// current item path does not multiply the baked per-vertex color, so
			// applying it here would add face shading that Java never renders.
			colors[index] = standard3dVertexColor(
				quad.getColor(index), tint, BakedModelEncoder.shouldMultiplyAlpha()
			);
			// Native layout receives original model-space normals, never a Java raster basis.
			normals[index] = quad.getAccurateNormal(index);
		}
		return new GuiItemMeshQuad(
			assetId, spriteIdentity.toString(), positions, atlasUvs, localUvs, colors, normals,
			quad.getLightFace().get3DDataValue(), quad.hasShade()
		);
	}

	private static int itemTint(BakedQuad quad, int[] tintLayers) {
		if (!quad.isTinted() || tintLayers == null || tintLayers.length == 0 || quad.tintIndex() < 0 || quad.tintIndex() >= tintLayers.length) return 0xffffffff;
		int tint = tintLayers[quad.tintIndex()];
		return tint == -1 ? 0xffffffff : tint;
	}

	static int standard3dVertexColor(int bakedColor, int tint, boolean multiplyBakedVertexColor) {
		if (!multiplyBakedVertexColor) {
			return tint;
		}
		return shadedColor(bakedColor, tint);
	}

	private static int shadedColor(int bakedColor, int tint) {
		int alpha = Math.round(((bakedColor >>> 24 & 0xff) / 255.0F) * (ARGB.alpha(tint) / 255.0F) * 255.0F);
		int red = Math.round(((bakedColor & 0xff) / 255.0F) * (ARGB.red(tint) / 255.0F) * 255.0F);
		int green = Math.round(((bakedColor >>> 8 & 0xff) / 255.0F) * (ARGB.green(tint) / 255.0F) * 255.0F);
		int blue = Math.round(((bakedColor >>> 16 & 0xff) / 255.0F) * (ARGB.blue(tint) / 255.0F) * 255.0F);
		return ARGB.color(Mth.clamp(alpha, 0, 255), Mth.clamp(red, 0, 255), Mth.clamp(green, 0, 255), Mth.clamp(blue, 0, 255));
	}

	public enum MaterialMode {
		OPAQUE,
		CUTOUT,
		TRANSLUCENT,
		GLINT
	}

	public record CollectionResult(GuiItemMesh mesh, String rejection) {
		private static CollectionResult accepted(GuiItemMesh mesh) {
			return new CollectionResult(mesh, null);
		}

		private static CollectionResult rejected(String rejection) {
			return new CollectionResult(null, rejection);
		}

		public boolean accepted() {
			return this.mesh != null;
		}
	}

	public record GuiItemMesh(
		String itemIdentity, int itemX, int itemY, int left, int top, int right, int bottom,
		float[] guiPose,
		List<GuiItemMeshLayer> layers, List<GuiItemTextureSource> sources,
		net.vulkanic.bridge.VulkanicGalBridge.GuiBlockItemRasterRecord blockItemRaster
	) {
		public GuiItemMesh {
			guiPose = checkedCopy(guiPose, 6, "GUI item pose");
			if (blockItemRaster == null) throw new IllegalArgumentException("GUI block mesh requires native model bounds and scale");
			layers = List.copyOf(layers);
			sources = List.copyOf(sources);
		}

		@Override
		public float[] guiPose() {
			return this.guiPose.clone();
		}

	}

	public record GuiItemMeshLayer(MaterialMode materialMode, boolean blockLight, float[] modelTransform, List<GuiItemMeshQuad> quads,
		net.vulkanic.bridge.VulkanicGalBridge.StandardItemFoilRecord itemFoil, int sourceFoilType) {
		public GuiItemMeshLayer(MaterialMode materialMode, boolean blockLight, float[] modelTransform, List<GuiItemMeshQuad> quads,
			net.vulkanic.bridge.VulkanicGalBridge.StandardItemFoilRecord itemFoil) {
			this(materialMode, blockLight, modelTransform, quads, itemFoil, 0);
		}
		public GuiItemMeshLayer(MaterialMode materialMode, boolean blockLight, float[] modelTransform, List<GuiItemMeshQuad> quads) {
			this(materialMode, blockLight, modelTransform, quads, null);
		}
		public GuiItemMeshLayer {
			if (sourceFoilType < 0 || sourceFoilType > 1) throw new IllegalArgumentException("unsupported source item foil type");
			if (itemFoil != null && materialMode != MaterialMode.GLINT) throw new IllegalArgumentException("foil requires glint layer");
			modelTransform = checkedCopy(modelTransform, 16, "GUI item model transform");
			quads = List.copyOf(quads);
		}

		@Override
		public float[] modelTransform() {
			return this.modelTransform.clone();
		}
	}

	public record GuiItemMeshQuad(
		long assetId, String materialIdentity, float[] positions, float[] atlasUvs, float[] localUvs, int[] colorsArgb, int[] packedNormals,
		int lightFace, boolean shade
	) {
		public GuiItemMeshQuad {
			if (assetId == 0L) {
				throw new IllegalArgumentException("GUI item mesh quad requires a non-zero semantic image asset id");
			}
			positions = checkedCopy(positions, 12, "GUI item positions");
			atlasUvs = checkedCopy(atlasUvs, 8, "GUI item atlas UVs");
			localUvs = checkedCopy(localUvs, 8, "GUI item local UVs");
			colorsArgb = colorsArgb.clone();
			packedNormals = packedNormals.clone();
			if (colorsArgb.length != 4 || packedNormals.length != 4) {
				throw new IllegalArgumentException("GUI item mesh quad requires four colors and item-lighting-space normals");
			}
		}

		@Override
		public float[] positions() {
			return this.positions.clone();
		}

		@Override
		public float[] atlasUvs() {
			return this.atlasUvs.clone();
		}

		@Override
		public float[] localUvs() {
			return this.localUvs.clone();
		}

		@Override
		public int[] colorsArgb() {
			return this.colorsArgb.clone();
		}

		@Override
		public int[] packedNormals() {
			return this.packedNormals.clone();
		}
	}

	private static float[] checkedCopy(float[] values, int expectedLength, String name) {
		if (values.length != expectedLength) {
			throw new IllegalArgumentException(name + " must contain " + expectedLength + " floats");
		}
		for (float value : values) {
			if (!Float.isFinite(value)) {
				throw new IllegalArgumentException(name + " must be finite");
			}
		}
		return values.clone();
	}
}
