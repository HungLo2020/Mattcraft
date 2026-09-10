package net.minecraft.client.renderer.special;

import net.blaze3d.vertex.PoseStack;
import com.mojang.serialization.MapCodec;
import java.util.Objects;
import java.util.Set;
import net.minecraft.api.EnvType;
import net.minecraft.api.Environment;
import net.minecraft.client.model.ShieldModel;
import net.minecraft.client.model.geom.ModelLayers;
import net.minecraft.client.renderer.SubmitNodeCollector;
import net.minecraft.client.renderer.blockentity.BannerRenderer;
import net.minecraft.client.resources.model.Material;
import net.minecraft.client.resources.model.MaterialSet;
import net.minecraft.client.resources.model.ModelBakery;
import net.minecraft.core.component.DataComponentMap;
import net.minecraft.core.component.DataComponents;
import net.minecraft.util.Unit;
import net.minecraft.world.item.DyeColor;
import net.minecraft.world.item.ItemDisplayContext;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.block.entity.BannerPatternLayers;
import org.jetbrains.annotations.Nullable;
import org.joml.Vector3f;

@Environment(EnvType.CLIENT)
public class ShieldSpecialRenderer implements SpecialModelRenderer<DataComponentMap> {
	private final MaterialSet materials;
	private final ShieldModel model;

	public ShieldSpecialRenderer(MaterialSet materialSet, ShieldModel shieldModel) {
		this.materials = materialSet;
		this.model = shieldModel;
	}

	/** Semantic GUI copier accessors; the model/material set stay Java-transient. */
	public ShieldModel model() { return this.model; }
	public net.minecraft.client.renderer.texture.TextureAtlasSprite sprite(Material material) { return this.materials.get(material); }

	@Nullable
	public DataComponentMap extractArgument(ItemStack itemStack) {
		return itemStack.immutableComponents();
	}

	public void submit(
		@Nullable DataComponentMap dataComponentMap,
		ItemDisplayContext itemDisplayContext,
		PoseStack poseStack,
		SubmitNodeCollector submitNodeCollector,
		int i,
		int j,
		boolean bl,
		int k
	) {
		BannerPatternLayers bannerPatternLayers = dataComponentMap != null
			? (BannerPatternLayers)dataComponentMap.getOrDefault(DataComponents.BANNER_PATTERNS, BannerPatternLayers.EMPTY)
			: BannerPatternLayers.EMPTY;
		DyeColor dyeColor = dataComponentMap != null ? (DyeColor)dataComponentMap.get(DataComponents.BASE_COLOR) : null;
		boolean bl2 = !bannerPatternLayers.layers().isEmpty() || dyeColor != null;
		poseStack.pushPose();
		poseStack.scale(1.0F, -1.0F, -1.0F);
		Material material = bl2 ? ModelBakery.SHIELD_BASE : ModelBakery.NO_PATTERN_SHIELD;
		submitNodeCollector.submitModelPartSemantic(
			this.model.handle(), poseStack, this.model.renderType(material.atlasLocation()), i, j, this.materials.get(material), false, false, -1, null, k
		);
		if (bl2) {
			BannerRenderer.submitPatterns(
				this.materials,
				poseStack,
				submitNodeCollector,
				i,
				j,
				this.model,
				Unit.INSTANCE,
				material,
				false,
				(DyeColor)Objects.requireNonNullElse(dyeColor, DyeColor.WHITE),
				bannerPatternLayers,
				bl,
				null,
				k
			);
		} else {
			submitNodeCollector.submitModelPartSemantic(
				this.model.plate(), poseStack, this.model.renderType(material.atlasLocation()), i, j, this.materials.get(material), false, bl, -1, null, k
			);
		}

		poseStack.popPose();
	}

	@Override
	public void getExtents(Set<Vector3f> set) {
		PoseStack poseStack = new PoseStack();
		poseStack.scale(1.0F, -1.0F, -1.0F);
		this.model.root().getExtentsForGui(poseStack, set);
	}

	@Override
	public boolean isAnimated(@Nullable DataComponentMap components) {
		BannerPatternLayers patterns = components != null
			? components.getOrDefault(DataComponents.BANNER_PATTERNS, BannerPatternLayers.EMPTY)
			: BannerPatternLayers.EMPTY;
		boolean patterned = !patterns.layers().isEmpty()
			|| components != null && components.get(DataComponents.BASE_COLOR) != null;
		if (this.materials.get(patterned ? ModelBakery.SHIELD_BASE : ModelBakery.NO_PATTERN_SHIELD).contents().isAnimated()) {
			return true;
		}
		if (patterned) {
			if (this.materials.get(net.minecraft.client.renderer.Sheets.SHIELD_BASE).contents().isAnimated()) {
				return true;
			}
			// Match the renderer's layer limit; unused pack sprites must not invalidate the icon.
			for (int index = 0; index < Math.min(16, patterns.layers().size()); index++) {
				if (this.materials.get(net.minecraft.client.renderer.Sheets.getShieldMaterial(
					patterns.layers().get(index).pattern())).contents().isAnimated()) {
					return true;
				}
			}
		}
		return false;
	}

	@Environment(EnvType.CLIENT)
	public record Unbaked() implements SpecialModelRenderer.Unbaked {
		public static final ShieldSpecialRenderer.Unbaked INSTANCE = new ShieldSpecialRenderer.Unbaked();
		public static final MapCodec<ShieldSpecialRenderer.Unbaked> MAP_CODEC = MapCodec.unit(INSTANCE);

		@Override
		public MapCodec<ShieldSpecialRenderer.Unbaked> type() {
			return MAP_CODEC;
		}

		@Override
		public SpecialModelRenderer<?> bake(SpecialModelRenderer.BakingContext bakingContext) {
			return new ShieldSpecialRenderer(bakingContext.materials(), new ShieldModel(bakingContext.entityModelSet().bakeLayer(ModelLayers.SHIELD)));
		}
	}
}
