package net.minecraft.client.renderer.state;

import net.blaze3d.buffers.GpuBufferSlice;
import net.blaze3d.systems.RenderPass;
import net.blaze3d.systems.RenderSystem;
import net.blaze3d.vertex.BufferBuilder;
import net.blaze3d.vertex.ByteBufferBuilder;
import net.blaze3d.vertex.DefaultVertexFormat;
import net.blaze3d.vertex.MeshData;
import net.blaze3d.vertex.VertexConsumer;
import net.blaze3d.vertex.VertexFormat;
import java.util.Arrays;
import java.util.HashMap;
import java.util.HashSet;
import java.util.Map;
import java.util.Map.Entry;
import java.util.Set;
import net.minecraft.api.EnvType;
import net.minecraft.api.Environment;
import net.minecraft.client.particle.SingleQuadParticle;
import net.minecraft.client.renderer.SubmitNodeCollector;
import net.minecraft.client.renderer.feature.ParticleFeatureRenderer;
import net.minecraft.client.renderer.texture.TextureManager;
import net.logging.LogUtils;
import net.sodium.client.render.vertex.VertexConsumerUtils;
import net.vulkanic.VulkanicAPI;
import net.vulkanic.VulkanicCoreAPI;
import org.jetbrains.annotations.Nullable;
import org.joml.Quaternionf;
import org.joml.Vector3f;
import org.joml.Vector4f;
import org.slf4j.Logger;
import net.vulkanic.world.RustGalWorldPrimitiveRenderer;

@Environment(EnvType.CLIENT)
public class QuadParticleRenderState implements SubmitNodeCollector.ParticleGroupRenderer, ParticleGroupRenderState {
	private static final Logger LOGGER = LogUtils.getLogger();
	private static int debugParticlePrepareLogCount;
	private static int debugParticleDrawLogCount;
	private static int debugParticleSampleLogCount;
	private static final int INITIAL_PARTICLE_CAPACITY = 1024;
	/**
	 * The Rust whole-frame particle stream is a bounded semantic queue.  Keep
	 * extraction bounded as well so a pathological particle emitter cannot build
	 * an unbounded Java-side snapshot before the Rust queue gets a chance to
	 * reject it.
	 */
	private static final int MAX_RUST_SEMANTIC_PARTICLES = 65_536;
	private static final int FLOATS_PER_PARTICLE = 12;
	private static final int INTS_PER_PARTICLE = 2;
	private final Map<SingleQuadParticle.Layer, QuadParticleRenderState.Storage> particles = new HashMap();
	private int particleCount;

	public void add(
		SingleQuadParticle.Layer layer, float f, float g, float h, float i, float j, float k, float l, float m, float n, float o, float p, float q, int r, int s
	) {
		boolean rustWholeFrame = net.vulkanic.VulkanicAPI.isVulkanBackendSelected()
			|| net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled();
		if (rustWholeFrame && (layer == null || !Float.isFinite(f) || !Float.isFinite(g) || !Float.isFinite(h)
			|| !Float.isFinite(i) || !Float.isFinite(j) || !Float.isFinite(k) || !Float.isFinite(l)
			|| !Float.isFinite(m) || !Float.isFinite(n) || !Float.isFinite(o) || !Float.isFinite(p)
			|| !Float.isFinite(q)
			|| !Float.isFinite(i * i + j * j + k * k + l * l)
			|| i * i + j * j + k * k + l * l <= 1.0e-8F)) {
			throw new IllegalArgumentException("Rust whole-frame particle admission requires finite copied quad semantics");
		}
		if (rustWholeFrame && this.particleCount >= MAX_RUST_SEMANTIC_PARTICLES) {
			throw new IllegalStateException(
				"Rust whole-frame particle route exceeded bounded semantic particle capacity "
					+ MAX_RUST_SEMANTIC_PARTICLES
			);
		}
		((QuadParticleRenderState.Storage)this.particles.computeIfAbsent(layer, layerx -> new QuadParticleRenderState.Storage()))
			.add(f, g, h, i, j, k, l, m, n, o, p, q, r, s);
		this.particleCount++;
		net.minecraft.client.dev.GraphicsAuditLavaFixture.observeParticle(f, g, h, i, j, k, l, m, r);
	}

