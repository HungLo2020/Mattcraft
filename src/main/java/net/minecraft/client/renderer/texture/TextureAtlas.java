package net.minecraft.client.renderer.texture;

import net.blaze3d.platform.TextureUtil;
import net.blaze3d.systems.RenderSystem;
import net.blaze3d.textures.TextureFormat;
import net.logging.LogUtils;
import java.io.IOException;
import java.io.Writer;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Objects;
import java.util.Map.Entry;
import net.minecraft.api.EnvType;
import net.minecraft.api.Environment;
import net.minecraft.CrashReport;
import net.minecraft.CrashReportCategory;
import net.minecraft.hooks.HookRegistry;
import net.minecraft.hooks.TextureAtlasHooks;
import net.minecraft.ReportedException;
import net.minecraft.SharedConstants;
import net.minecraft.resources.ResourceLocation;
import net.vulkanic.VulkanicAPI;
import org.jetbrains.annotations.Nullable;
import org.slf4j.Logger;

@Environment(EnvType.CLIENT)
public class TextureAtlas extends AbstractTexture implements Dumpable, Tickable, net.irisshaders.iris.pbr.texture.TextureAtlasExtension {
	private static final Logger LOGGER = LogUtils.getLogger();
	@Deprecated
	public static final ResourceLocation LOCATION_BLOCKS = ResourceLocation.withDefaultNamespace("textures/atlas/blocks.png");
	@Deprecated
	public static final ResourceLocation LOCATION_PARTICLES = ResourceLocation.withDefaultNamespace("textures/atlas/particles.png");
	private List<SpriteContents> sprites = List.of();
	private List<TextureAtlasSprite.Ticker> animatedTextures = List.of();
	public Map<ResourceLocation, TextureAtlasSprite> texturesByName = Map.of();
	@Nullable
	private TextureAtlasSprite missingSprite;
	private final ResourceLocation location;
	private final int maxSupportedTextureSize;
	public int width;
	public int height;
	public int mipLevel;
	private long semanticSnapshotGeneration;
	private long semanticReloadGeneration;
	/** Frame selection used to build the cached CPU semantic atlas snapshot. */
	private long semanticSnapshotFrameKey = Long.MIN_VALUE;
	@Nullable
	private TextureAtlas.SemanticRawSnapshot semanticRawSnapshot;
	@Nullable
	private net.vulkanic.world.AtlasAnimationResource semanticAnimationResource;
	
	// Iris PBR: From texture.pbr.MixinTextureAtlas - PBR atlas holder
	@Nullable
	private net.irisshaders.iris.pbr.texture.PBRAtlasHolder iris$pbrHolder;

	public TextureAtlas(ResourceLocation resourceLocation) {
		this.location = resourceLocation;
		this.maxSupportedTextureSize = net.vulkanic.VulkanicAPI.getBackendMaxTextureSize();
	}

	private void createTexture(int i, int j, int k) {
		LOGGER.info("Created: {}x{}x{} {}-atlas", i, j, k, this.location);
		this.close();
		this.texture = net.vulkanic.VulkanicAPI.createTexture(this.location::toString, 7, TextureFormat.RGBA8, i, j, 1, k + 1);
		this.textureView = net.vulkanic.VulkanicAPI.createTextureView(this.texture);
		this.width = i;
		this.height = j;
		this.mipLevel = k;
	}

	private void refreshVulkanMipmaps() {
		if (net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()
			|| VulkanicAPI.isVulkanBackendSelected()) {
			// Selected Vulkan atlases are CPU-owned semantic snapshots. Never
			// re-enter Iris' legacy texture/mipmap state, even if a stale Java
			// texture survived a resource reload race.
			return;
		}
		if (this.texture == null || this.texture.getMipLevels() <= 1) {
			return;
		}
	}

