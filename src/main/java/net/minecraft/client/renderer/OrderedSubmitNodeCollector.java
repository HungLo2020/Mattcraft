package net.minecraft.client.renderer;

import net.blaze3d.vertex.PoseStack;
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
import net.minecraft.util.FormattedCharSequence;
import net.minecraft.world.item.ItemDisplayContext;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.Vec3;
import net.minecraft.resources.ResourceLocation;
import org.jetbrains.annotations.Nullable;
import org.joml.Quaternionf;

@Environment(EnvType.CLIENT)
public interface OrderedSubmitNodeCollector {
	void submitHitbox(PoseStack poseStack, EntityRenderState entityRenderState, HitboxesRenderState hitboxesRenderState);
	default void submitHitboxSemantic(PoseStack poseStack, EntityRenderState entityRenderState, HitboxesRenderState hitboxesRenderState) {
		submitHitbox(poseStack, entityRenderState, hitboxesRenderState);
	}

	void submitShadow(PoseStack poseStack, float f, List<EntityRenderState.ShadowPiece> list);
	default void submitShadowSemantic(PoseStack poseStack, float radius, List<EntityRenderState.ShadowPiece> pieces) {
		submitShadow(poseStack, radius, pieces);
	}

	void submitNameTag(PoseStack poseStack, @Nullable Vec3 vec3, int i, Component component, boolean bl, int j, double d, CameraRenderState cameraRenderState);

	/** Records copied name-tag semantics for Rust-owned world-text extraction. */
	default void submitNameTagSemantic(
		PoseStack poseStack, @Nullable Vec3 offset, int packedLight, Component text,
		boolean seeThrough, int width, double distance, CameraRenderState cameraRenderState
	) {
		submitNameTag(poseStack, offset, packedLight, text, seeThrough, width, distance, cameraRenderState);
	}

	void submitText(
		PoseStack poseStack, float f, float g, FormattedCharSequence formattedCharSequence, boolean bl, Font.DisplayMode displayMode, int i, int j, int k, int l
	);

	/** Records copied text semantics for Rust-owned world-text extraction. */
	default void submitTextSemantic(
		PoseStack poseStack, float x, float y, FormattedCharSequence text, boolean shadow,
		Font.DisplayMode mode, int color, int backgroundColor, int packedLight, int packedOverlay
	) {
		submitText(poseStack, x, y, text, shadow, mode, color, backgroundColor, packedLight, packedOverlay);
	}

	void submitFlame(PoseStack poseStack, EntityRenderState entityRenderState, Quaternionf quaternionf);
	default void submitFlameSemantic(PoseStack poseStack, EntityRenderState entityRenderState, Quaternionf quaternionf) {
		submitFlame(poseStack, entityRenderState, quaternionf);
	}

	void submitLeash(PoseStack poseStack, EntityRenderState.LeashState leashState);
	default void submitLeashSemantic(PoseStack poseStack, EntityRenderState.LeashState leashState) {
		submitLeash(poseStack, leashState);
	}

	/** Explicit semantic End Portal cube; Rust owns the animated layer material. */
	default boolean submitEndPortal(PoseStack poseStack, boolean[] faces, float gameTime, int lightCoords) {
		return false;
	}

	/** Explicit semantic End Portal submission for copied Rust extraction. */
	default boolean submitEndPortalSemantic(PoseStack poseStack, boolean[] faces, float gameTime, int lightCoords) {
		return submitEndPortal(poseStack, faces, gameTime, lightCoords);
	}

	<S> void submitModel(
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
	);

	/** Explicit semantic model submission for Rust-owned indexed extraction. */
	default <S> void submitModelSemantic(
		Model<? super S> model, S object, PoseStack poseStack, RenderType renderType,
		int light, int overlay, int color, @Nullable TextureAtlasSprite sprite,
		int outlineColor, @Nullable ModelFeatureRenderer.CrumblingOverlay crumblingOverlay
	) {
		submitModel(model, object, poseStack, renderType, light, overlay, color, sprite, outlineColor, crumblingOverlay);
	}

	/** Convenience semantic overload for model callsites without an atlas sprite. */
	default <S> void submitModelSemantic(
		Model<? super S> model, S object, PoseStack poseStack, RenderType renderType,
		int light, int overlay, int outlineColor,
		@Nullable ModelFeatureRenderer.CrumblingOverlay crumblingOverlay
	) {
		this.submitModelSemantic(model, object, poseStack, renderType, light, overlay, -1,
			null, outlineColor, crumblingOverlay);
	}