	/** Copies the extracted particle semantics into the explicit Rust world stream. */
	public int enqueueRustGal() {
		int unsupportedLayers = this.rustGalUnsupportedLayerCount();
		if (unsupportedLayers > 0) {
			RustGalWorldPrimitiveRenderer.recordUnsupportedParticleGroup();
			throw new IllegalStateException(
				"Rust whole-frame particle route encountered " + unsupportedLayers
					+ " atlas layer(s) without a bounded copied texture snapshot"
			);
		}
		if (!RustGalWorldPrimitiveRenderer.hasMaterialQuadCapacity(this.particleCount)) {
			RustGalWorldPrimitiveRenderer.recordUnsupportedParticleGroup();
			throw new IllegalStateException("Rust whole-frame particle route exceeded bounded material-quad capacity");
		}
		Set<net.minecraft.resources.ResourceLocation> atlasIdentities = new HashSet<>();
		for (SingleQuadParticle.Layer layer : this.particles.keySet()) {
			if (!atlasIdentities.add(layer.textureAtlasLocation())) continue;
			if (!RustGalWorldPrimitiveRenderer.ensureParticleAtlasAvailable(layer.textureAtlasLocation())) {
				RustGalWorldPrimitiveRenderer.recordUnsupportedParticleGroup();
				throw new IllegalStateException("Rust whole-frame particle atlas preflight rejected " + layer.textureAtlasLocation());
			}
		}
		int checkpoint = RustGalWorldPrimitiveRenderer.markMaterialQuadBatch();
		try {
			int submitted = 0;
			for (Entry<SingleQuadParticle.Layer, QuadParticleRenderState.Storage> entry : this.particles.entrySet()) {
				SingleQuadParticle.Layer layer = entry.getKey();
				QuadParticleRenderState.Storage storage = entry.getValue();
				if (net.minecraft.client.renderer.texture.TextureAtlas.LOCATION_PARTICLES.equals(layer.textureAtlasLocation())) {
					storage.forEachParticle((x, y, z, qx, qy, qz, qw, size, u0, u1, v0, v1, color, light) ->
						RustGalWorldPrimitiveRenderer.enqueueParticleQuad(layer.translucent(), x, y, z, qx, qy, qz, qw, size, u0, u1, v0, v1, color, light));
					submitted += storage.count();
				} else {
					int[] admitted = {0};
					storage.forEachParticle((x, y, z, qx, qy, qz, qw, size, u0, u1, v0, v1, color, light) ->
						admitted[0] += RustGalWorldPrimitiveRenderer.enqueueParticleQuadForAtlas(
							layer.textureAtlasLocation(), layer.translucent(), x, y, z, qx, qy, qz, qw,
							size, u0, u1, v0, v1, color, light) ? 1 : 0);
					if (admitted[0] != storage.count()) {
						// Atlas eligibility is only a preflight. A bounded snapshot can
						// still fail while it is being staged; never let the successful
						// prefix masquerade as a complete Rust particle group.
						RustGalWorldPrimitiveRenderer.recordUnsupportedParticleGroup();
						throw new IllegalStateException("Rust whole-frame particle atlas admission changed while staging");
					}
					submitted += admitted[0];
				}
			}
			return submitted;
		} catch (RuntimeException failure) {
			RustGalWorldPrimitiveRenderer.rollbackMaterialQuadBatch(checkpoint);
			throw failure;
		}
	}

	/**
	 * Counts extracted layers that the Rust particle material contract cannot
	 * consume. Block-atlas layers are handled by dedicated semantic producers;
	 * other atlas layers are admitted only when a copied semantic snapshot is
	 * available, otherwise they remain an explicit ownership gap.
	 */
	public int rustGalUnsupportedLayerCount() {
		int unsupported = 0;
		for (Entry<SingleQuadParticle.Layer, QuadParticleRenderState.Storage> entry : this.particles.entrySet()) {
			if (entry.getValue().count() > 0
				&& !net.minecraft.client.renderer.texture.TextureAtlas.LOCATION_PARTICLES.equals(entry.getKey().textureAtlasLocation())
				&& !RustGalWorldPrimitiveRenderer.canUseParticleAtlas(entry.getKey().textureAtlasLocation())) {
				unsupported++;
			}
		}
		return unsupported;
	}

