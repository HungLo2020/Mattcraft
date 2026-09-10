package net.minecraft.client.renderer;

import net.blaze3d.vertex.PoseStack;
import it.unimi.dsi.fastutil.ints.Int2ObjectAVLTreeMap;
import java.util.List;
import net.minecraft.api.EnvType;
import net.minecraft.api.Environment;
import net.minecraft.core.BlockPos;
import net.minecraft.client.gui.Font;
import net.minecraft.client.model.Model;
import net.minecraft.client.model.geom.ModelPart;
import net.minecraft.client.renderer.block.MovingBlockRenderState;
import net.minecraft.client.renderer.block.model.BakedQuad;
import net.minecraft.client.renderer.block.model.BlockStateModel;
import net.minecraft.client.renderer.entity.state.EntityRenderState;
import net.minecraft.client.renderer.entity.state.HitboxesRenderState;
import net.minecraft.client.renderer.feature.ModelFeatureRenderer;
import net.minecraft.client.renderer.item.ItemStackRenderState;
import net.minecraft.client.renderer.state.CameraRenderState;
import net.minecraft.client.renderer.texture.TextureAtlasSprite;
import net.minecraft.network.chat.Component;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.util.FormattedCharSequence;
import net.minecraft.world.item.ItemDisplayContext;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.Vec3;
import net.sodium.client.render.frapi.render.OrderedSubmitNodeCollectorExtension;
import org.jetbrains.annotations.Nullable;
import org.joml.Matrix4f;
import org.joml.Quaternionf;
import org.joml.Vector3f;

@Environment(EnvType.CLIENT)
public class SubmitNodeStorage implements SubmitNodeCollector, OrderedSubmitNodeCollectorExtension {
	private final Int2ObjectAVLTreeMap<SubmitNodeCollection> submitsPerOrder = new Int2ObjectAVLTreeMap<>();

	private static boolean rustWholeFrame() {
		return net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()
			|| net.vulkanic.VulkanicAPI.isVulkanBackendSelected();
	}

	public SubmitNodeCollection order(int i) {
		return this.submitsPerOrder.computeIfAbsent(i, ix -> new SubmitNodeCollection(this));
	}
	
	// Sodium FRAPI: Ordered item submission support
	@Override
	public void fabric_submitItem(PoseStack matrices, ItemDisplayContext displayContext, int light, int overlay, int outlineColors, int[] tintLayers, List<BakedQuad> quads, RenderType renderLayer, ItemStackRenderState.FoilType foilType, net.fabricmc.fabric.api.renderer.v1.mesh.MeshView mesh) {
		SubmitNodeCollection queue = order(0);

		if (queue instanceof OrderedSubmitNodeCollectorExtension access) {
			access.fabric_submitItem(matrices, displayContext, light, overlay, outlineColors, tintLayers, quads, renderLayer, foilType, mesh);
		} else {
			queue.submitItem(matrices, displayContext, light, overlay, outlineColors, tintLayers, quads, renderLayer, foilType);
		}
	}

	@Override
	public void submitHitbox(PoseStack poseStack, EntityRenderState entityRenderState, HitboxesRenderState hitboxesRenderState) {
		this.order(0).submitHitbox(poseStack, entityRenderState, hitboxesRenderState);
	}

	@Override
	public void submitHitboxSemantic(PoseStack poseStack, EntityRenderState entityRenderState, HitboxesRenderState hitboxesRenderState) {
		this.order(0).submitHitboxSemantic(poseStack, entityRenderState, hitboxesRenderState);
	}

	@Override
	public void submitShadow(PoseStack poseStack, float f, List<EntityRenderState.ShadowPiece> list) {
		this.order(0).submitShadow(poseStack, f, list);
	}

	@Override
	public void submitShadowSemantic(PoseStack poseStack, float radius, List<EntityRenderState.ShadowPiece> pieces) {
		this.order(0).submitShadowSemantic(poseStack, radius, pieces);
	}

	@Override
	public void submitNameTag(
		PoseStack poseStack, @Nullable Vec3 vec3, int i, Component component, boolean bl, int j, double d, CameraRenderState cameraRenderState
	) {
		this.order(0).submitNameTag(poseStack, vec3, i, component, bl, j, d, cameraRenderState);
	}

	@Override
	public void submitNameTagSemantic(
		PoseStack poseStack, @Nullable Vec3 offset, int packedLight, Component text, boolean seeThrough,
		int width, double distance, CameraRenderState cameraRenderState
	) {
		this.order(0).submitNameTagSemantic(poseStack, offset, packedLight, text, seeThrough, width, distance, cameraRenderState);
	}