	public void upload(SpriteLoader.Preparations preparations) {
		boolean rustWholeFrame = net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()
			|| net.vulkanic.VulkanicAPI.isVulkanBackendSelected();
		if (rustWholeFrame) {
			// Semantic GUI consumers retain only the stitched CPU source. Do not
			// allocate a Java texture/view merely to provide metadata that Rust
			// already receives as an explicit raw-image asset.
			this.close();
			this.width = preparations.width();
			this.height = preparations.height();
			this.mipLevel = preparations.mipLevel();
		} else {
			this.createTexture(preparations.width(), preparations.height(), preparations.mipLevel());
		}
		this.clearTextureData();
		if (!rustWholeFrame) {
			this.setFilter(false, this.mipLevel > 1);
		}
		this.texturesByName = Map.copyOf(preparations.regions());
		this.semanticSnapshotGeneration++;
		this.semanticReloadGeneration++;
		this.semanticSnapshotFrameKey = Long.MIN_VALUE;
		this.semanticRawSnapshot = null;
		this.missingSprite = (TextureAtlasSprite)this.texturesByName.get(MissingTextureAtlasSprite.getLocation());
		if (this.missingSprite == null) {
			throw new IllegalStateException("Atlas '" + this.location + "' (" + this.texturesByName.size() + " sprites) has no missing texture sprite");
		} else {
			List<SpriteContents> list = new ArrayList();
			List<TextureAtlasSprite.Ticker> list2 = new ArrayList();

			for (TextureAtlasSprite textureAtlasSprite : preparations.regions().values()) {
				list.add(textureAtlasSprite.contents());

				if (!rustWholeFrame) {
					try {
						textureAtlasSprite.uploadFirstFrame(this.texture);
					} catch (Throwable var10) {
						CrashReport crashReport = CrashReport.forThrowable(var10, "Stitching texture atlas");
						CrashReportCategory crashReportCategory = crashReport.addCategory("Texture being stitched together");
						crashReportCategory.setDetail("Atlas path", this.location);
						crashReportCategory.setDetail("Sprite", textureAtlasSprite);
						throw new ReportedException(crashReport);
					}
				}

				TextureAtlasSprite.Ticker ticker = rustWholeFrame ? null : textureAtlasSprite.createTicker();
				if (ticker != null) {
					list2.add(ticker);
				}
			}

			this.sprites = List.copyOf(list);
			this.animatedTextures = List.copyOf(list2);
			if (rustWholeFrame && (LOCATION_BLOCKS.equals(this.location)
				|| LOCATION_PARTICLES.equals(this.location)
				|| net.vulkanic.world.AtlasAnimationResource.privateShieldLifecycleEnabled()
					&& net.minecraft.client.renderer.Sheets.SHIELD_SHEET.equals(this.location))) {
				// The incarnation starts with atlas upload, before any resource lookup
				// or world publication can lose its semantic sprite-use events.
				var resource = new net.vulkanic.world.AtlasAnimationResource(this.location,
					LOCATION_BLOCKS.equals(this.location)
						? net.vulkanic.world.RustGalWorldPrimitiveRenderer.MATERIAL_TEXTURE_TERRAIN_BLOCK_ATLAS
						: LOCATION_PARTICLES.equals(this.location)
							? net.vulkanic.world.RustGalWorldPrimitiveRenderer.MATERIAL_TEXTURE_PARTICLE_ATLAS
							: net.vulkanic.world.RustGalWorldPrimitiveRenderer.shieldAtlasTextureId(),
					this.semanticAnimationSource());
				for (var declaration : resource.source().sprites()) {
					this.texturesByName.get(declaration.name()).bindSemanticAnimationResource(resource);
				}
				this.semanticAnimationResource = resource;
			}
			this.refreshVulkanMipmaps();
			if (SharedConstants.DEBUG_DUMP_TEXTURE_ATLAS) {
				Path path = TextureUtil.getDebugTexturePath();

				try {
					Files.createDirectories(path);
					this.dumpContents(this.location, path);
				} catch (IOException var9) {
					LOGGER.warn("Failed to dump atlas contents to {}", path);
				}
			}
		}
		
		if (!rustWholeFrame) {
			// Call hooks after atlas upload
			for (TextureAtlasHooks hook : HookRegistry.getTextureAtlasHooks()) {
				hook.onAtlasUpload(this, this.location, preparations);
			}

			// Iris PBR: From texture.pbr.MixinTextureAtlas - track texture after upload
			net.irisshaders.iris.pbr.TextureTracker.INSTANCE.trackTexture(net.vulkanic.VulkanicCoreAPI.textureId(texture), this);
		}
	}