	@Override
	public void clear() {
		this.particles.values().forEach(QuadParticleRenderState.Storage::clear);
		this.particleCount = 0;
	}

	@Nullable
	@Override
	public QuadParticleRenderState.PreparedBuffers prepare(ParticleFeatureRenderer.ParticleBufferCache particleBufferCache) {
		if (net.vulkanic.VulkanicAPI.isVulkanBackendSelected()
			|| net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()) {
			throw new IllegalStateException("Java particle buffer preparation is unavailable on Rust Vulkan");
		}
		int i = this.particleCount * 4;
		net.minecraft.client.renderer.LightTexture lightTexture = net.minecraft.client.Minecraft.getInstance().gameRenderer.lightTexture();
		if (this.particleCount > 0 && debugParticlePrepareLogCount < 32) {
			debugParticlePrepareLogCount++;
			StringBuilder layerSummary = new StringBuilder();
			for (Entry<SingleQuadParticle.Layer, QuadParticleRenderState.Storage> entry : this.particles.entrySet()) {
				if (layerSummary.length() > 0) {
					layerSummary.append(", ");
				}
				layerSummary
					.append(entry.getKey().pipeline())
					.append("[")
					.append(entry.getValue().count())
					.append("]");
			}
			LOGGER.info(
				"QuadParticleRenderState prepare#{} particleCount={} layers={}",
				debugParticlePrepareLogCount,
				this.particleCount,
				layerSummary
			);
		}

		Object var13;
		try (ByteBufferBuilder byteBufferBuilder = ByteBufferBuilder.exactlySized(i * DefaultVertexFormat.PARTICLE.getVertexSize())) {
			BufferBuilder bufferBuilder = new BufferBuilder(byteBufferBuilder, VertexFormat.Mode.QUADS, DefaultVertexFormat.PARTICLE);
			Map<SingleQuadParticle.Layer, QuadParticleRenderState.PreparedLayer> map = new HashMap();
			int j = 0;

			for (Entry<SingleQuadParticle.Layer, QuadParticleRenderState.Storage> entry : this.particles.entrySet()) {
				QuadParticleRenderState.Storage storage = (QuadParticleRenderState.Storage)entry.getValue();
				if (storage.count() > 0 && debugParticleSampleLogCount < 48) {
					debugParticleSampleLogCount++;
					LOGGER.info(
						"QuadParticleRenderState sample#{} layerPipeline={} {}",
						debugParticleSampleLogCount,
						entry.getKey().pipeline(),
						storage.describeFirstParticle(lightTexture)
					);
				}
				((QuadParticleRenderState.Storage)entry.getValue())
					.forEachParticle((f, g, h, ix, jx, k, l, m, n, o, p, q, r, s) -> this.renderRotatedQuad(bufferBuilder, f, g, h, ix, jx, k, l, m, n, o, p, q, r, s));
				if (storage.count() > 0) {
					map.put(
						(SingleQuadParticle.Layer)entry.getKey(), new QuadParticleRenderState.PreparedLayer(j, storage.count() * 6)
					);
				}

				j += storage.count() * 4;
			}

			MeshData meshData = bufferBuilder.build();
			if (meshData != null) {
				particleBufferCache.write(meshData.vertexBuffer());
				VulkanicAPI.getSequentialBuffer(VertexFormat.Mode.QUADS).getBuffer(meshData.drawState().indexCount());
				GpuBufferSlice gpuBufferSlice = VulkanicAPI.getDynamicUniforms()
					.writeTransform(
						VulkanicAPI.getModelViewMatrix(),
						new Vector4f(1.0F, 1.0F, 1.0F, 1.0F),
						new Vector3f(),
						VulkanicAPI.getTextureMatrix(),
						VulkanicAPI.getShaderLineWidth()
					);
				return new QuadParticleRenderState.PreparedBuffers(meshData.drawState().indexCount(), gpuBufferSlice, map);
			}

			var13 = null;
		}

		return (QuadParticleRenderState.PreparedBuffers)var13;
	}

