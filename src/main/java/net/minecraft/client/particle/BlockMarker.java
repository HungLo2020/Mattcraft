package net.minecraft.client.particle;

import net.minecraft.api.EnvType;
import net.minecraft.api.Environment;
import net.minecraft.client.Camera;
import net.minecraft.client.Minecraft;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.renderer.state.QuadParticleRenderState;
import net.minecraft.core.particles.BlockParticleOption;
import net.minecraft.util.RandomSource;
import net.minecraft.world.level.block.state.BlockState;
import net.vulkanic.world.RustGalWorldPrimitiveRenderer;

@Environment(EnvType.CLIENT)
public class BlockMarker extends SingleQuadParticle {
	// Iris: Track whether particle is opaque (from MixinStationaryItemParticle)
	private boolean isOpaque;
	private final BlockState blockState;
	
	BlockMarker(ClientLevel clientLevel, double d, double e, double f, BlockState blockState) {
		super(clientLevel, d, e, f, Minecraft.getInstance().getBlockRenderer().getBlockModelShaper().getParticleIcon(blockState));
		this.blockState = blockState;
		this.gravity = 0.0F;
		this.lifetime = 80;
		if (!System.getProperty("mattmc.dev.rustGalWorldMaterial.blockMarkerScenario", "").isBlank()) {
			this.lifetime = 20_000;
		}
		this.hasPhysics = false;
		
		// Iris: Resolve translucency (from MixinStationaryItemParticle)
		net.minecraft.client.renderer.chunk.ChunkSectionLayer type = net.minecraft.client.renderer.ItemBlockRenderTypes.getChunkRenderType(blockState);
		if (type == net.minecraft.client.renderer.chunk.ChunkSectionLayer.SOLID || 
		    type == net.minecraft.client.renderer.chunk.ChunkSectionLayer.CUTOUT || 
		    type == net.minecraft.client.renderer.chunk.ChunkSectionLayer.CUTOUT_MIPPED) {
			isOpaque = true;
		}
	}

	@Override
	public SingleQuadParticle.Layer getLayer() {
		// Iris: Override particle render type for opaque particles (from MixinStationaryItemParticle)
		if (isOpaque) {
			return net.irisshaders.iris.fantastic.IrisParticleRenderTypes.TERRAIN_OPAQUE;
		}
		return SingleQuadParticle.Layer.TERRAIN;
	}

	@Override
	public void extract(QuadParticleRenderState quadParticleRenderState, Camera camera, float f) {
		if (this.enqueueRustGal(camera, f)) {
			return;
		}
		if (net.vulkanic.VulkanicAPI.isVulkanBackendSelected()
			|| net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()) {
			throw new IllegalStateException("Rust whole-frame block-marker semantics were rejected; Java particle extraction is not a fallback");
		}
		super.extract(quadParticleRenderState, camera, f);
	}

	boolean enqueueRustGal(Camera camera, float f) {
		if (net.vulkanic.world.WorldRenderRoutePolicy.currentMaterialRoute().usesRustWholeFrameVulkan()) {
			// Same immutable inputs as SingleQuadParticle.extractRotatedQuad.
			// Rust owns billboard expansion, particle shading and atlas sampling.
			// The normal Vulkan route stops at immutable particle semantics.
			org.joml.Quaternionf rotation = new org.joml.Quaternionf();
			this.getFacingCameraMode().setRotation(rotation, camera, f);
			if (this.roll != 0.0F) {
				rotation.rotateZ(net.minecraft.util.Mth.lerp(f, this.oRoll, this.roll));
			}
			boolean queued = RustGalWorldPrimitiveRenderer.enqueueTerrainParticle(
				this.blockState, this.sprite.contents().name(), this.sprite.semanticAnimationResource(),
				camera, this.xo, this.x, this.yo, this.y, this.zo, this.z, rotation, f,
				this.getQuadSize(f), this.getU0(), this.getU1(), this.getV0(), this.getV1(),
				net.minecraft.util.ARGB.colorFromFloat(this.alpha, this.rCol, this.gCol, this.bCol),
				this.getLightColor(f), this.isOpaque
					? net.vulkanic.bridge.VulkanicGalBridge.ParticleSurface.TERRAIN_OPAQUE
					: net.vulkanic.bridge.VulkanicGalBridge.ParticleSurface.TERRAIN_TRANSLUCENT);
			if (!queued) {
				throw new IllegalStateException("Native block-marker semantics rejected; expanded Java geometry is not a fallback");
			}
			return true;
		}
		return RustGalWorldPrimitiveRenderer.enqueueBlockMarker(
			this.blockState,
			camera,
			this.xo,
			this.x,
			this.yo,
			this.y,
			this.zo,
			this.z,
			f,
			this.getQuadSize(f),
			0xFFFFFFFF,
			this.sprite == null ? null : this.sprite.contents().name(),
			this.getU0(),
			this.getU1(),
			this.getV0(),
			this.getV1()
		);
	}

	@Override
	public float getQuadSize(float f) {
		return 0.5F;
	}

	@Environment(EnvType.CLIENT)
	public static class Provider implements ParticleProvider<BlockParticleOption> {
		public Particle createParticle(
			BlockParticleOption blockParticleOption, ClientLevel clientLevel, double d, double e, double f, double g, double h, double i, RandomSource randomSource
		) {
			return new BlockMarker(clientLevel, d, e, f, blockParticleOption.getState());
		}
	}
}
