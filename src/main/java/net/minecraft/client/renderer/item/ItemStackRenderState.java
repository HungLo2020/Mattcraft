package net.minecraft.client.renderer.item;

import net.blaze3d.vertex.PoseStack;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import java.util.function.Consumer;
import java.util.function.Supplier;
import net.minecraft.api.EnvType;
import net.minecraft.api.Environment;
import net.minecraft.client.renderer.RenderType;
import net.minecraft.client.renderer.SubmitNodeCollector;
import net.minecraft.client.renderer.block.model.BakedQuad;
import net.minecraft.client.renderer.block.model.ItemTransform;
import net.minecraft.client.renderer.special.SpecialModelRenderer;
import net.minecraft.client.renderer.texture.TextureAtlasSprite;
import net.minecraft.util.RandomSource;
import net.minecraft.world.item.ItemDisplayContext;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.AABB.Builder;
import net.sodium.client.render.frapi.mesh.MutableMeshImpl;
import net.sodium.client.render.frapi.render.AccessLayerRenderState;
import net.sodium.client.render.frapi.render.OrderedSubmitNodeCollectorExtension;
import net.sodium.client.render.frapi.render.QuadToPosPipe;
import net.vulkanic.world.ItemEntityRenderOwnershipPolicy;
import net.vulkanic.world.RustGalWorldPrimitiveRenderer;
import net.vulkanic.world.WorldRenderRoutePolicy;
import org.jetbrains.annotations.Nullable;
import org.joml.Matrix4f;
import org.joml.Vector3f;
import org.joml.Vector3fc;

@Environment(EnvType.CLIENT)
public class ItemStackRenderState implements net.irisshaders.iris.mixinterface.ItemContextState {
	ItemDisplayContext displayContext = ItemDisplayContext.NONE;
	private int activeLayerCount;
	private boolean animated;
	private boolean oversizedInGui;
	@Nullable
	private AABB cachedModelBoundingBox;
	private ItemStackRenderState.LayerRenderState[] layers = new ItemStackRenderState.LayerRenderState[]{new ItemStackRenderState.LayerRenderState()};
	// Iris: ItemContextState fields
	private net.minecraft.world.item.Item iris_displayStack;
	private net.minecraft.resources.ResourceLocation iris_displayModelId;

	public void ensureCapacity(int i) {
		int j = this.layers.length;
		int k = this.activeLayerCount + i;
		if (k > j) {
			this.layers = (ItemStackRenderState.LayerRenderState[])Arrays.copyOf(this.layers, k);

			for (int l = j; l < k; l++) {
				this.layers[l] = new ItemStackRenderState.LayerRenderState();
			}
		}
	}

	public ItemStackRenderState.LayerRenderState newLayer() {
		this.ensureCapacity(1);
		return this.layers[this.activeLayerCount++];
	}

	public void clear() {
		this.displayContext = ItemDisplayContext.NONE;

		for (int i = 0; i < this.activeLayerCount; i++) {
			this.layers[i].clear();
		}

		this.activeLayerCount = 0;
		this.animated = false;
		this.oversizedInGui = false;
		this.cachedModelBoundingBox = null;
		// Iris: Clear display stack
		this.iris_displayStack = null;
		this.iris_displayModelId = null;
	}

	public void setAnimated() {
		this.animated = true;
	}

	public boolean isAnimated() {
		return this.animated;
	}

	public boolean hasSpecialRenderer() {
		for (int index = 0; index < this.activeLayerCount; index++) {
			if (this.layers[index].specialRenderer != null) return true;
		}
		return false;
	}

	public void appendModelIdentityElement(Object object) {
	}

	private ItemStackRenderState.LayerRenderState firstLayer() {
		return this.layers[0];
	}

	public boolean isEmpty() {
		return this.activeLayerCount == 0;
	}