	@Override
	public void submitText(
		PoseStack poseStack, float f, float g, FormattedCharSequence formattedCharSequence, boolean bl, Font.DisplayMode displayMode, int i, int j, int k, int l
	) {
		this.order(0).submitText(poseStack, f, g, formattedCharSequence, bl, displayMode, i, j, k, l);
	}

	@Override
	public void submitTextSemantic(
		PoseStack poseStack, float x, float y, FormattedCharSequence text, boolean shadow,
		Font.DisplayMode mode, int color, int backgroundColor, int packedLight, int packedOverlay
	) {
		this.order(0).submitTextSemantic(poseStack, x, y, text, shadow, mode, color, backgroundColor, packedLight, packedOverlay);
	}

	@Override
	public boolean submitGuardianBeam(PoseStack poseStack, RenderType renderType, net.minecraft.resources.ResourceLocation textureIdentity, float[] vertices, float[] uvs, int[] colors, int lightCoords) {
		return this.order(0).submitGuardianBeam(poseStack, renderType, textureIdentity, vertices, uvs, colors, lightCoords);
	}
	public boolean submitGuardianBeamSemantic(PoseStack poseStack, RenderType renderType, net.minecraft.resources.ResourceLocation textureIdentity, float[] vertices, float[] uvs, int[] colors, int lightCoords) {
		return this.order(0).submitGuardianBeamSemantic(poseStack, renderType, textureIdentity, vertices, uvs, colors, lightCoords);
	}

	@Override
	public boolean submitCrystalBeam(PoseStack poseStack, RenderType renderType, net.minecraft.resources.ResourceLocation textureIdentity, float[] vertices, float[] uvs, int[] colors, int lightCoords) {
		return this.order(0).submitCrystalBeam(poseStack, renderType, textureIdentity, vertices, uvs, colors, lightCoords);
	}
	public boolean submitCrystalBeamSemantic(PoseStack poseStack, RenderType renderType, net.minecraft.resources.ResourceLocation textureIdentity, float[] vertices, float[] uvs, int[] colors, int lightCoords) {
		return this.order(0).submitCrystalBeamSemantic(poseStack, renderType, textureIdentity, vertices, uvs, colors, lightCoords);
	}

	@Override
	public boolean submitTexturedQuad(PoseStack poseStack, RenderType renderType, net.minecraft.resources.ResourceLocation textureIdentity, float[] vertices, float[] uvs, int color, int lightCoords) {
		return this.order(0).submitTexturedQuad(poseStack, renderType, textureIdentity, vertices, uvs, color, lightCoords);
	}
	public boolean submitTexturedQuadSemantic(PoseStack poseStack, RenderType renderType, net.minecraft.resources.ResourceLocation textureIdentity, float[] vertices, float[] uvs, int color, int lightCoords) {
		return this.order(0).submitTexturedQuadSemantic(poseStack, renderType, textureIdentity, vertices, uvs, color, lightCoords);
	}

	@Override
	public boolean submitTranslucentTexturedQuad(PoseStack poseStack, RenderType renderType, net.minecraft.resources.ResourceLocation textureIdentity, float[] vertices, float[] uvs, int color, int lightCoords) {
		return this.order(0).submitTranslucentTexturedQuad(poseStack, renderType, textureIdentity, vertices, uvs, color, lightCoords);
	}
	public boolean submitTranslucentTexturedQuadSemantic(PoseStack poseStack, RenderType renderType, net.minecraft.resources.ResourceLocation textureIdentity, float[] vertices, float[] uvs, int color, int lightCoords) {
		return this.order(0).submitTranslucentTexturedQuadSemantic(poseStack, renderType, textureIdentity, vertices, uvs, color, lightCoords);
	}

	@Override
	public boolean submitTexturedQuads(PoseStack poseStack, RenderType renderType, net.minecraft.resources.ResourceLocation textureIdentity, float[] vertices, float[] uvs, int[] colors, int lightCoords) {
		return this.order(0).submitTexturedQuads(poseStack, renderType, textureIdentity, vertices, uvs, colors, lightCoords);
	}
	public boolean submitTexturedQuadsSemantic(PoseStack poseStack, RenderType renderType, net.minecraft.resources.ResourceLocation textureIdentity, float[] vertices, float[] uvs, int[] colors, int lightCoords) {
		return this.order(0).submitTexturedQuadsSemantic(poseStack, renderType, textureIdentity, vertices, uvs, colors, lightCoords);
	}