	@Override
	public void dumpContents(ResourceLocation resourceLocation, Path path) throws IOException {
		String string = resourceLocation.toDebugFileName();
		if (net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()
			|| net.vulkanic.VulkanicAPI.isVulkanBackendSelected()) {
			// Rust-owned atlases have no Java GPU image to read back. Preserve the
			// useful CPU diagnostic (sprite placement) without reopening a Java view.
			dumpSpriteNames(path, string, this.texturesByName);
			return;
		}
		TextureUtil.writeAsPNG(path, string, this.getTexture(), this.mipLevel, i -> i);
		dumpSpriteNames(path, string, this.texturesByName);
	}

	private static void dumpSpriteNames(Path path, String string, Map<ResourceLocation, TextureAtlasSprite> map) {
		Path path2 = path.resolve(string + ".txt");

		try {
			Writer writer = Files.newBufferedWriter(path2);

			try {
				for (Entry<ResourceLocation, TextureAtlasSprite> entry : map.entrySet().stream().sorted(Entry.comparingByKey()).toList()) {
					TextureAtlasSprite textureAtlasSprite = (TextureAtlasSprite)entry.getValue();
					writer.write(
						String.format(
							Locale.ROOT,
							"%s\tx=%d\ty=%d\tw=%d\th=%d%n",
							entry.getKey(),
							textureAtlasSprite.getX(),
							textureAtlasSprite.getY(),
							textureAtlasSprite.contents().width(),
							textureAtlasSprite.contents().height()
						)
					);
				}
			} catch (Throwable var9) {
				if (writer != null) {
					try {
						writer.close();
					} catch (Throwable var8) {
						var9.addSuppressed(var8);
					}
				}

				throw var9;
			}

			if (writer != null) {
				writer.close();
			}
		} catch (IOException var10) {
			LOGGER.warn("Failed to write file {}", path2, var10);
		}
	}

	public void cycleAnimationFrames() {
		if (net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()
			|| net.vulkanic.VulkanicAPI.isVulkanBackendSelected()) {
			// Vulkan animation clocks belong to Rust. Even a stale OpenGL ticker
			// must not select Java frames after backend selection changes. Live
			// block-atlas events progress independently of which draw consumes them.
			if (this.semanticAnimationResource != null
				&& this.semanticAnimationResource.tickDeliveryEnabled()) {
				this.semanticAnimationResource.enqueueNextTick(
					net.sodium.client.SodiumClientMod.options().performance.animateOnlyVisibleTextures);
			}
			return;
		}
		if (this.texture != null) {
			for (TextureAtlasSprite.Ticker ticker : this.animatedTextures) {
				ticker.tickAndUpload(this.texture);
			}
			if (!this.animatedTextures.isEmpty()) {
				this.refreshVulkanMipmaps();
			}
		}
		// Iris PBR: From texture.pbr.MixinTextureAtlas - cycle PBR animation frames
		if (iris$pbrHolder != null) {
			iris$pbrHolder.cycleAnimationFrames();
		}
	}

	@Override
	public void tick() {
		this.cycleAnimationFrames();
	}

	public TextureAtlasSprite getSprite(ResourceLocation resourceLocation) {
		TextureAtlasSprite textureAtlasSprite = (TextureAtlasSprite)this.texturesByName.getOrDefault(resourceLocation, this.missingSprite);
		if (textureAtlasSprite == null) {
			throw new IllegalStateException("Tried to lookup sprite, but atlas is not initialized");
		} else {
			// Call hooks after sprite is retrieved
			for (TextureAtlasHooks hook : HookRegistry.getTextureAtlasHooks()) {
				hook.onSpriteRetrieved(resourceLocation, textureAtlasSprite);
			}
			
			return textureAtlasSprite;
		}
	}