	/**
	 * Exposes one Java-only snapshot of the resolved item model layers. Consumers
	 * must copy the needed semantics immediately; renderer objects never cross
	 * the native boundary.
	 */
	public void forEachSemanticLayer(Consumer<SemanticLayer> consumer) {
		for (int i = 0; i < this.activeLayerCount; i++) {
			ItemStackRenderState.LayerRenderState layer = this.layers[i];
			PoseStack.Pose transformPose = new PoseStack.Pose();
			layer.transform.apply(this.displayContext.leftHand(), transformPose);
			float[] modelTransform = new float[16];
			transformPose.pose().get(modelTransform);
			consumer.accept(new SemanticLayer(
				layer.quads,
				layer.tintLayers,
				layer.renderType,
				layer.foilType,
				layer.usesBlockLight,
				layer.specialRenderer != null,
				layer.transform == ItemTransform.NO_TRANSFORM,
				modelTransform
			));
		}
	}

	/**
	 * Visits resolved special-model producers while they are still Java-side
	 * semantic inputs.  GUI routes use this only to select a bounded copier;
	 * the renderer and its argument never cross the native boundary.
	 */
	public void forEachSpecialRenderer(Consumer<SpecialRender> consumer) {
		for (int index = 0; index < this.activeLayerCount; index++) {
			LayerRenderState layer = this.layers[index];
			if (layer.specialRenderer == null) continue;
			PoseStack.Pose transformPose = new PoseStack.Pose();
			layer.transform.apply(this.displayContext.leftHand(), transformPose);
			consumer.accept(new SpecialRender(layer.specialRenderer, layer.argumentForSpecialRendering, transformPose));
		}
	}

	public ItemDisplayContext displayContext() {
		return this.displayContext;
	}

	public record SpecialRender(SpecialModelRenderer<Object> renderer, @Nullable Object argument, PoseStack.Pose transform) {
	}

	/**
	 * Dispatches resolved special-model producers as semantic callsites.  The
	 * renderer object is used only during this Java call; any geometry accepted
	 * by the active Rust route is copied by the collector immediately.
	 */
	public void submitSpecialRenderers(
		PoseStack poseStack, SubmitNodeCollector submitNodeCollector, int packedLight, int overlayCoords, int outlineColor
	) {
		for (int index = 0; index < this.activeLayerCount; index++) {
			LayerRenderState layer = this.layers[index];
			if (layer.specialRenderer == null) continue;
			poseStack.pushPose();
			layer.transform.apply(this.displayContext.leftHand(), poseStack.last());
			layer.specialRenderer.submit(
				layer.argumentForSpecialRendering,
				this.displayContext,
				poseStack,
				submitNodeCollector,
				packedLight,
				overlayCoords,
				layer.foilType != FoilType.NONE,
				outlineColor
			);
			poseStack.popPose();
		}
	}

	public boolean usesBlockLight() {
		return this.firstLayer().usesBlockLight;
	}

	@Nullable
	public TextureAtlasSprite pickParticleIcon(RandomSource randomSource) {
		return this.activeLayerCount == 0 ? null : this.layers[randomSource.nextInt(this.activeLayerCount)].particleIcon;
	}

	public void visitExtents(Consumer<Vector3fc> consumer) {
		Vector3f vector3f = new Vector3f();
		PoseStack.Pose pose = new PoseStack.Pose();
		// Sodium FRAPI: QuadToPosPipe for mesh processing (merged from ItemRenderStateMixin)
		QuadToPosPipe pipe = null;

		for (int i = 0; i < this.activeLayerCount; i++) {
			ItemStackRenderState.LayerRenderState layerRenderState = this.layers[i];
			layerRenderState.transform.apply(this.displayContext.leftHand(), pose);
			Matrix4f matrix4f = pose.pose();
			Vector3f[] vector3fs = (Vector3f[])layerRenderState.extents.get();

			for (Vector3f vector3f2 : vector3fs) {
				consumer.accept(vector3f.set(vector3f2).mulPosition(matrix4f));
			}

			// Sodium FRAPI: Process mutable mesh before resetting pose (merged from ItemRenderStateMixin)
			MutableMeshImpl mutableMesh = ((AccessLayerRenderState) layerRenderState).fabric_getMutableMesh();

			if (mutableMesh.size() > 0) {
				if (pipe == null) {
					pipe = new QuadToPosPipe(consumer, vector3f);
				}
				pipe.matrix = matrix4f;
				// Use the mutable version here as it does not use a ThreadLocal or cursor stack
				mutableMesh.forEachMutable(pipe);
			}

			pose.setIdentity();
		}
	}

	public void submit(PoseStack poseStack, SubmitNodeCollector submitNodeCollector, int i, int j, int k) {
		this.submitSemantic(poseStack, submitNodeCollector, i, j, k);
	}