	@Override
	public boolean submitOpticalTexturedQuads(PoseStack poseStack, RenderType renderType, net.minecraft.resources.ResourceLocation textureIdentity, float[] vertices, float[] uvs, int[] colors, int lightCoords, int materialMode) {
		return this.order(0).submitOpticalTexturedQuads(poseStack, renderType, textureIdentity, vertices, uvs, colors, lightCoords, materialMode);
	}
	public boolean submitOpticalTexturedQuadsSemantic(PoseStack poseStack, RenderType renderType, net.minecraft.resources.ResourceLocation textureIdentity, float[] vertices, float[] uvs, int[] colors, int lightCoords, int materialMode) {
		return this.order(0).submitOpticalTexturedQuadsSemantic(poseStack, renderType, textureIdentity, vertices, uvs, colors, lightCoords, materialMode);
	}

	@Override
	public boolean submitLineSegments(PoseStack poseStack, float[] endpoints, int color, float lineWidth) {
		return this.order(0).submitLineSegments(poseStack, endpoints, color, lineWidth);
	}
	public boolean submitLineSegmentsSemantic(PoseStack poseStack, float[] endpoints, int color, float lineWidth) {
		return this.order(0).submitLineSegmentsSemantic(poseStack, endpoints, color, lineWidth);
	}

	@Override
	public boolean submitColoredQuads(PoseStack poseStack, RenderType renderType, float[] vertices, float[] uvs, int[] colors, int lightCoords) {
		return this.order(0).submitColoredQuads(poseStack, renderType, vertices, uvs, colors, lightCoords);
	}
	public boolean submitColoredQuadsSemantic(PoseStack poseStack, RenderType renderType, float[] vertices, float[] uvs, int[] colors, int lightCoords) {
		return this.order(0).submitColoredQuadsSemantic(poseStack, renderType, vertices, uvs, colors, lightCoords);
	}

	/** Stores text copied from a semantic extraction callback without Iris state capture. */
	public void submitTextSemantic(
		int order, PoseStack poseStack, float f, float g, FormattedCharSequence formattedCharSequence, boolean bl, Font.DisplayMode displayMode, int i, int j, int k, int l
	) {
		this.order(order).submitTextSemantic(poseStack, f, g, formattedCharSequence, bl, displayMode, i, j, k, l);
	}

	@Override
	public void submitFlame(PoseStack poseStack, EntityRenderState entityRenderState, Quaternionf quaternionf) {
		this.order(0).submitFlame(poseStack, entityRenderState, quaternionf);
	}

	@Override
	public void submitFlameSemantic(PoseStack poseStack, EntityRenderState entityRenderState, Quaternionf quaternionf) {
		this.order(0).submitFlameSemantic(poseStack, entityRenderState, quaternionf);
	}

	@Override
	public void submitLeash(PoseStack poseStack, EntityRenderState.LeashState leashState) {
		this.order(0).submitLeash(poseStack, leashState);
	}

	@Override
	public void submitLeashSemantic(PoseStack poseStack, EntityRenderState.LeashState leashState) {
		this.order(0).submitLeashSemantic(poseStack, leashState);
	}

	@Override
	public <S> void submitModel(
		Model<? super S> model,
		S object,
		PoseStack poseStack,
		RenderType renderType,
		int i,
		int j,
		int k,
		@Nullable TextureAtlasSprite textureAtlasSprite,
		int l,
		@Nullable ModelFeatureRenderer.CrumblingOverlay crumblingOverlay
	) {
		this.order(0).submitModel(model, object, poseStack, renderType, i, j, k, textureAtlasSprite, l, crumblingOverlay);
	}

	@Override
	public <S> void submitModelSemantic(
		Model<? super S> model, S object, PoseStack poseStack, RenderType renderType,
		int light, int overlay, int color, @Nullable TextureAtlasSprite sprite,
		int outlineColor, @Nullable ModelFeatureRenderer.CrumblingOverlay crumblingOverlay
	) {
		this.order(0).submitModelSemantic(model, object, poseStack, renderType, light, overlay, color, sprite, outlineColor, crumblingOverlay);
	}

	@Override
	public <S> void submitModelSemanticTexture(
		Model<? super S> model, S object, PoseStack poseStack, RenderType renderType,
		int lightCoords, int overlayCoords, int tintedColor,
		ResourceLocation textureIdentity, int outlineColor,
		@Nullable ModelFeatureRenderer.CrumblingOverlay crumblingOverlay
	) {
		this.order(0).submitModelSemanticTexture(model, object, poseStack, renderType,
			lightCoords, overlayCoords, tintedColor, textureIdentity, outlineColor, crumblingOverlay);
	}