	public TextureAtlasSprite missingSprite() {
		return (TextureAtlasSprite)Objects.requireNonNull(this.missingSprite, "Atlas not initialized");
	}

	public void clearTextureData() {
		if (this.semanticAnimationResource != null) {
			this.semanticAnimationResource.close();
			this.semanticAnimationResource = null;
		}
		this.sprites.forEach(SpriteContents::close);
		this.animatedTextures.forEach(TextureAtlasSprite.Ticker::close);
		this.sprites = List.of();
		this.animatedTextures = List.of();
		this.texturesByName = Map.of();
		this.missingSprite = null;
		this.semanticSnapshotGeneration++;
		this.semanticReloadGeneration++;
		this.semanticRawSnapshot = null;
	}

	/** Resource declarations only; the Vulkan incarnation retains its immutable copy. */
	public synchronized SemanticAtlasAnimationSource semanticAnimationSource() {
		if (this.semanticAnimationResource != null) return this.semanticAnimationResource.source();
		return SemanticAtlasAnimationSource.copy(this.semanticSnapshotGeneration,
			this.width, this.height, this.mipLevel + 1, this.texturesByName);
	}

	@Nullable
	public synchronized net.vulkanic.world.AtlasAnimationResource semanticAnimationResource() {
		return this.semanticAnimationResource;
	}

	/**
	 * Bounded CPU copy of the exact first-frame atlas composition. This is a
	 * semantic resource-pack snapshot for Rust whole-frame consumers; it does
	 * not read or expose the Java GPU texture.
	 */
	@Nullable
	public synchronized TextureAtlas.SemanticRawSnapshot semanticRawSnapshot() {
		if (this.width <= 0 || this.height <= 0 || this.texturesByName.isEmpty()
			|| (long)this.width * this.height > 16L * 1024L * 1024L) {
			return null;
		}
		// The reload generation is not sufficient for animated sprites: the
		// semantic CPU snapshot must track the currently selected frame even when
		// the Java GPU atlas is not involved in the Rust route.  The atlas contents
		// map is immutable between reloads, so an ordered bounded hash is enough to
		// distinguish frame selections without retaining Java texture state.
		long frameKey = semanticFrameKey();
		if (this.semanticRawSnapshot != null
			&& this.semanticRawSnapshot.generation() == this.semanticSnapshotGeneration
			&& this.semanticSnapshotFrameKey == frameKey) {
			return this.semanticRawSnapshot;
		}
		// Frozen samples the per-sprite CPU mip chain built by SpriteContents,
		// then uploads each level into the matching atlas level.  A whole-atlas
		// mip generator would blur unrelated neighbouring sprites together. Copy
		// the same immutable CPU mip pixels for Rust-owned sampling; this neither
		// reads nor retains a Java GPU texture.
		int mipCount = Math.max(1, this.mipLevel + 1);
		List<byte[]> mipPixels = new ArrayList<>(mipCount);
		for (int mip = 0; mip < mipCount; mip++) {
			int mipWidth = Math.max(1, this.width >> mip);
			int mipHeight = Math.max(1, this.height >> mip);
			byte[] pixels;
			try {
				pixels = new byte[Math.multiplyExact(Math.multiplyExact(mipWidth, mipHeight), 4)];
			} catch (ArithmeticException error) {
				return null;
			}
			for (TextureAtlasSprite sprite : this.texturesByName.values()) {
				SpriteContents contents = sprite.contents();
				if (mip >= contents.byMipLevel.length || contents.byMipLevel[mip] == null) {
					return null;
				}
				var sourceImage = contents.byMipLevel[mip];
				int spriteWidth = Math.max(1, contents.width() >> mip);
				int spriteHeight = Math.max(1, contents.height() >> mip);
				int sourceX = 0;
				int sourceY = 0;
				if (contents.animatedTexture != null) {
					if (contents.animatedTexture.frames.isEmpty()) {
						return null;
					}
					int frame = contents.semanticFrameIndex();
					sourceX = contents.animatedTexture.getFrameX(frame) * spriteWidth;
					sourceY = contents.animatedTexture.getFrameY(frame) * spriteHeight;
				}
				int targetX = sprite.getX() >> mip;
				int targetY = sprite.getY() >> mip;
				if (targetX < 0 || targetY < 0 || targetX + spriteWidth > mipWidth
					|| targetY + spriteHeight > mipHeight
					|| sourceX + spriteWidth > sourceImage.getWidth()
					|| sourceY + spriteHeight > sourceImage.getHeight()) {
					return null;
				}
				for (int y = 0; y < spriteHeight; y++) {
					for (int x = 0; x < spriteWidth; x++) {
						int argb = sourceImage.getPixel(sourceX + x, sourceY + y);
						int offset = ((targetY + y) * mipWidth + targetX + x) * 4;
						pixels[offset] = (byte)net.minecraft.util.ARGB.red(argb);
						pixels[offset + 1] = (byte)net.minecraft.util.ARGB.green(argb);
						pixels[offset + 2] = (byte)net.minecraft.util.ARGB.blue(argb);
						pixels[offset + 3] = (byte)net.minecraft.util.ARGB.alpha(argb);
					}
				}
			}
			mipPixels.add(pixels);
		}
		this.semanticRawSnapshot = new TextureAtlas.SemanticRawSnapshot(
			this.location, this.semanticSnapshotGeneration, this.width, this.height, mipPixels.getFirst(), mipPixels
		);
		this.semanticSnapshotFrameKey = frameKey;
		return this.semanticRawSnapshot;
	}