	/** Explicit semantic item submission used by Rust-owned extraction callsites. */
	public void submitSemantic(PoseStack poseStack, SubmitNodeCollector submitNodeCollector, int i, int j, int k) {
		for (int l = 0; l < this.activeLayerCount; l++) {
			this.layers[l].submit(poseStack, submitNodeCollector, i, j, k);
		}
	}

	public AABB getModelBoundingBox() {
		if (this.cachedModelBoundingBox != null) {
			return this.cachedModelBoundingBox;
		} else {
			Builder builder = new Builder();
			this.visitExtents(builder::include);
			AABB aABB = builder.build();
			this.cachedModelBoundingBox = aABB;
			return aABB;
		}
	}

	public void setOversizedInGui(boolean bl) {
		this.oversizedInGui = bl;
	}

	public boolean isOversizedInGui() {
		return this.oversizedInGui;
	}

	@Environment(EnvType.CLIENT)
	public static enum FoilType {
		NONE,
		STANDARD,
		SPECIAL;
	}

	/**
	 * Java-local resolved item layer semantics. This type deliberately contains
	 * vanilla objects only so an extractor can consume them before FFI packing.
	 */
	public record SemanticLayer(
		List<BakedQuad> quads,
		int[] tintLayers,
		@Nullable RenderType renderType,
		FoilType foilType,
		boolean usesBlockLight,
		boolean hasSpecialRenderer,
		boolean identityTransform,
		float[] modelTransform
	) {
		public SemanticLayer {
			quads = List.copyOf(quads);
			tintLayers = tintLayers.clone();
			if (modelTransform.length != 16) {
				throw new IllegalArgumentException("semantic item layer transform must contain 16 floats");
			}
			for (float value : modelTransform) {
				if (!Float.isFinite(value)) {
					throw new IllegalArgumentException("semantic item layer transform must be finite");
				}
			}
			modelTransform = modelTransform.clone();
		}

		@Override
		public int[] tintLayers() {
			return this.tintLayers.clone();
		}

		@Override
		public float[] modelTransform() {
			return this.modelTransform.clone();
		}
	}

	@Environment(EnvType.CLIENT)
	public class LayerRenderState implements net.fabricmc.fabric.api.renderer.v1.render.FabricLayerRenderState, AccessLayerRenderState {
		private static final Vector3f[] NO_EXTENTS = new Vector3f[0];
		public static final Supplier<Vector3f[]> NO_EXTENTS_SUPPLIER = () -> NO_EXTENTS;
		private final List<BakedQuad> quads = new ArrayList();
		boolean usesBlockLight;
		@Nullable
		TextureAtlasSprite particleIcon;
		ItemTransform transform = ItemTransform.NO_TRANSFORM;
		@Nullable
		private RenderType renderType;
		private ItemStackRenderState.FoilType foilType = ItemStackRenderState.FoilType.NONE;
		private int[] tintLayers = new int[0];
		@Nullable
		private SpecialModelRenderer<Object> specialRenderer;
		@Nullable
		private Object argumentForSpecialRendering;
		Supplier<Vector3f[]> extents = NO_EXTENTS_SUPPLIER;
		// Fabric Rendering API support (from ItemLayerRenderStateMixin)
		private final MutableMeshImpl mutableMesh = new MutableMeshImpl();

		public void clear() {
			this.quads.clear();
			this.renderType = null;
			this.foilType = ItemStackRenderState.FoilType.NONE;
			this.specialRenderer = null;
			this.argumentForSpecialRendering = null;
			Arrays.fill(this.tintLayers, -1);
			this.usesBlockLight = false;
			this.particleIcon = null;
			this.transform = ItemTransform.NO_TRANSFORM;
			this.extents = NO_EXTENTS_SUPPLIER;
			// Clear mutable mesh (from ItemLayerRenderStateMixin)
			this.mutableMesh.clear();
		}

		public List<BakedQuad> prepareQuadList() {
			return this.quads;
		}

		public void setRenderType(RenderType renderType) {
			this.renderType = renderType;
		}

		public void setUsesBlockLight(boolean bl) {
			this.usesBlockLight = bl;
		}