	@Override
	public <S> void submitModelOutlineSemanticTexture(Model<? super S> model, S state,
		PoseStack poseStack, RenderType material, int light, ResourceLocation textureIdentity, int outlineColor) {
		this.order(0).submitModelOutlineSemanticTexture(model, state, poseStack, material, light, textureIdentity, outlineColor);
	}

	@Override
	public void submitModelPart(
		ModelPart modelPart,
		PoseStack poseStack,
		RenderType renderType,
		int i,
		int j,
		@Nullable TextureAtlasSprite textureAtlasSprite,
		boolean bl,
		boolean bl2,
		int k,
		ModelFeatureRenderer.CrumblingOverlay crumblingOverlay,
		int l
	) {
		this.order(0).submitModelPart(modelPart, poseStack, renderType, i, j, textureAtlasSprite, bl, bl2, k, crumblingOverlay, l);
	}

	@Override
	public void submitModelPartSemantic(
		ModelPart modelPart, PoseStack poseStack, RenderType renderType, int light, int overlay,
		@Nullable TextureAtlasSprite sprite, boolean emissive, boolean glint, int color,
		@Nullable ModelFeatureRenderer.CrumblingOverlay crumblingOverlay, int outlineColor
	) {
		this.order(0).submitModelPartSemantic(modelPart, poseStack, renderType, light, overlay, sprite, emissive, glint, color, crumblingOverlay, outlineColor);
	}

	@Override
	public void submitBlock(PoseStack poseStack, BlockState blockState, int i, int j, int k) {
		this.order(0).submitBlock(poseStack, blockState, i, j, k);
	}

	@Override
	public void submitBlockSemantic(PoseStack poseStack, BlockState blockState, int light, int overlay, int outlineColor) {
		this.order(0).submitBlockSemantic(poseStack, blockState, light, overlay, outlineColor);
	}

	@Override
	public void submitBlockDisplay(PoseStack poseStack, BlockState blockState, int i, int j, int k) {
		this.order(0).submitBlockDisplay(poseStack, blockState, i, j, k);
	}

	@Override
	public void submitBlockDisplaySemantic(PoseStack poseStack, BlockState blockState, int light, int overlay, int outlineColor) {
		this.order(0).submitBlockDisplaySemantic(poseStack, blockState, light, overlay, outlineColor);
	}

	@Override
	public void submitBlockDisplaySemantic(PoseStack poseStack, BlockState blockState, int light, int overlay, int outlineColor, BlockPos tintPos) {
		this.order(0).submitBlockDisplaySemantic(poseStack, blockState, light, overlay, outlineColor, tintPos);
	}

	@Override
	public void submitPrimedTntBlock(PoseStack poseStack, BlockState blockState, int i, int j, int k) {
		this.order(0).submitPrimedTntBlock(poseStack, blockState, i, j, k);
	}

	public void submitPrimedTntBlockSemantic(PoseStack poseStack, BlockState blockState, int i, int j, int k) {
		this.order(0).submitPrimedTntBlockSemantic(poseStack, blockState, i, j, k);
	}

	@Override
	public void submitMovingBlock(PoseStack poseStack, MovingBlockRenderState movingBlockRenderState) {
		this.submitMovingBlock(poseStack, movingBlockRenderState, MovingBlockSubmitSource.UNKNOWN);
	}

	@Override
	public void submitMovingBlock(PoseStack poseStack, MovingBlockRenderState movingBlockRenderState, MovingBlockSubmitSource source) {
		this.order(0).submitMovingBlock(poseStack, movingBlockRenderState, source);
	}

	@Override
	public void submitMovingBlockSemantic(PoseStack poseStack, MovingBlockRenderState movingBlockRenderState, MovingBlockSubmitSource source) {
		this.order(0).submitMovingBlockSemantic(poseStack, movingBlockRenderState, source);
	}

	@Override
	public void submitBlockModel(PoseStack poseStack, RenderType renderType, BlockStateModel blockStateModel, float f, float g, float h, int i, int j, int k) {
		this.order(0).submitBlockModel(poseStack, renderType, blockStateModel, f, g, h, i, j, k);
	}

	@Override
	public void submitBlockModelSemantic(PoseStack poseStack, RenderType renderType, BlockStateModel blockStateModel, float red, float green, float blue, int light, int overlay, int outlineColor) {
		this.order(0).submitBlockModelSemantic(poseStack, renderType, blockStateModel, red, green, blue, light, overlay, outlineColor);
	}

	@Override
	public void submitItem(
		PoseStack poseStack,
		ItemDisplayContext itemDisplayContext,
		int i,
		int j,
		int k,
		int[] is,
		List<BakedQuad> list,
		RenderType renderType,
		ItemStackRenderState.FoilType foilType
	) {
		this.order(0).submitItem(poseStack, itemDisplayContext, i, j, k, is, list, renderType, foilType);
	}