	/** Explicit visibility intent: this mesh contributes only an entity outline. */
	default <S> void submitModelOutlineSemanticTexture(
		Model<? super S> model, S state, PoseStack poseStack, RenderType material,
		int light, ResourceLocation textureIdentity, int outlineColor
	) {
		throw new UnsupportedOperationException("outline-only semantic model collector unavailable");
	}

	/** Explicit direct-texture semantic model submit used by emissive feature layers. */
	default <S> void submitModelSemanticTexture(
		Model<? super S> model,
		S object,
		PoseStack poseStack,
		RenderType renderType,
		int lightCoords,
		int overlayCoords,
		int tintedColor,
		ResourceLocation textureIdentity,
		int outlineColor,
		@Nullable ModelFeatureRenderer.CrumblingOverlay crumblingOverlay
	) {
		this.submitModel(model, object, poseStack, renderType, lightCoords, overlayCoords, tintedColor, null, outlineColor, crumblingOverlay);
	}

	default <S> void submitAnimatedModelSemanticTexture(
		Model<? super S> model, S object, PoseStack poseStack, RenderType renderType,
		int lightCoords, int overlayCoords, int tintedColor, ResourceLocation textureIdentity,
		int outlineColor, @Nullable ModelFeatureRenderer.CrumblingOverlay crumblingOverlay,
		float uvOffsetU, float uvOffsetV, int textureWidth, int textureHeight
	) {
		this.submitModelSemanticTexture(model, object, poseStack, renderType, lightCoords, overlayCoords,
			tintedColor, textureIdentity, outlineColor, crumblingOverlay);
	}

	default <S> void submitModel(
		Model<? super S> model,
		S object,
		PoseStack poseStack,
		RenderType renderType,
		int i,
		int j,
		int k,
		@Nullable ModelFeatureRenderer.CrumblingOverlay crumblingOverlay
	) {
		this.submitModel(model, object, poseStack, renderType, i, j, -1, null, k, crumblingOverlay);
	}

	default void submitModelPart(ModelPart modelPart, PoseStack poseStack, RenderType renderType, int i, int j, @Nullable TextureAtlasSprite textureAtlasSprite) {
		this.submitModelPart(modelPart, poseStack, renderType, i, j, textureAtlasSprite, false, false, -1, null, 0);
	}

	default void submitModelPart(
		ModelPart modelPart,
		PoseStack poseStack,
		RenderType renderType,
		int i,
		int j,
		@Nullable TextureAtlasSprite textureAtlasSprite,
		int k,
		@Nullable ModelFeatureRenderer.CrumblingOverlay crumblingOverlay
	) {
		this.submitModelPart(modelPart, poseStack, renderType, i, j, textureAtlasSprite, false, false, k, crumblingOverlay, 0);
	}

	default void submitModelPart(
		ModelPart modelPart, PoseStack poseStack, RenderType renderType, int i, int j, @Nullable TextureAtlasSprite textureAtlasSprite, boolean bl, boolean bl2
	) {
		this.submitModelPart(modelPart, poseStack, renderType, i, j, textureAtlasSprite, bl, bl2, -1, null, 0);
	}

	void submitModelPart(
		ModelPart modelPart,
		PoseStack poseStack,
		RenderType renderType,
		int i,
		int j,
		@Nullable TextureAtlasSprite textureAtlasSprite,
		boolean bl,
		boolean bl2,
		int k,
		@Nullable ModelFeatureRenderer.CrumblingOverlay crumblingOverlay,
		int l
	);

	/** Explicit semantic model-part submission for Rust-owned indexed extraction. */
	default void submitModelPartSemantic(
		ModelPart modelPart, PoseStack poseStack, RenderType renderType, int light, int overlay,
		@Nullable TextureAtlasSprite sprite, boolean emissive, boolean glint, int color,
		@Nullable ModelFeatureRenderer.CrumblingOverlay crumblingOverlay, int outlineColor
	) {
		submitModelPart(modelPart, poseStack, renderType, light, overlay, sprite, emissive, glint, color, crumblingOverlay, outlineColor);
	}

	void submitBlock(PoseStack poseStack, BlockState blockState, int i, int j, int k);

	/** Explicit semantic block-state submission for Rust-owned extraction. */
	default void submitBlockSemantic(PoseStack poseStack, BlockState blockState, int light, int overlay, int outlineColor) {
		submitBlock(poseStack, blockState, light, overlay, outlineColor);
	}