		public void setExtents(Supplier<Vector3f[]> supplier) {
			this.extents = supplier;
		}

		public void setParticleIcon(TextureAtlasSprite textureAtlasSprite) {
			this.particleIcon = textureAtlasSprite;
		}

		public void setTransform(ItemTransform itemTransform) {
			this.transform = itemTransform;
		}

		public <T> void setupSpecialModel(SpecialModelRenderer<T> specialModelRenderer, @Nullable T object) {
			this.specialRenderer = eraseSpecialRenderer(specialModelRenderer);
			this.argumentForSpecialRendering = object;
		}

		private static SpecialModelRenderer<Object> eraseSpecialRenderer(SpecialModelRenderer<?> specialModelRenderer) {
			return (SpecialModelRenderer<Object>)specialModelRenderer;
		}

		public void setFoilType(ItemStackRenderState.FoilType foilType) {
			this.foilType = foilType;
		}

		public int[] prepareTintLayers(int i) {
			if (i > this.tintLayers.length) {
				this.tintLayers = new int[i];
				Arrays.fill(this.tintLayers, -1);
			}

			return this.tintLayers;
		}

		void submit(PoseStack poseStack, SubmitNodeCollector submitNodeCollector, int i, int j, int k) {
			boolean selectedVulkan = net.vulkanic.VulkanicAPI.isVulkanBackendSelected();
			boolean rustWholeFrameHandoff = net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled();
			boolean indexedItemScope = RustGalWorldPrimitiveRenderer.isIndexedItemSubmissionActive();
			// Text/coverage traversals replay the same producer callbacks, but their
			// collectors must receive semantics without a second native submission.
			if (!submitNodeCollector.isSemanticCoverageOnly()
				&& (selectedVulkan || rustWholeFrameHandoff || indexedItemScope)) {
				WorldRenderRoutePolicy.Route ownership = ItemEntityRenderOwnershipPolicy.currentOwnershipRoute();
				if (rustWholeFrameHandoff && !selectedVulkan && !indexedItemScope) {
					throw new IllegalStateException(
						"Rust whole-frame item route requires Vulkan selection; Java special-item rendering is unavailable during handoff"
					);
				}
				if (selectedVulkan && !ownership.usesRustWholeFrameVulkan()) {
					throw new IllegalStateException(
						"Rust whole-frame item route is unavailable; selected Vulkan cannot fall through to Java item rendering"
					);
				}
				if (ownership.usesRustWholeFrameVulkan()) {
					poseStack.pushPose();
					this.transform.apply(ItemStackRenderState.this.displayContext.leftHand(), poseStack.last());
					if ((Object)this.specialRenderer instanceof net.minecraft.client.renderer.special.TaczGlock17SpecialRenderer taczRenderer) {
						// TACZ owns a copied Bedrock semantic producer for third-person,
						// ground, and fixed item contexts. Dispatch it before the generic
						// special-renderer rejection; every other special renderer remains
						// explicitly unavailable on the Rust whole-frame route.
						int meshCountBefore = RustGalWorldPrimitiveRenderer.pendingSemanticItemSubmissionCount();
						taczRenderer.submit(
							ItemStackRenderState.this.displayContext,
							poseStack,
							submitNodeCollector,
							i,
							j,
							false,
							k
						);
						boolean rustQueued = RustGalWorldPrimitiveRenderer.pendingSemanticItemSubmissionCount() > meshCountBefore;
						RustGalWorldPrimitiveRenderer.recordItemEntityRouteDecision(
							rustQueued ? "rust-vulkan-whole-frame" : "rust-vulkan-unavailable", rustQueued,
							rustQueued ? null : "tacz-special-renderer-semantic-receipt", rustQueued, rustQueued, false
						);
						poseStack.popPose();
						if (!rustQueued) {
							throw new IllegalStateException("Rust whole-frame TACZ item route produced no semantic mesh");
						}
						return;
					}
					if (this.specialRenderer != null) {
						// Vanilla special item renderers are semantic producers: their
						// submit methods feed Model/ModelPart calls back into the same
						// collector, where Rust admission and asset copying still apply.
						// Measure the copied mesh receipt so an unsupported special
						// renderer remains explicitly unavailable.
						int meshCountBefore = RustGalWorldPrimitiveRenderer.pendingSemanticItemSubmissionCount();
						this.specialRenderer.submit(
							this.argumentForSpecialRendering,
							ItemStackRenderState.this.displayContext,
							poseStack,
							submitNodeCollector,
							i,
							j,
							this.foilType != FoilType.NONE,
							k
						);
						boolean rustQueued = RustGalWorldPrimitiveRenderer.pendingSemanticItemSubmissionCount() > meshCountBefore;
						RustGalWorldPrimitiveRenderer.recordItemEntityRouteDecision(
							rustQueued ? "rust-vulkan-whole-frame" : "rust-vulkan-unavailable",
							rustQueued,
							rustQueued ? null : "special-renderer-semantic-receipt",
							rustQueued,
							rustQueued,
							false
						);
						poseStack.popPose();
						if (!rustQueued) {
							throw new IllegalStateException("Rust whole-frame special-item route produced no semantic mesh");
						}
						return;
					}
					if (this.mutableMesh.size() > 0
						&& submitNodeCollector instanceof net.sodium.client.render.frapi.render.OrderedSubmitNodeCollectorExtension access) {
						// Fabric's MeshView is admitted through the explicit semantic
						// collector. The collector copies QuadView records and owns the
						// resulting Rust indexed asset; no Java FRAPI draw is allowed.
						int meshCountBefore = RustGalWorldPrimitiveRenderer.pendingSemanticItemSubmissionCount();
						access.fabric_submitItem(
							poseStack, ItemStackRenderState.this.displayContext, i, j, k,
							this.tintLayers, this.quads, this.renderType, this.foilType, this.mutableMesh
						);
						boolean accepted = RustGalWorldPrimitiveRenderer.pendingSemanticItemSubmissionCount() > meshCountBefore;
						poseStack.popPose();
						if (!accepted) {
							throw new IllegalStateException("Rust whole-frame Fabric item route rejected semantic MeshView");
						}
						RustGalWorldPrimitiveRenderer.recordItemEntityRouteDecision(
							"rust-vulkan-whole-frame", true, null, true, true, false
						);
						return;
					}

					String unavailableReason;
					if (this.specialRenderer != null) {
						unavailableReason = "special-renderer";
					} else if (this.mutableMesh.size() > 0) {
						unavailableReason = "fabric-mesh-collector-unavailable";
					} else {
						unavailableReason = RustGalWorldPrimitiveRenderer.itemEntityMeshIneligibility(
							ItemStackRenderState.this.displayContext,
							i,
							j,
							k,
							this.tintLayers,
							this.quads,
							this.renderType,
							this.foilType
						);
					}

					if (unavailableReason != null) {
						RustGalWorldPrimitiveRenderer.recordItemEntityRouteDecision(
							"rust-vulkan-unavailable", false, unavailableReason, false, false, false
						);
						poseStack.popPose();
						throw new IllegalStateException("Rust whole-frame item route has no semantic mesh: " + unavailableReason);
					}

					boolean rustQueued = RustGalWorldPrimitiveRenderer.enqueueItemEntityMesh(
						poseStack.last(),
						ItemStackRenderState.this.displayContext,
						i,
						j,
						k,
						this.tintLayers,
						this.quads,
						this.renderType,
						this.foilType
					);
					RustGalWorldPrimitiveRenderer.recordItemEntityRouteDecision(
						"rust-vulkan-whole-frame", true, null, true, rustQueued, false
					);
					poseStack.popPose();
					if (!rustQueued) {
						throw new IllegalStateException("Rust whole-frame item-entity layer was eligible but did not enqueue a copied indexed mesh request");
					}
					return;
				}
			}

			// Iris: Save block entity state before rendering (from ItemStackStateLayerMixin).
			// Rust semantic/item submission owns its material identity explicitly and
			// must not publish or read the Java/Iris captured-rendering singleton.
			boolean captureIrisRenderState = !submitNodeCollector.isSemanticCoverageOnly()
				&& !net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()
				&& !net.vulkanic.VulkanicAPI.isVulkanBackendSelected();
			int lastBState = captureIrisRenderState
				? net.irisshaders.iris.uniforms.CapturedRenderingState.INSTANCE.getCurrentRenderedBlockEntity()
				: 0;
			if (captureIrisRenderState) {
				iris$setupId(ItemStackRenderState.this.iris_displayStack, ItemStackRenderState.this.iris_displayModelId);
			}
			
			poseStack.pushPose();
			this.transform.apply(ItemStackRenderState.this.displayContext.leftHand(), poseStack.last());
			if (this.specialRenderer != null) {
				this.specialRenderer
					.submit(
						this.argumentForSpecialRendering,
						ItemStackRenderState.this.displayContext,
						poseStack,
						submitNodeCollector,
						i,
						j,
						this.foilType != ItemStackRenderState.FoilType.NONE,
						k
					);
			} else if (this.renderType != null) {
				// Fabric Rendering API support (from ItemLayerRenderStateMixin redirect)
				if (ItemStackRenderState.this.displayContext != ItemDisplayContext.GUI
					&& this.mutableMesh.size() > 0
					&& submitNodeCollector instanceof OrderedSubmitNodeCollectorExtension access) {
					// We don't have to copy the mesh here because vanilla doesn't copy the tint array or quad list either.
					access.fabric_submitItem(poseStack, ItemStackRenderState.this.displayContext, i, j, k, this.tintLayers, this.quads, this.renderType, this.foilType, this.mutableMesh);
				} else {
					submitNodeCollector.submitItemSemantic(poseStack, ItemStackRenderState.this.displayContext, i, j, k, this.tintLayers, this.quads, this.renderType, this.foilType);
				}
			}

			poseStack.popPose();
			
			// Iris: Restore state after rendering (from ItemStackStateLayerMixin)
			if (captureIrisRenderState) {
				net.irisshaders.iris.uniforms.CapturedRenderingState.INSTANCE.setCurrentBlockEntity(lastBState);
				net.irisshaders.iris.uniforms.CapturedRenderingState.INSTANCE.setCurrentRenderedItem(0);
			}
		}
		