	@Override
	public void submitItemSemantic(
		PoseStack poseStack, ItemDisplayContext itemDisplayContext, int light, int overlay, int outlineColor,
		int[] tintLayers, List<BakedQuad> quads, RenderType renderType, ItemStackRenderState.FoilType foilType
	) {
		this.order(0).submitItemSemantic(poseStack, itemDisplayContext, light, overlay, outlineColor, tintLayers, quads, renderType, foilType);
	}

	@Override
	public void submitCustomGeometry(PoseStack poseStack, RenderType renderType, SubmitNodeCollector.CustomGeometryRenderer customGeometryRenderer) {
		this.order(0).submitCustomGeometry(poseStack, renderType, customGeometryRenderer);
	}

	public void submitCustomGeometrySemantic(PoseStack poseStack, RenderType renderType, SubmitNodeCollector.CustomGeometryRenderer customGeometryRenderer) {
		this.order(0).submitCustomGeometrySemantic(poseStack, renderType, customGeometryRenderer);
	}

	@Override
	public void submitParticleGroup(SubmitNodeCollector.ParticleGroupRenderer particleGroupRenderer) {
		this.order(0).submitParticleGroup(particleGroupRenderer);
	}

	@Override
	public void submitParticleGroupSemantic(SubmitNodeCollector.ParticleGroupRenderer particleGroupRenderer) {
		this.order(0).submitParticleGroupSemantic(particleGroupRenderer);
	}

	public void clear() {
		this.submitsPerOrder.values().forEach(SubmitNodeCollection::clear);
	}

	public void endFrame() {
		this.submitsPerOrder.values().removeIf(submitNodeCollection -> !submitNodeCollection.wasUsed());
		this.submitsPerOrder.values().forEach(SubmitNodeCollection::endFrame);
	}

	public Int2ObjectAVLTreeMap<SubmitNodeCollection> getSubmitsPerOrder() {
		return this.submitsPerOrder;
	}

	/**
	 * Aggregates copied feature-family counts after ordinary Java extraction.
	 * It deliberately exposes no renderer state: Rust uses it to reject a
	 * selected-source frame before unsupported work could be silently omitted.
	 */
	public SubmitNodeCollection.WorldFeatureCoverageSnapshot worldFeatureCoverageSnapshot() {
		int modelSubmits = 0;
		int modelPartSubmits = 0;
		int blockModelSubmits = 0;
		int ordinaryBlockSubmits = 0;
		int itemSubmits = 0;
		int customGeometrySubmits = 0;
		int shadowSubmits = 0;
		int flameSubmits = 0;
		int nameTagSubmits = 0;
		int textSubmits = 0;
		int hitboxSubmits = 0;
		int leashSubmits = 0;
		int particleGroupSubmits = 0;
		for (SubmitNodeCollection.WorldFeatureCoverageSnapshot coverage : this.submitsPerOrder.values()
			.stream().map(SubmitNodeCollection::worldFeatureCoverageSnapshot).toList()) {
			modelSubmits += coverage.modelSubmits();
			modelPartSubmits += coverage.modelPartSubmits();
			blockModelSubmits += coverage.blockModelSubmits();
			ordinaryBlockSubmits += coverage.ordinaryBlockSubmits();
			itemSubmits += coverage.itemSubmits();
			customGeometrySubmits += coverage.customGeometrySubmits();
			shadowSubmits += coverage.shadowSubmits();
			flameSubmits += coverage.flameSubmits();
			nameTagSubmits += coverage.nameTagSubmits();
			textSubmits += coverage.textSubmits();
			hitboxSubmits += coverage.hitboxSubmits();
			leashSubmits += coverage.leashSubmits();
			particleGroupSubmits += coverage.particleGroupSubmits();
		}
		return new SubmitNodeCollection.WorldFeatureCoverageSnapshot(
			modelSubmits, modelPartSubmits, blockModelSubmits, ordinaryBlockSubmits, itemSubmits,
			customGeometrySubmits, shadowSubmits, flameSubmits, nameTagSubmits, textSubmits,
			hitboxSubmits, leashSubmits, particleGroupSubmits
		);
	}

	@Environment(EnvType.CLIENT)
	public record BlockModelSubmit(
		PoseStack.Pose pose, RenderType renderType, BlockStateModel model, float r, float g, float b, int lightCoords, int overlayCoords, int outlineColor
	) {
	}

	@Environment(EnvType.CLIENT)
	public enum BlockSubmitSource {
		ORDINARY,
		BLOCK_DISPLAY,
		PRIMED_TNT
	}

