package net.minecraft.client.renderer.item;

import com.mojang.serialization.MapCodec;
import com.mojang.serialization.codecs.RecordCodecBuilder;
import java.util.HashSet;
import java.util.Set;
import net.minecraft.api.EnvType;
import net.minecraft.api.Environment;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.renderer.block.model.TextureSlots;
import net.minecraft.client.renderer.special.SpecialModelRenderer;
import net.minecraft.client.renderer.special.SpecialModelRenderers;
import net.minecraft.client.resources.model.ModelBaker;
import net.minecraft.client.resources.model.ResolvableModel;
import net.minecraft.client.resources.model.ResolvedModel;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.world.entity.ItemOwner;
import net.minecraft.world.item.ItemDisplayContext;
import net.minecraft.world.item.ItemStack;
import org.jetbrains.annotations.Nullable;
import org.joml.Vector3f;

@Environment(EnvType.CLIENT)
public class SpecialModelWrapper<T> implements ItemModel {
	private final SpecialModelRenderer<T> specialRenderer;
	private final ModelRenderProperties properties;

	public SpecialModelWrapper(SpecialModelRenderer<T> specialModelRenderer, ModelRenderProperties modelRenderProperties) {
		this.specialRenderer = specialModelRenderer;
		this.properties = modelRenderProperties;
	}

	@Override
	public void update(
		ItemStackRenderState itemStackRenderState,
		ItemStack itemStack,
		ItemModelResolver itemModelResolver,
		ItemDisplayContext itemDisplayContext,
		@Nullable ClientLevel clientLevel,
		@Nullable ItemOwner itemOwner,
		int i
	) {
		itemStackRenderState.appendModelIdentityElement(this);
		ItemStackRenderState.LayerRenderState layerRenderState = itemStackRenderState.newLayer();
		if (itemStack.hasFoil()) {
			ItemStackRenderState.FoilType foilType = ItemStackRenderState.FoilType.STANDARD;
			layerRenderState.setFoilType(foilType);
			itemStackRenderState.setAnimated();
			itemStackRenderState.appendModelIdentityElement(foilType);
		}

		T object = this.specialRenderer.extractArgument(itemStack);
		if (this.specialRenderer.isAnimated(object)) {
			itemStackRenderState.setAnimated();
		}
		layerRenderState.setExtents(() -> {
			Set<Vector3f> set = new HashSet();
			this.specialRenderer.getExtents(set);
			return (Vector3f[])set.toArray(new Vector3f[0]);
		});
		layerRenderState.setupSpecialModel(this.specialRenderer, object);
		if (object != null) {
			itemStackRenderState.appendModelIdentityElement(object);
		}

		this.properties.applyToLayer(layerRenderState, itemDisplayContext);
	}

	@Environment(EnvType.CLIENT)
	public record Unbaked(ResourceLocation base, SpecialModelRenderer.Unbaked specialModel) implements ItemModel.Unbaked {
		public static final MapCodec<SpecialModelWrapper.Unbaked> MAP_CODEC = RecordCodecBuilder.mapCodec(
			instance -> instance.group(
					ResourceLocation.CODEC.fieldOf("base").forGetter(SpecialModelWrapper.Unbaked::base),
					SpecialModelRenderers.CODEC.fieldOf("model").forGetter(SpecialModelWrapper.Unbaked::specialModel)
				)
				.apply(instance, SpecialModelWrapper.Unbaked::new)
		);

		@Override
		public void resolveDependencies(ResolvableModel.Resolver resolver) {
			resolver.markDependency(this.base);
		}

		@Override
		public ItemModel bake(ItemModel.BakingContext bakingContext) {
			SpecialModelRenderer<?> specialModelRenderer = this.specialModel.bake(bakingContext);
			if (specialModelRenderer == null) {
				return bakingContext.missingItemModel();
			} else {
				ModelRenderProperties modelRenderProperties = this.getProperties(bakingContext);
				return new SpecialModelWrapper<>(specialModelRenderer, modelRenderProperties);
			}
		}

		private ModelRenderProperties getProperties(ItemModel.BakingContext bakingContext) {
			ModelBaker modelBaker = bakingContext.blockModelBaker();
			ResolvedModel resolvedModel = modelBaker.getModel(this.base);
			TextureSlots textureSlots = resolvedModel.getTopTextureSlots();
			return ModelRenderProperties.fromResolvedModel(modelBaker, resolvedModel, textureSlots);
		}

		@Override
		public MapCodec<SpecialModelWrapper.Unbaked> type() {
			return MAP_CODEC;
		}
	}
}