	@Override
	public void render(
		QuadParticleRenderState.PreparedBuffers preparedBuffers,
		ParticleFeatureRenderer.ParticleBufferCache particleBufferCache,
		RenderPass renderPass,
		TextureManager textureManager,
		boolean bl
	) {
		if (net.vulkanic.VulkanicAPI.isVulkanBackendSelected()
			|| net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()) {
			throw new IllegalStateException("Java particle rendering is unavailable on Rust Vulkan");
		}
		VulkanicAPI.AutoStorageIndexBuffer autoStorageIndexBuffer = VulkanicAPI.getSequentialBuffer(VertexFormat.Mode.QUADS);
		renderPass.setVertexBuffer(0, particleBufferCache.get());
		renderPass.setIndexBuffer(autoStorageIndexBuffer.getBuffer(preparedBuffers.indexCount), autoStorageIndexBuffer.type());

		for (Entry<SingleQuadParticle.Layer, QuadParticleRenderState.PreparedLayer> entry : preparedBuffers.layers.entrySet()) {
			if (bl == ((SingleQuadParticle.Layer)entry.getKey()).translucent()) {
				var lightTextureView = net.minecraft.client.Minecraft.getInstance().gameRenderer.lightTexture().getTextureView();
				var particleTexture = textureManager.getTexture(((SingleQuadParticle.Layer)entry.getKey()).textureAtlasLocation());
				var liveParticleTexture = particleTexture.getTexture();
				particleTexture.setFilter(false, false);
				var particleTextureView = particleTexture.getTextureView();
				lightTextureView.texture().flushModeChanges2D();
				particleTextureView.texture().flushModeChanges2D();
				if (debugParticleDrawLogCount < 64) {
					debugParticleDrawLogCount++;
					LOGGER.info(
						"QuadParticleRenderState draw#{} translucentPass={} layerPipeline={} atlas={} texId={} label={} minFilter={} magFilter={} mipmaps={} indexCount={} vertexOffset={}",
						debugParticleDrawLogCount,
						bl,
						((SingleQuadParticle.Layer)entry.getKey()).pipeline(),
						((SingleQuadParticle.Layer)entry.getKey()).textureAtlasLocation(),
						VulkanicCoreAPI.textureId(liveParticleTexture),
						liveParticleTexture.getLabel(),
						liveParticleTexture.getMinFilter(),
						liveParticleTexture.getMagFilter(),
						liveParticleTexture.usesMipmaps(),
						((QuadParticleRenderState.PreparedLayer)entry.getValue()).indexCount,
						((QuadParticleRenderState.PreparedLayer)entry.getValue()).vertexOffset
					);
				}
				renderPass.setPipeline(((SingleQuadParticle.Layer)entry.getKey()).pipeline());
				VulkanicAPI.bindDefaultUniforms(renderPass);
				renderPass.setUniform("DynamicTransforms", preparedBuffers.dynamicTransforms);
				renderPass.bindSampler("Sampler2", lightTextureView);
				renderPass.bindSampler("Sampler0", particleTextureView);
				renderPass.drawIndexed(
					((QuadParticleRenderState.PreparedLayer)entry.getValue()).vertexOffset, 0, ((QuadParticleRenderState.PreparedLayer)entry.getValue()).indexCount, 1
				);
			}
		}
	}

	// Sodium: Optimized particle rendering (merged from QuadParticleRenderStateMixin)
	private static final Quaternionf TEMP_QUAT = new Quaternionf();
	private static final Vector3f TEMP_VECTOR = new Vector3f();