	@Environment(EnvType.CLIENT)
	public record BlockSubmit(PoseStack.Pose pose, BlockState state, int lightCoords, int overlayCoords, int outlineColor, BlockSubmitSource source, BlockPos tintPos) {
	}

	@Environment(EnvType.CLIENT)
	public record CustomGeometrySubmit(PoseStack.Pose pose, SubmitNodeCollector.CustomGeometryRenderer customGeometryRenderer) implements net.irisshaders.iris.mixinterface.ModelStorage {
		private static final java.util.WeakHashMap<CustomGeometrySubmit, ModelStorageData> STORAGE = new java.util.WeakHashMap<>();
		
		public CustomGeometrySubmit {
			if (!SubmitNodeStorage.rustWholeFrame()) {
				// Iris: Capture state on construction
				ModelStorageData data = STORAGE.computeIfAbsent(this, k -> new ModelStorageData());
				data.entityId = net.irisshaders.iris.uniforms.CapturedRenderingState.INSTANCE.getCurrentRenderedEntity();
				data.beId = net.irisshaders.iris.uniforms.CapturedRenderingState.INSTANCE.getCurrentRenderedBlockEntity();
				data.itemId = net.irisshaders.iris.uniforms.CapturedRenderingState.INSTANCE.getCurrentRenderedItem();
				data.isRenderingBEs = net.irisshaders.iris.vertices.ImmediateState.isRenderingBEs;
			}
		}
		
		@Override
		public void iris$capture() {
			if (SubmitNodeStorage.rustWholeFrame()) return;
			ModelStorageData data = STORAGE.computeIfAbsent(this, k -> new ModelStorageData());
			data.entityId = net.irisshaders.iris.uniforms.CapturedRenderingState.INSTANCE.getCurrentRenderedEntity();
			data.beId = net.irisshaders.iris.uniforms.CapturedRenderingState.INSTANCE.getCurrentRenderedBlockEntity();
			data.itemId = net.irisshaders.iris.uniforms.CapturedRenderingState.INSTANCE.getCurrentRenderedItem();
			data.isRenderingBEs = net.irisshaders.iris.vertices.ImmediateState.isRenderingBEs;
		}
		
		@Override
		public void iris$set() {
			if (SubmitNodeStorage.rustWholeFrame()) return;
			ModelStorageData data = STORAGE.get(this);
			if (data != null) {
				net.irisshaders.iris.uniforms.CapturedRenderingState.INSTANCE.setCurrentEntity(data.entityId);
				net.irisshaders.iris.uniforms.CapturedRenderingState.INSTANCE.setCurrentBlockEntity(data.beId);
				net.irisshaders.iris.uniforms.CapturedRenderingState.INSTANCE.setCurrentRenderedItem(data.itemId);
			}
		}
		
		@Override
		public boolean iris$wasBE() {
			ModelStorageData data = STORAGE.get(this);
			return data != null && data.isRenderingBEs;
		}
		
		private static class ModelStorageData {
			int entityId, beId, itemId;
			boolean isRenderingBEs;
		}
	}

	@Environment(EnvType.CLIENT)
	public record FlameSubmit(PoseStack.Pose pose, EntityRenderState entityRenderState, Quaternionf rotation) {
	}

	@Environment(EnvType.CLIENT)
	public record HitboxSubmit(Matrix4f pose, EntityRenderState entityRenderState, HitboxesRenderState hitboxesRenderState) {
	}

	@Environment(EnvType.CLIENT)
	public record ItemSubmit(
		PoseStack.Pose pose,
		ItemDisplayContext displayContext,
		int lightCoords,
		int overlayCoords,
		int outlineColor,
		int[] tintLayers,
		List<BakedQuad> quads,
		RenderType renderType,
		ItemStackRenderState.FoilType foilType
	) implements net.irisshaders.iris.mixinterface.ModelStorage {
		// Iris: ModelStorage implementation using WeakHashMap to store per-instance state
		private static final java.util.WeakHashMap<ItemSubmit, ModelStorageData> STORAGE = new java.util.WeakHashMap<>();
		
		@Override
		public void iris$capture() {
			if (SubmitNodeStorage.rustWholeFrame()) return;
			ModelStorageData data = STORAGE.computeIfAbsent(this, k -> new ModelStorageData());
			data.entityId = net.irisshaders.iris.uniforms.CapturedRenderingState.INSTANCE.getCurrentRenderedEntity();
			data.beId = net.irisshaders.iris.uniforms.CapturedRenderingState.INSTANCE.getCurrentRenderedBlockEntity();
			data.itemId = net.irisshaders.iris.uniforms.CapturedRenderingState.INSTANCE.getCurrentRenderedItem();
			data.isRenderingBEs = net.irisshaders.iris.vertices.ImmediateState.isRenderingBEs;
		}
		
		@Override
		public void iris$set() {
			if (SubmitNodeStorage.rustWholeFrame()) return;
			ModelStorageData data = STORAGE.get(this);
			if (data != null) {
				net.irisshaders.iris.uniforms.CapturedRenderingState.INSTANCE.setCurrentEntity(data.entityId);
				net.irisshaders.iris.uniforms.CapturedRenderingState.INSTANCE.setCurrentBlockEntity(data.beId);
				net.irisshaders.iris.uniforms.CapturedRenderingState.INSTANCE.setCurrentRenderedItem(data.itemId);
			}
		}
		
		@Override
		public boolean iris$wasBE() {
			ModelStorageData data = STORAGE.get(this);
			return data != null && data.isRenderingBEs;
		}
		
		private static class ModelStorageData {
			int entityId, beId, itemId;
			boolean isRenderingBEs;
		}
	}