	default void submitBlockDisplay(PoseStack poseStack, BlockState blockState, int i, int j, int k) {
		this.submitBlock(poseStack, blockState, i, j, k);
	}

	default void submitBlockDisplaySemantic(PoseStack poseStack, BlockState blockState, int light, int overlay, int outlineColor) {
		this.submitBlockDisplay(poseStack, blockState, light, overlay, outlineColor);
	}

	default void submitBlockDisplaySemantic(PoseStack poseStack, BlockState blockState, int light, int overlay, int outlineColor, BlockPos tintPos) {
		this.submitBlockDisplay(poseStack, blockState, light, overlay, outlineColor, tintPos);
	}

	default void submitBlockDisplay(PoseStack poseStack, BlockState blockState, int i, int j, int k, BlockPos tintPos) {
		this.submitBlockDisplay(poseStack, blockState, i, j, k);
	}

	/**
	 * Marks the bounded primed-TNT block submit without changing its copied
	 * gameplay inputs. Implementations that do not retain submit provenance
	 * still receive the ordinary block call; the collection used by rendering
	 * records the explicit semantic source before route selection.
	 */
	default void submitPrimedTntBlock(PoseStack poseStack, BlockState blockState, int i, int j, int k) {
		this.submitBlock(poseStack, blockState, i, j, k);
	}

	/** Explicit semantic Primed TNT block submission for copied Rust extraction. */
	default void submitPrimedTntBlockSemantic(PoseStack poseStack, BlockState blockState, int i, int j, int k) {
		this.submitPrimedTntBlock(poseStack, blockState, i, j, k);
	}

	void submitMovingBlock(PoseStack poseStack, MovingBlockRenderState movingBlockRenderState);

	default void submitMovingBlock(PoseStack poseStack, MovingBlockRenderState movingBlockRenderState, SubmitNodeStorage.MovingBlockSubmitSource source) {
		this.submitMovingBlock(poseStack, movingBlockRenderState);
	}

	default void submitMovingBlockSemantic(PoseStack poseStack, MovingBlockRenderState movingBlockRenderState, SubmitNodeStorage.MovingBlockSubmitSource source) {
		this.submitMovingBlock(poseStack, movingBlockRenderState, source);
	}

	void submitBlockModel(PoseStack poseStack, RenderType renderType, BlockStateModel blockStateModel, float f, float g, float h, int i, int j, int k);

	default void submitBlockModelSemantic(PoseStack poseStack, RenderType renderType, BlockStateModel blockStateModel, float red, float green, float blue, int light, int overlay, int outlineColor) {
		submitBlockModel(poseStack, renderType, blockStateModel, red, green, blue, light, overlay, outlineColor);
	}

	void submitItem(
		PoseStack poseStack,
		ItemDisplayContext itemDisplayContext,
		int i,
		int j,
		int k,
		int[] is,
		List<BakedQuad> list,
		RenderType renderType,
		ItemStackRenderState.FoilType foilType
	);

	default void submitItemSemantic(
		PoseStack poseStack, ItemDisplayContext itemDisplayContext, int light, int overlay, int outlineColor,
		int[] tintLayers, List<BakedQuad> quads, RenderType renderType, ItemStackRenderState.FoilType foilType
	) {
		submitItem(poseStack, itemDisplayContext, light, overlay, outlineColor, tintLayers, quads, renderType, foilType);
	}

	void submitCustomGeometry(PoseStack poseStack, RenderType renderType, SubmitNodeCollector.CustomGeometryRenderer customGeometryRenderer);

	/** Explicit semantic custom-geometry submission for copied Rust extraction. */
	default void submitCustomGeometrySemantic(PoseStack poseStack, RenderType renderType, SubmitNodeCollector.CustomGeometryRenderer customGeometryRenderer) {
		submitCustomGeometry(poseStack, renderType, customGeometryRenderer);
	}

	/** Explicit semantic Guardian beam primitive; implementations may reject unsupported backends. */
	default boolean submitGuardianBeam(
		PoseStack poseStack,
		RenderType renderType,
		ResourceLocation textureIdentity,
		float[] vertices,
		float[] uvs,
		int[] colors,
		int lightCoords
	) {
		return false;
	}
	default boolean submitGuardianBeamSemantic(PoseStack poseStack, RenderType renderType, ResourceLocation textureIdentity, float[] vertices, float[] uvs, int[] colors, int lightCoords) {
		return submitGuardianBeam(poseStack, renderType, textureIdentity, vertices, uvs, colors, lightCoords);
	}