	protected void renderRotatedQuad(
		VertexConsumer vertexConsumer, float f, float g, float h, float i, float j, float k, float l, float m, float n, float o, float p, float q, int r, int s
	) {
		Quaternionf quaternionf = new Quaternionf(i, j, k, l);
		this.renderVertex(vertexConsumer, quaternionf, f, g, h, 1.0F, -1.0F, m, o, q, r, s);
		this.renderVertex(vertexConsumer, quaternionf, f, g, h, 1.0F, 1.0F, m, o, p, r, s);
		this.renderVertex(vertexConsumer, quaternionf, f, g, h, -1.0F, 1.0F, m, n, p, r, s);
		this.renderVertex(vertexConsumer, quaternionf, f, g, h, -1.0F, -1.0F, m, n, q, r, s);
	}

	// Sodium: Optimized vertex emission (merged from QuadParticleRenderStateMixin)
	private void sodium$emitVertices(net.sodium.api.vertex.buffer.VertexBufferWriter writer, float x, float y, float z, float size, float u0, float u1, float v0, float v1, int color, int light, Quaternionf quaternion) {
		try (org.lwjgl.system.MemoryStack stack = org.lwjgl.system.MemoryStack.stackPush()) {
			long buffer = stack.nmalloc(4 * net.sodium.api.vertex.format.common.ParticleVertex.STRIDE);
			long ptr = buffer;

			TEMP_VECTOR.set(1.0F, -1.0F, 0.0F).rotate(quaternion).mul(size).add(x, y, z);
			net.sodium.api.vertex.format.common.ParticleVertex.put(ptr, TEMP_VECTOR.x, TEMP_VECTOR.y, TEMP_VECTOR.z, u1, v1, color, light);
			ptr += net.sodium.api.vertex.format.common.ParticleVertex.STRIDE;

			TEMP_VECTOR.set(1.0F, 1.0F, 0.0F).rotate(quaternion).mul(size).add(x, y, z);
			net.sodium.api.vertex.format.common.ParticleVertex.put(ptr, TEMP_VECTOR.x, TEMP_VECTOR.y, TEMP_VECTOR.z, u1, v0, color, light);
			ptr += net.sodium.api.vertex.format.common.ParticleVertex.STRIDE;

			TEMP_VECTOR.set(-1.0F, 1.0F, 0.0F).rotate(quaternion).mul(size).add(x, y, z);
			net.sodium.api.vertex.format.common.ParticleVertex.put(ptr, TEMP_VECTOR.x, TEMP_VECTOR.y, TEMP_VECTOR.z, u0, v0, color, light);
			ptr += net.sodium.api.vertex.format.common.ParticleVertex.STRIDE;

			TEMP_VECTOR.set(-1.0F, -1.0F, 0.0F).rotate(quaternion).mul(size).add(x, y, z);
			net.sodium.api.vertex.format.common.ParticleVertex.put(ptr, TEMP_VECTOR.x, TEMP_VECTOR.y, TEMP_VECTOR.z, u0, v1, color, light);
			ptr += net.sodium.api.vertex.format.common.ParticleVertex.STRIDE;

			writer.push(stack, buffer, 4, net.sodium.api.vertex.format.common.ParticleVertex.FORMAT);
		}
	}

	private void renderVertex(
		VertexConsumer vertexConsumer, Quaternionf quaternionf, float f, float g, float h, float i, float j, float k, float l, float m, int n, int o
	) {
		Vector3f vector3f = new Vector3f(i, j, 0.0F).rotate(quaternionf).mul(k).add(f, g, h);
		vertexConsumer.addVertex(vector3f.x(), vector3f.y(), vector3f.z()).setUv(l, m).setColor(n).setLight(o);
	}

	@Override
	public void submit(SubmitNodeCollector submitNodeCollector, CameraRenderState cameraRenderState) {
		if (this.particleCount > 0) {
			if (net.vulkanic.VulkanicAPI.isVulkanBackendSelected()
				|| net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()) {
				submitNodeCollector.submitParticleGroupSemantic(this);
			} else {
				submitNodeCollector.submitParticleGroup(this);
			}
		}
	}

	@Override
	public void submitSemantic(SubmitNodeCollector submitNodeCollector, CameraRenderState cameraRenderState) {
		if (this.particleCount > 0) {
			submitNodeCollector.submitParticleGroupSemantic(this);
		}
	}

	@FunctionalInterface
	@Environment(EnvType.CLIENT)
	public interface ParticleConsumer {
		void consume(float f, float g, float h, float i, float j, float k, float l, float m, float n, float o, float p, float q, int r, int s);
	}