	@Environment(EnvType.CLIENT)
	public record LeashSubmit(Matrix4f pose, EntityRenderState.LeashState leashState) {
	}

	@Environment(EnvType.CLIENT)
	public record ModelPartSubmit(
		PoseStack.Pose pose,
		ModelPart modelPart,
		int lightCoords,
		int overlayCoords,
		@Nullable TextureAtlasSprite sprite,
		boolean sheeted,
		boolean hasFoil,
		int tintedColor,
		@Nullable ModelFeatureRenderer.CrumblingOverlay crumblingOverlay,
		int outlineColor
	) implements net.irisshaders.iris.mixinterface.ModelStorage {
		private static final java.util.WeakHashMap<ModelPartSubmit, ModelStorageData> STORAGE = new java.util.WeakHashMap<>();
		
		@Override
		public void iris$capture() {
			if (SubmitNodeStorage.rustWholeFrame()) return;
			ModelStorageData data = STORAGE.computeIfAbsent(this, k -> new ModelStorageData());
			data.entityId = net.irisshaders.iris.uniforms.CapturedRenderingState.INSTANCE.getCurrentRenderedEntity();
			data.beId = net.irisshaders.iris.uniforms.CapturedRenderingState.INSTANCE.getCurrentRenderedBlockEntity();
			data.itemId = net.irisshaders.iris.uniforms.CapturedRenderingState.INSTANCE.getCurrentRenderedItem();
			data.isRenderingBEs = net.irisshaders.iris.vertices.ImmediateState.isRenderingBEs;
		}
		
		@Override
		public void iris$set() {
			if (SubmitNodeStorage.rustWholeFrame()) return;
			ModelStorageData data = STORAGE.get(this);
			if (data != null) {
				net.irisshaders.iris.uniforms.CapturedRenderingState.INSTANCE.setCurrentEntity(data.entityId);
				net.irisshaders.iris.uniforms.CapturedRenderingState.INSTANCE.setCurrentBlockEntity(data.beId);
				net.irisshaders.iris.uniforms.CapturedRenderingState.INSTANCE.setCurrentRenderedItem(data.itemId);
			}
		}
		
		@Override
		public boolean iris$wasBE() {
			ModelStorageData data = STORAGE.get(this);
			return data != null && data.isRenderingBEs;
		}
		
		private static class ModelStorageData {
			int entityId, beId, itemId;
			boolean isRenderingBEs;
		}
	}

	@Environment(EnvType.CLIENT)
	public record ModelSubmit<S>(
		PoseStack.Pose pose,
		Model<? super S> model,
		S state,
		int lightCoords,
		int overlayCoords,
		int tintedColor,
		@Nullable TextureAtlasSprite sprite,
		int outlineColor,
		@Nullable ModelFeatureRenderer.CrumblingOverlay crumblingOverlay
	) implements net.irisshaders.iris.mixinterface.ModelStorage {
		private static final java.util.WeakHashMap<ModelSubmit<?>, ModelStorageData> STORAGE = new java.util.WeakHashMap<>();
		
		@Override
		public void iris$capture() {
			if (SubmitNodeStorage.rustWholeFrame()) return;
			ModelStorageData data = STORAGE.computeIfAbsent(this, k -> new ModelStorageData());
			data.entityId = net.irisshaders.iris.uniforms.CapturedRenderingState.INSTANCE.getCurrentRenderedEntity();
			data.beId = net.irisshaders.iris.uniforms.CapturedRenderingState.INSTANCE.getCurrentRenderedBlockEntity();
			data.itemId = net.irisshaders.iris.uniforms.CapturedRenderingState.INSTANCE.getCurrentRenderedItem();
			data.isRenderingBEs = net.irisshaders.iris.vertices.ImmediateState.isRenderingBEs;
		}
		
		@Override
		public void iris$set() {
			if (SubmitNodeStorage.rustWholeFrame()) return;
			ModelStorageData data = STORAGE.get(this);
			if (data != null) {
				net.irisshaders.iris.uniforms.CapturedRenderingState.INSTANCE.setCurrentEntity(data.entityId);
				net.irisshaders.iris.uniforms.CapturedRenderingState.INSTANCE.setCurrentBlockEntity(data.beId);
				net.irisshaders.iris.uniforms.CapturedRenderingState.INSTANCE.setCurrentRenderedItem(data.itemId);
			}
		}
		
		@Override
		public boolean iris$wasBE() {
			ModelStorageData data = STORAGE.get(this);
			return data != null && data.isRenderingBEs;
		}
		
		private static class ModelStorageData {
			int entityId, beId, itemId;
			boolean isRenderingBEs;
		}
	}