	/** Explicit semantic End Crystal beam primitive with copied quad data. */
	default boolean submitCrystalBeam(
		PoseStack poseStack, RenderType renderType, ResourceLocation textureIdentity,
		float[] vertices, float[] uvs, int[] colors, int lightCoords
	) {
		return false;
	}
	default boolean submitCrystalBeamSemantic(PoseStack poseStack, RenderType renderType, ResourceLocation textureIdentity, float[] vertices, float[] uvs, int[] colors, int lightCoords) {
		return submitCrystalBeam(poseStack, renderType, textureIdentity, vertices, uvs, colors, lightCoords);
	}

	/** Explicit semantic textured billboard quad for small projectile renderers. */
	default boolean submitTexturedQuad(
		PoseStack poseStack, RenderType renderType, ResourceLocation textureIdentity,
		float[] vertices, float[] uvs, int color, int lightCoords
	) {
		return false;
	}
	default boolean submitTexturedQuadSemantic(PoseStack poseStack, RenderType renderType, ResourceLocation textureIdentity, float[] vertices, float[] uvs, int color, int lightCoords) {
		return submitTexturedQuad(poseStack, renderType, textureIdentity, vertices, uvs, color, lightCoords);
	}

	/** Explicit semantic translucent textured billboard quad. */
	default boolean submitTranslucentTexturedQuad(
		PoseStack poseStack, RenderType renderType, ResourceLocation textureIdentity,
		float[] vertices, float[] uvs, int color, int lightCoords
	) {
		return false;
	}
	default boolean submitTranslucentTexturedQuadSemantic(PoseStack poseStack, RenderType renderType, ResourceLocation textureIdentity, float[] vertices, float[] uvs, int color, int lightCoords) {
		return submitTranslucentTexturedQuad(poseStack, renderType, textureIdentity, vertices, uvs, color, lightCoords);
	}

	/** Explicit semantic batch of textured quads sharing one copied texture asset. */
	default boolean submitTexturedQuads(
		PoseStack poseStack, RenderType renderType, ResourceLocation textureIdentity,
		float[] vertices, float[] uvs, int[] colors, int lightCoords
	) {
		return false;
	}
	default boolean submitTexturedQuadsSemantic(PoseStack poseStack, RenderType renderType, ResourceLocation textureIdentity, float[] vertices, float[] uvs, int[] colors, int lightCoords) {
		return submitTexturedQuads(poseStack, renderType, textureIdentity, vertices, uvs, colors, lightCoords);
	}

	/** Explicit first-person optical mesh batch with a Rust-owned stencil role. */
	default boolean submitOpticalTexturedQuads(
		PoseStack poseStack, RenderType renderType, ResourceLocation textureIdentity,
		float[] vertices, float[] uvs, int[] colors, int lightCoords, int materialMode
	) {
		return false;
	}
	default boolean submitOpticalTexturedQuadsSemantic(PoseStack poseStack, RenderType renderType, ResourceLocation textureIdentity, float[] vertices, float[] uvs, int[] colors, int lightCoords, int materialMode) {
		return submitOpticalTexturedQuads(poseStack, renderType, textureIdentity, vertices, uvs, colors, lightCoords, materialMode);
	}

	/** Explicit semantic line-list primitive for bounded entity ropes and lines. */
	default boolean submitLineSegments(PoseStack poseStack, float[] endpoints, int color, float lineWidth) {
		return false;
	}
	default boolean submitLineSegmentsSemantic(PoseStack poseStack, float[] endpoints, int color, float lineWidth) {
		return submitLineSegments(poseStack, endpoints, color, lineWidth);
	}

	/** Explicit semantic colored textured-quad list for procedural effects. */
	default boolean submitColoredQuads(PoseStack poseStack, RenderType renderType, float[] vertices, float[] uvs, int[] colors, int lightCoords) {
		return false;
	}
	default boolean submitColoredQuadsSemantic(PoseStack poseStack, RenderType renderType, float[] vertices, float[] uvs, int[] colors, int lightCoords) {
		return submitColoredQuads(poseStack, renderType, vertices, uvs, colors, lightCoords);
	}

	void submitParticleGroup(SubmitNodeCollector.ParticleGroupRenderer particleGroupRenderer);
	default void submitParticleGroupSemantic(SubmitNodeCollector.ParticleGroupRenderer particleGroupRenderer) {
		throw new IllegalStateException(
			"semantic particle submission requires an explicit Rust collector for "
				+ (particleGroupRenderer == null ? "null" : particleGroupRenderer.getClass().getName())
		);
	}
}