	@Environment(EnvType.CLIENT)
	public record PreparedBuffers(int indexCount, GpuBufferSlice dynamicTransforms, Map<SingleQuadParticle.Layer, QuadParticleRenderState.PreparedLayer> layers) {
	}

	@Environment(EnvType.CLIENT)
	public record PreparedLayer(int vertexOffset, int indexCount) {
	}

	@Environment(EnvType.CLIENT)
	static class Storage {
		private int capacity = 1024;
		private float[] floatValues = new float[12288];
		private int[] intValues = new int[2048];
		private int currentParticleIndex;

		public void add(float f, float g, float h, float i, float j, float k, float l, float m, float n, float o, float p, float q, int r, int s) {
			if (this.currentParticleIndex >= this.capacity) {
				this.grow();
			}

			int t = this.currentParticleIndex * 12;
			this.floatValues[t++] = f;
			this.floatValues[t++] = g;
			this.floatValues[t++] = h;
			this.floatValues[t++] = i;
			this.floatValues[t++] = j;
			this.floatValues[t++] = k;
			this.floatValues[t++] = l;
			this.floatValues[t++] = m;
			this.floatValues[t++] = n;
			this.floatValues[t++] = o;
			this.floatValues[t++] = p;
			this.floatValues[t] = q;
			t = this.currentParticleIndex * 2;
			this.intValues[t++] = r;
			this.intValues[t] = s;
			this.currentParticleIndex++;
		}

		public void forEachParticle(QuadParticleRenderState.ParticleConsumer particleConsumer) {
			for (int i = 0; i < this.currentParticleIndex; i++) {
				int j = i * 12;
				int k = i * 2;
				particleConsumer.consume(
					this.floatValues[j++],
					this.floatValues[j++],
					this.floatValues[j++],
					this.floatValues[j++],
					this.floatValues[j++],
					this.floatValues[j++],
					this.floatValues[j++],
					this.floatValues[j++],
					this.floatValues[j++],
					this.floatValues[j++],
					this.floatValues[j++],
					this.floatValues[j],
					this.intValues[k++],
					this.intValues[k]
				);
			}
		}

		public void clear() {
			this.currentParticleIndex = 0;
		}

		public String describeFirstParticle(net.minecraft.client.renderer.LightTexture lightTexture) {
			if (this.currentParticleIndex <= 0) {
				return "empty";
			}

			int floatIndex = 0;
			int intIndex = 0;
			float x = this.floatValues[floatIndex++];
			float y = this.floatValues[floatIndex++];
			float z = this.floatValues[floatIndex++];
			float qx = this.floatValues[floatIndex++];
			float qy = this.floatValues[floatIndex++];
			float qz = this.floatValues[floatIndex++];
			float qw = this.floatValues[floatIndex++];
			float size = this.floatValues[floatIndex++];
			float u0 = this.floatValues[floatIndex++];
			float u1 = this.floatValues[floatIndex++];
			float v0 = this.floatValues[floatIndex++];
			float v1 = this.floatValues[floatIndex];
			int color = this.intValues[intIndex++];
			int light = this.intValues[intIndex];
			return "pos=(%.3f,%.3f,%.3f) quat=(%.3f,%.3f,%.3f,%.3f) size=%.3f uv=[%.6f,%.6f]-[%.6f,%.6f] color=%s %s".formatted(
				x,
				y,
				z,
				qx,
				qy,
				qz,
				qw,
				size,
				u0,
				u1,
				v0,
				v1,
				String.format("0x%08X rgba=(%d,%d,%d,%d)", color, color & 255, color >> 8 & 255, color >> 16 & 255, color >>> 24),
				lightTexture.debugDescribePackedLight(light)
			);
		}

		private void grow() {
			this.capacity *= 2;
			this.floatValues = Arrays.copyOf(this.floatValues, this.capacity * 12);
			this.intValues = Arrays.copyOf(this.intValues, this.capacity * 2);
		}

		public int count() {
			return this.currentParticleIndex;
		}
	}
}