	@Environment(EnvType.CLIENT)
	public enum MovingBlockSubmitSource {
		UNKNOWN,
		FALLING_BLOCK,
		PISTON
	}

	@Environment(EnvType.CLIENT)
	public record MovingBlockSubmit(Matrix4f pose, MovingBlockRenderState movingBlockRenderState, MovingBlockSubmitSource source) {
	}

	@Environment(EnvType.CLIENT)
	public record NameTagSubmit(Matrix4f pose, float x, float y, Component text, int lightCoords, int color, int backgroundColor, double distanceToCameraSq) {
	}

	@Environment(EnvType.CLIENT)
	public record ShadowSubmit(Matrix4f pose, float radius, List<EntityRenderState.ShadowPiece> pieces) {
	}

	@Environment(EnvType.CLIENT)
	public record TextSubmit(
		Matrix4f pose,
		float x,
		float y,
		FormattedCharSequence string,
		boolean dropShadow,
		Font.DisplayMode displayMode,
		int lightCoords,
		int color,
		int backgroundColor,
		int outlineColor,
		int blockEntityId
	) implements net.irisshaders.iris.mixinterface.ModelStorage {
		public TextSubmit(Matrix4f pose, float x, float y, FormattedCharSequence string, boolean dropShadow,
			Font.DisplayMode displayMode, int lightCoords, int color, int backgroundColor, int outlineColor) {
			this(pose, x, y, string, dropShadow, displayMode, lightCoords, color, backgroundColor, outlineColor,
				net.vulkanic.bridge.VulkanicGalBridge.activeSemanticBlockEntityId());
		}
		private static final java.util.WeakHashMap<TextSubmit, ModelStorageData> STORAGE = new java.util.WeakHashMap<>();
		
		@Override
		public void iris$capture() {
			if (SubmitNodeStorage.rustWholeFrame()) return;
			ModelStorageData data = STORAGE.computeIfAbsent(this, k -> new ModelStorageData());
			data.entityId = net.irisshaders.iris.uniforms.CapturedRenderingState.INSTANCE.getCurrentRenderedEntity();
			data.beId = net.irisshaders.iris.uniforms.CapturedRenderingState.INSTANCE.getCurrentRenderedBlockEntity();
			data.itemId = net.irisshaders.iris.uniforms.CapturedRenderingState.INSTANCE.getCurrentRenderedItem();
			data.isRenderingBEs = net.irisshaders.iris.vertices.ImmediateState.isRenderingBEs;
		}
		
		@Override
		public void iris$set() {
			if (SubmitNodeStorage.rustWholeFrame()) return;
			ModelStorageData data = STORAGE.get(this);
			if (data != null) {
				net.irisshaders.iris.uniforms.CapturedRenderingState.INSTANCE.setCurrentEntity(data.entityId);
				net.irisshaders.iris.uniforms.CapturedRenderingState.INSTANCE.setCurrentBlockEntity(data.beId);
				net.irisshaders.iris.uniforms.CapturedRenderingState.INSTANCE.setCurrentRenderedItem(data.itemId);
			}
		}
		
		@Override
		public boolean iris$wasBE() {
			ModelStorageData data = STORAGE.get(this);
			return data != null && data.isRenderingBEs;
		}
		
		private static class ModelStorageData {
			int entityId, beId, itemId;
			boolean isRenderingBEs;
		}
	}

	@Environment(EnvType.CLIENT)
	public record TranslucentModelSubmit<S>(SubmitNodeStorage.ModelSubmit<S> modelSubmit, RenderType renderType, Vector3f position) {
	}
}