		// Iris: Helper method from ItemStackStateLayerMixin
		private void iris$setupId(net.minecraft.world.item.Item item, net.minecraft.resources.ResourceLocation modelId) {
			if (net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()
				|| net.vulkanic.VulkanicAPI.isVulkanBackendSelected()) return;
			if (net.irisshaders.iris.shaderpack.materialmap.WorldRenderingSettings.INSTANCE.getItemIds() == null) return;

			if (item instanceof net.minecraft.world.item.BlockItem blockItem && !(item instanceof net.minecraft.world.item.SolidBucketItem)) {
				if (net.irisshaders.iris.shaderpack.materialmap.WorldRenderingSettings.INSTANCE.getBlockStateIds() == null) return;

				net.irisshaders.iris.uniforms.CapturedRenderingState.INSTANCE.setCurrentBlockEntity(1);
				net.irisshaders.iris.uniforms.CapturedRenderingState.INSTANCE.setCurrentRenderedItem(net.irisshaders.iris.shaderpack.materialmap.WorldRenderingSettings.INSTANCE.getBlockStateIds().getOrDefault(blockItem.getBlock().defaultBlockState(), 0));
			} else {
				net.minecraft.resources.ResourceLocation location = modelId != null ? modelId : net.minecraft.core.registries.BuiltInRegistries.ITEM.getKey(item);
				net.irisshaders.iris.uniforms.CapturedRenderingState.INSTANCE.setCurrentRenderedItem(net.irisshaders.iris.shaderpack.materialmap.WorldRenderingSettings.INSTANCE.getItemIds().applyAsInt(new net.irisshaders.iris.shaderpack.materialmap.NamespacedId(location.getNamespace(), location.getPath())));
			}
		}
		
		// Fabric Rendering API support (from ItemLayerRenderStateMixin)
		@Override
		public MutableMeshImpl fabric_getMutableMesh() {
			return this.mutableMesh;
		}
	}
	
	// Iris: ItemContextState implementation
	@Override
	public void setDisplayItem(net.minecraft.world.item.Item itemStack, net.minecraft.resources.ResourceLocation modelId) {
		this.iris_displayStack = itemStack;
		this.iris_displayModelId = modelId;
	}

	@Override
	public net.minecraft.world.item.Item getDisplayItem() {
		return iris_displayStack;
	}
	
	public net.minecraft.resources.ResourceLocation getDisplayItemModel() {
		return iris_displayModelId;
	}
}