	/** Returns the reload generation without allocating a CPU pixel snapshot. */
	public synchronized long semanticSnapshotGeneration() {
		return this.semanticSnapshotGeneration;
	}

	/** Returns the resource-reload generation, excluding animation-frame ticks. */
	public synchronized long semanticReloadGeneration() {
		return this.semanticReloadGeneration;
	}

	/** Returns the currently selected animation-frame key without allocating pixels. */
	public synchronized long semanticSnapshotFrameKey() {
		return semanticFrameKey();
	}

	private long semanticFrameKey() {
		long frameKey = 0xcbf29ce484222325L;
		for (TextureAtlasSprite sprite : this.texturesByName.values()) {
			frameKey ^= sprite.contents().semanticFrameIndex() & 0xffffffffL;
			frameKey *= 0x100000001b3L;
		}
		return frameKey;
	}

	public record SemanticRawSnapshot(ResourceLocation atlasLocation, long generation, int width, int height, byte[] pixels, List<byte[]> mipPixels) {
		public SemanticRawSnapshot {
			pixels = pixels.clone();
			mipPixels = mipPixels.stream().map(byte[]::clone).toList();
		}

		@Override
		public byte[] pixels() {
			return this.pixels.clone();
		}

		@Override
		public List<byte[]> mipPixels() {
			return this.mipPixels.stream().map(byte[]::clone).toList();
		}
	}

	public ResourceLocation location() {
		return this.location;
	}

	public int maxSupportedTextureSize() {
		return this.maxSupportedTextureSize;
	}

	public int getWidth() {
		return this.width;
	}

	public int getHeight() {
		return this.height;
	}
	
	// Iris PBR: From texture.pbr.MixinTextureAtlas - PBR holder interface implementation
	@Override
	@Nullable
	public net.irisshaders.iris.pbr.texture.PBRAtlasHolder getPBRHolder() {
		return iris$pbrHolder;
	}
	
	@Override
	public net.irisshaders.iris.pbr.texture.PBRAtlasHolder getOrCreatePBRHolder() {
		if (net.vulkanic.VulkanicAPI.isVulkanBackendSelected()
			|| net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()) {
			throw new IllegalStateException("Java Iris PBR atlas state is unavailable on the Rust Vulkan route");
		}
		if (iris$pbrHolder == null) {
			iris$pbrHolder = new net.irisshaders.iris.pbr.texture.PBRAtlasHolder();
		}
		return iris$pbrHolder;
	}
}
