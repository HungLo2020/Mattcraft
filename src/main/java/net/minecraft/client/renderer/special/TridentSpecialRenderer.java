package net.minecraft.client.renderer.special;

import net.blaze3d.vertex.PoseStack;
import com.mojang.serialization.MapCodec;
import java.util.Set;
import net.minecraft.api.EnvType;
import net.minecraft.api.Environment;
import net.minecraft.client.model.TridentModel;
import net.minecraft.client.model.geom.ModelLayers;
import net.minecraft.client.renderer.SubmitNodeCollector;
import net.minecraft.world.item.ItemDisplayContext;
import org.joml.Vector3f;

@Environment(EnvType.CLIENT)
public class TridentSpecialRenderer implements NoDataSpecialModelRenderer {
	private final TridentModel model;

	public TridentSpecialRenderer(TridentModel tridentModel) {
		this.model = tridentModel;
	}

	/** Semantic GUI copier access; the model remains Java-transient. */
	public TridentModel model() {
		return this.model;
	}

	@Override
	public void submit(ItemDisplayContext itemDisplayContext, PoseStack poseStack, SubmitNodeCollector submitNodeCollector, int i, int j, boolean bl, int k) {
		poseStack.pushPose();
		poseStack.scale(1.0F, -1.0F, -1.0F);
		if (submitNodeCollector.isSemanticCoverageOnly()) {
			// Coverage/text replay observes the original submission, never enqueues
			// another native foil draw or captures Iris rendering state.
			submitNodeCollector.submitModelPartSemantic(this.model.root(), poseStack,
				this.model.renderType(TridentModel.TEXTURE), i, j, null, false, bl, -1, null, k);
		} else if (!bl) {
			// The special-item path has no atlas sprite, but the copied TridentModel
			// already carries a complete direct texture identity. Keep non-foil
			// tridents on the shared semantic model route.
			submitNodeCollector.submitModelSemanticTexture(
				this.model,
				net.minecraft.util.Unit.INSTANCE,
				poseStack,
				this.model.renderType(TridentModel.TEXTURE),
				i,
				j,
				-1,
				TridentModel.TEXTURE,
				k,
				null
			);
		} else if ((net.vulkanic.VulkanicAPI.isVulkanBackendSelected()
				|| net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled())
			&& net.vulkanic.world.WorldRenderRoutePolicy.currentModelMeshRoute(true).usesRustWholeFrameVulkan()) {
			// The base trident remains an ordinary copied direct-texture model; the
			// foil overlay is a second explicit Rust glint mesh. Keeping both
			// submissions semantic avoids the sprite-less ModelPart fallback.
			submitNodeCollector.submitModelSemanticTexture(
				this.model, net.minecraft.util.Unit.INSTANCE, poseStack,
				this.model.renderType(TridentModel.TEXTURE), i, j, -1,
				TridentModel.TEXTURE, k, null
			);
			if (!net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueStandaloneGlintModelMesh(
				this.model, net.minecraft.util.Unit.INSTANCE, poseStack.last(),
				this.model.renderType(TridentModel.TEXTURE), TridentModel.TEXTURE, i, j
			)) {
				throw new IllegalStateException("Rust whole-frame trident foil route selected without a copied glint mesh");
			}
		} else {
			submitNodeCollector.submitModelPartSemantic(this.model.root(), poseStack, this.model.renderType(TridentModel.TEXTURE), i, j, null, false, true, -1, null, k);
		}
		poseStack.popPose();
	}

	@Override
	public void getExtents(Set<Vector3f> set) {
		PoseStack poseStack = new PoseStack();
		poseStack.scale(1.0F, -1.0F, -1.0F);
		this.model.root().getExtentsForGui(poseStack, set);
	}

	@Environment(EnvType.CLIENT)
	public record Unbaked() implements SpecialModelRenderer.Unbaked {
		public static final MapCodec<TridentSpecialRenderer.Unbaked> MAP_CODEC = MapCodec.unit(new TridentSpecialRenderer.Unbaked());

		@Override
		public MapCodec<TridentSpecialRenderer.Unbaked> type() {
			return MAP_CODEC;
		}

		@Override
		public SpecialModelRenderer<?> bake(SpecialModelRenderer.BakingContext bakingContext) {
			return new TridentSpecialRenderer(new TridentModel(bakingContext.entityModelSet().bakeLayer(ModelLayers.TRIDENT)));
		}
	}
}
