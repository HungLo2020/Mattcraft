package net.minecraft.client.renderer.special;

import net.blaze3d.vertex.PoseStack;
import com.mojang.serialization.MapCodec;
import java.util.Set;
import net.minecraft.api.EnvType;
import net.minecraft.api.Environment;
import net.minecraft.client.model.geom.EntityModelSet;
import net.minecraft.client.renderer.PlayerSkinRenderCache;
import net.minecraft.client.renderer.SubmitNodeCollector;
import net.minecraft.client.resources.model.MaterialSet;
import net.minecraft.world.item.ItemDisplayContext;
import net.minecraft.world.item.ItemStack;
import org.jetbrains.annotations.Nullable;
import org.joml.Vector3f;

@Environment(EnvType.CLIENT)
public interface SpecialModelRenderer<T> {
	void submit(
		@Nullable T object, ItemDisplayContext itemDisplayContext, PoseStack poseStack, SubmitNodeCollector submitNodeCollector, int i, int j, boolean bl, int k
	);

	void getExtents(Set<Vector3f> set);

	/** Whether the model's selected materials require a fresh GUI raster each frame. */
	default boolean isAnimated(@Nullable T argument) {
		return false;
	}

	@Nullable
	T extractArgument(ItemStack itemStack);

	@Environment(EnvType.CLIENT)
	public interface BakingContext {
		EntityModelSet entityModelSet();

		MaterialSet materials();

		PlayerSkinRenderCache playerSkinRenderCache();

		@Environment(EnvType.CLIENT)
		public record Simple(EntityModelSet entityModelSet, MaterialSet materials, PlayerSkinRenderCache playerSkinRenderCache)
			implements SpecialModelRenderer.BakingContext {
		}
	}

	@Environment(EnvType.CLIENT)
	public interface Unbaked {
		@Nullable
		SpecialModelRenderer<?> bake(SpecialModelRenderer.BakingContext bakingContext);

		MapCodec<? extends SpecialModelRenderer.Unbaked> type();
	}
}
