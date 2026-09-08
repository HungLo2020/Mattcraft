package net.vulkanic.shaderpack;

import net.vulkanic.bridge.VulkanicGalBridge;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.server.packs.resources.Resource;
import net.minecraft.server.packs.resources.ResourceManager;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.properties.Property;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.FileSystem;
import java.nio.file.FileSystems;
import java.nio.file.Path;
import java.util.Comparator;
import java.util.List;
import java.util.Map;
import java.util.Objects;
import java.util.Optional;
import java.util.Properties;
import java.util.TreeMap;
import java.util.stream.Stream;

/** Bounded filesystem extraction of semantic shader-pack source files. */
public final class RustShaderPackSourceCollector {
	public static final int MAX_FILES = 4096;
	public static final int MAX_FILE_BYTES = 4 * 1024 * 1024;
	public static final long MAX_TOTAL_BYTES = 64L * 1024L * 1024L;
	public static final int MAX_ASSET_FILES = 4096;
	public static final int MAX_ASSET_FILE_BYTES = 32 * 1024 * 1024;
	public static final long MAX_ASSET_TOTAL_BYTES = 256L * 1024L * 1024L;
	private static final int MAX_RUNTIME_PROPERTIES = 4096;
	/** Reserved Rust-owned semantic source-config path. */
	public static final String RUNTIME_OPTIONS_PATH = "mattmc/runtime-options.properties";
	/** Reserved Rust-owned typed source-constant configuration path. */
	public static final String RUNTIME_CONSTANTS_PATH = "mattmc/runtime-constants.properties";
	/** Reserved Rust-owned semantic source-environment path. */
	public static final String RUNTIME_ENVIRONMENT_PATH = "mattmc/runtime-environment.properties";
	/** Reserved Rust-owned table of canonical Minecraft block-state identities. */
	public static final String RUNTIME_BLOCK_STATE_IDENTITIES_PATH = "mattmc/runtime-block-states.properties";

	private RustShaderPackSourceCollector() {
	}

	public static SourceGeneration collectConfiguredPack(long generation) throws IOException {
		// Backend selection is an ownership decision that precedes activation of
		// the whole-frame shell.  A selected Vulkan backend must therefore use the
		// copied, filesystem-only collector during that startup window as well;
		// falling through to the Iris compatibility collector would borrow Iris
		// runtime state before Rust has begun presenting.
		if (net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()
			|| net.vulkanic.VulkanicAPI.isVulkanBackendSelected()) {
			// Vanilla/resource-pack post effects are semantic game resources, not
			// Iris shader-pack execution. In particular Fabulous owns
			// minecraft:transparency even when Iris shaders are disabled, so do
			// not let Iris's preference suppress the copied Rust snapshot.
			Optional<String> postEffect = activeVanillaPostEffectId();
			if (!wholeFrameShaderConfigEnabled() && postEffect.isEmpty()) {
				return disabled(generation);
			}
			Optional<String> configuredName = configuredPackNameFromDisk();
			if (configuredName.isEmpty()) {
				return postEffect.isPresent()
					? collectVanillaPostEffect(postEffect.get(), generation)
					: disabled(generation);
			}
			Path shaderpacks = net.minecraft.client.Minecraft.getInstance().gameDirectory.toPath()
				.resolve("shaderpacks").toAbsolutePath().normalize();
			String packName = configuredName.get();
			Path pack = shaderpacks.resolve(packName).normalize();
			if (!pack.startsWith(shaderpacks) || !pack.getFileName().toString().equals(packName)) {
				throw new IOException("configured shader-pack path escapes shaderpacks directory");
			}
			SourceGeneration source;
			if (Files.isDirectory(pack)) {
				source = collectWithAssets(pack.resolve("shaders"), packName, generation);
			} else {
				try (FileSystem archive = FileSystems.newFileSystem(pack)) {
					source = collectWithAssets(archive.getPath("/shaders"), packName, generation);
				}
			}
			// Rust owns option/profile interpretation. Only copied source, assets,
			// and immutable Minecraft block-state identities cross the boundary.
			return withRuntimeEnvironmentSnapshot(
				withRuntimeOptionSnapshot(
					withRuntimeBlockStateIdentitySnapshot(withActiveVanillaPostEffectResources(source)),
					readWholeFramePackOptions(shaderpacks, packName)
				),
				wholeFrameEnvironmentDefines()
			);
		}
		return IrisShaderPackCompatibilityCollector.collectConfiguredPack(generation);
	}

	/**
	 * Returns the configured pack name only after Iris has completed its own
	 * pack activation. This exposes configuration readiness only: callers use
	 * it to defer one bounded semantic source collection until startup can
	 * produce a complete immutable snapshot.
	 */
	public static Optional<String> activeConfiguredPackName() {
		if (net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()
			|| net.vulkanic.VulkanicAPI.isVulkanBackendSelected()) {
			try {
				Optional<String> postEffect = activeVanillaPostEffectId();
				if (!wholeFrameShaderConfigEnabled() && postEffect.isEmpty()) {
					return Optional.empty();
				}
				Optional<String> configured = configuredPackNameFromDisk();
				if (configured.isEmpty()) {
					return postEffect;
				}
				Path shaderpacks = net.minecraft.client.Minecraft.getInstance().gameDirectory.toPath()
					.resolve("shaderpacks").toAbsolutePath().normalize();
				Path pack = shaderpacks.resolve(configured.get()).normalize();
				if (!pack.startsWith(shaderpacks) || !Files.exists(pack)) {
					return Optional.empty();
				}
				return Optional.of(sourceGenerationKey(configured.get(), activeVanillaPostEffectId()));
			} catch (IOException error) {
				return Optional.empty();
			}
		}
		return IrisShaderPackCompatibilityCollector.activeConfiguredPackName();
	}

	static String sourceGenerationKey(String packName, Optional<String> postEffect) {
		return postEffect.map(identity -> packName + "#post-effect:" + identity).orElse(packName);
	}

	/**
	 * Returns the active identity whose copied resources feed the generic Rust
	 * fullscreen executor. Execution admission remains Rust-owned, including
	 * any copied graph that is still private pending validation.
	 */
	private static Optional<String> activeVanillaPostEffectId() {
		try {
			var minecraft = net.minecraft.client.Minecraft.getInstance();
			ResourceLocation effect = minecraft.gameRenderer.currentPostEffect();
			// Fabulous transparency is not installed as GameRenderer's ordinary
			// PostChain. It is nevertheless an active vanilla post-effect with a
			// resource-pack-overridable definition, so publish only its immutable
			// semantic identity for Rust-owned collection and execution.
			if (effect == null && net.minecraft.client.Minecraft.useShaderTransparency()) {
				effect = ResourceLocation.withDefaultNamespace("transparency");
			}
			if (effect == null) {
				return Optional.empty();
			}
			String identity = effect.toString();
			return Optional.of(identity);
		} catch (RuntimeException ignored) {
			return Optional.empty();
		}
	}

	/**
	 * Copies one active vanilla/resource-pack post-effect definition and the
	 * bounded shader source tree into the same immutable semantic snapshot used
	 * by Iris packs. No Resource, ResourceManager, or Java renderer object crosses
	 * the bridge; Rust receives only copied bytes and normalized relative paths.
	 */
	private static SourceGeneration collectVanillaPostEffect(String identity, long generation) throws IOException {
		ResourceManager manager = net.minecraft.client.Minecraft.getInstance().getResourceManager();
		ResourceLocation effect = ResourceLocation.parse(identity);
		String effectPath = effect.getPath() + ".json";
		Resource effectResource = manager.getResource(
			ResourceLocation.fromNamespaceAndPath(effect.getNamespace(), "post_effect/" + effectPath)
		).orElseThrow(() -> new IOException("active post-effect definition is missing: " + identity));
		byte[] definition = readBoundedResource(effectResource, MAX_ASSET_FILE_BYTES, "post-effect definition");
		List<VulkanicGalBridge.ShaderPackSourceFileRecord> sources = new java.util.ArrayList<>();
		byte[] format = "namespace-qualified=1\n".getBytes(StandardCharsets.UTF_8);
		sources.add(new VulkanicGalBridge.ShaderPackSourceFileRecord("mattmc/vanilla-shader-snapshot.properties", format));
		long sourceBytes = format.length;
		for (var entry : manager.listResources("shaders", location -> {
			String path = location.getPath();
			return path.endsWith(".vsh") || path.endsWith(".fsh") || path.endsWith(".glsl");
		}).entrySet().stream().sorted(Map.Entry.comparingByKey()).toList()) {
			String path = vanillaShaderSourcePath(entry.getKey());
			if (sources.size() >= MAX_FILES) {
				throw new IOException("resource-pack shader source count exceeds " + MAX_FILES);
			}
			byte[] contents = readBoundedResource(entry.getValue(), MAX_FILE_BYTES, "post-effect shader source");
			sourceBytes = Math.addExact(sourceBytes, contents.length);
			if (sourceBytes > MAX_TOTAL_BYTES) {
				throw new IOException("resource-pack post-effect source payload exceeds " + MAX_TOTAL_BYTES + " bytes");
			}
			sources.add(new VulkanicGalBridge.ShaderPackSourceFileRecord(path, contents));
		}
		List<VulkanicGalBridge.ShaderPackAssetFileRecord> assets = new java.util.ArrayList<>();
		assets.add(new VulkanicGalBridge.ShaderPackAssetFileRecord("post_effect/" + effectPath, definition));
		assets.addAll(collectPostEffectTextureAssets(manager, effect, definition));
		long assetBytes = assets.stream().mapToLong(asset -> asset.contents().length).sum();
		if (assetBytes > MAX_ASSET_TOTAL_BYTES) {
			throw new IOException("resource-pack post-effect asset payload exceeds " + MAX_ASSET_TOTAL_BYTES + " bytes");
		}
		return new SourceGeneration(
			"minecraft-resource-pack:" + identity,
			generation,
			List.copyOf(sources),
			sourceBytes,
			List.copyOf(assets),
			assetBytes
		);
	}

	static String vanillaShaderSourcePath(ResourceLocation location) {
		if (!location.getPath().startsWith("shaders/") || location.getNamespace().equals(".") || location.getNamespace().equals("..")) {
			throw new IllegalArgumentException("invalid copied shader resource identity: " + location);
		}
		return "assets/" + location.getNamespace() + "/" + location.getPath();
	}

	/** Copies only explicit TextureInput locations from the active graph. */
	private static List<VulkanicGalBridge.ShaderPackAssetFileRecord> collectPostEffectTextureAssets(
		ResourceManager manager, ResourceLocation effect, byte[] definition) throws IOException {
		JsonElement root;
		try {
			root = JsonParser.parseString(new String(definition, StandardCharsets.UTF_8));
		} catch (RuntimeException error) {
			throw new IOException("post-effect texture inputs are malformed JSON", error);
		}
		if (!root.isJsonObject() || !root.getAsJsonObject().has("passes")) {
			return List.of();
		}
		TreeMap<String, VulkanicGalBridge.ShaderPackAssetFileRecord> assets = new TreeMap<>();
		TreeMap<String, ResourceLocation> assetOwners = new TreeMap<>();
		for (JsonElement passElement : root.getAsJsonObject().getAsJsonArray("passes")) {
			if (!passElement.isJsonObject()) continue;
			JsonElement inputs = passElement.getAsJsonObject().get("inputs");
			if (inputs == null || !inputs.isJsonArray()) continue;
			for (JsonElement inputElement : inputs.getAsJsonArray()) {
				if (!inputElement.isJsonObject()) continue;
				JsonObject input = inputElement.getAsJsonObject();
				if (!input.has("location") || !input.get("location").isJsonPrimitive()) continue;
				ResourceLocation location = ResourceLocation.parse(input.get("location").getAsString());
				String path = location.getPath();
				if (!path.startsWith("textures/")) path = "textures/" + path;
				if (!path.endsWith(".png")) path += ".png";
				ResourceLocation previousOwner = assetOwners.putIfAbsent(path, location);
				if (previousOwner != null) {
					if (!previousOwner.equals(location)) {
						throw new IOException(
							"post-effect texture input path " + path + " is ambiguous between "
								+ previousOwner + " and " + location
						);
					}
					if (assets.containsKey(path)) continue;
				}
				if (assets.size() >= MAX_ASSET_FILES) {
					throw new IOException("post-effect texture asset count exceeds " + MAX_ASSET_FILES);
				}
				Resource resource = manager.getResource(
					ResourceLocation.fromNamespaceAndPath(location.getNamespace(), path)
				).orElseThrow(() -> new IOException("post-effect texture input is missing: " + location));
				byte[] contents = readBoundedResource(resource, MAX_ASSET_FILE_BYTES, "post-effect texture input");
				long currentBytes = assets.values().stream().mapToLong(asset -> asset.contents().length).sum();
				if (Math.addExact(currentBytes, contents.length) > MAX_ASSET_TOTAL_BYTES) {
					throw new IOException("post-effect texture asset payload exceeds " + MAX_ASSET_TOTAL_BYTES + " bytes");
				}
				assets.put(path, new VulkanicGalBridge.ShaderPackAssetFileRecord(
					path, contents));
			}
		}
		return List.copyOf(assets.values());
	}

	/**
	 * Adds the active resource-pack post-effect to an existing Iris snapshot.
	 * Iris-owned files win on path collisions; resource-pack files only fill
	 * missing stages. This keeps pack precedence deterministic while allowing a
	 * vanilla/resource-pack post graph to use bundled or copied fallback stages.
	 */
	static SourceGeneration withActiveVanillaPostEffectResources(SourceGeneration source) throws IOException {
		Optional<String> identity = activeVanillaPostEffectId();
		if (identity.isEmpty()) {
			return source;
		}
		ResourceManager manager = net.minecraft.client.Minecraft.getInstance().getResourceManager();
		ResourceLocation effect = ResourceLocation.parse(identity.get());
		String effectPath = effect.getPath() + ".json";
		Resource effectResource = manager.getResource(
			ResourceLocation.fromNamespaceAndPath(effect.getNamespace(), "post_effect/" + effectPath)
		).orElseThrow(() -> new IOException("active post-effect definition is missing: " + identity.get()));
		String assetPath = "post_effect/" + effectPath;
		Map<String, VulkanicGalBridge.ShaderPackAssetFileRecord> assets = new TreeMap<>();
		for (var asset : source.assets()) {
			assets.put(asset.path(), asset);
		}
		if (!assets.containsKey(assetPath)) {
			if (assets.size() >= MAX_ASSET_FILES) {
				throw new IOException("merged post-effect shader asset count exceeds " + MAX_ASSET_FILES);
			}
			assets.put(assetPath, new VulkanicGalBridge.ShaderPackAssetFileRecord(
				assetPath,
				readBoundedResource(effectResource, MAX_ASSET_FILE_BYTES, "post-effect definition")
			));
		}
		for (var texture : collectPostEffectTextureAssets(manager, effect, readBoundedResource(effectResource, MAX_ASSET_FILE_BYTES, "post-effect definition"))) {
			assets.putIfAbsent(texture.path(), texture);
		}
		Map<String, VulkanicGalBridge.ShaderPackSourceFileRecord> files = new TreeMap<>();
		for (var file : source.files()) {
			files.put(file.path(), file);
		}
		for (var entry : manager.listResources("shaders", location -> {
			String path = location.getPath();
			return location.getNamespace().equals(effect.getNamespace())
				&& (path.endsWith(".vsh") || path.endsWith(".fsh") || path.endsWith(".glsl"));
		}).entrySet()) {
			String path = normalizeShaderSourcePath(entry.getKey().getPath());
			if (files.containsKey(path)) {
				continue;
			}
			if (files.size() >= MAX_FILES) {
				throw new IOException("merged post-effect shader source count exceeds " + MAX_FILES);
			}
			files.put(path, new VulkanicGalBridge.ShaderPackSourceFileRecord(
				path,
				readBoundedResource(entry.getValue(), MAX_FILE_BYTES, "post-effect shader source")
			));
		}
		long sourceBytes = files.values().stream().mapToLong(file -> file.contentsUtf8().length).sum();
		long assetBytes = assets.values().stream().mapToLong(asset -> asset.contents().length).sum();
		if (sourceBytes > MAX_TOTAL_BYTES || assetBytes > MAX_ASSET_TOTAL_BYTES) {
			throw new IOException("merged resource-pack post-effect snapshot exceeds its bounded payload");
		}
		return new SourceGeneration(
			source.packName(),
			source.generation(),
			List.copyOf(files.values()),
			sourceBytes,
			List.copyOf(assets.values()),
			assetBytes
		);
	}

	/**
	 * ResourceManager paths include the lookup root ("shaders/") while the
	 * Rust semantic source snapshot is rooted at that directory. Keep the
	 * copied path identical to filesystem shader-pack collection so post-effect
	 * source resolution cannot reject a valid resource-pack shader merely due to
	 * Java's namespace prefix.
	 */
	static String normalizeShaderSourcePath(String path) {
		if (path == null || path.isBlank()) {
			throw new IllegalArgumentException("shader source path must be non-empty");
		}
		return path.startsWith("shaders/") ? path.substring("shaders/".length()) : path;
	}

	private static byte[] readBoundedResource(Resource resource, int maximumBytes, String kind) throws IOException {
		try (var input = resource.open()) {
			byte[] bytes = input.readNBytes(Math.addExact(maximumBytes, 1));
			if (bytes.length > maximumBytes) {
				throw new IOException(kind + " exceeds " + maximumBytes + " bytes");
			}
			return bytes;
		}
	}

	/** Reads only the copied on-disk preference used to reject unported packs. */
	private static boolean wholeFrameShaderConfigEnabled() throws IOException {
		Path config = net.minecraft.client.Minecraft.getInstance().gameDirectory.toPath()
			.resolve("config").resolve("iris.properties");
		if (!Files.isRegularFile(config)) {
			return true;
		}
		Properties values = new Properties();
		try (var input = Files.newInputStream(config)) {
			values.load(input);
		}
		return !"false".equals(values.getProperty("enableShaders"));
	}

	private static Optional<String> configuredPackNameFromDisk() throws IOException {
		Path config = net.minecraft.client.Minecraft.getInstance().gameDirectory.toPath()
			.resolve("config").resolve("iris.properties");
		return configuredPackNameFromProperties(config);
	}

	/** Stable source-environment defaults used before any pack-specific Iris option graph exists. */
	static Map<String, String> wholeFrameEnvironmentDefines() {
		// DH execution is intentionally unavailable on the vanilla Rust Vulkan
		// route.  Do not let a copied shader source opt into its DH branches just
		// because the mod happens to be installed; that would create a hidden
		// dependency on an unadmitted renderer family.
		return wholeFrameEnvironmentDefines(false);
	}

	/**
	 * Returns the explicit source environment for one Rust-owned world route.
	 *
	 * <p>{@code DISTANT_HORIZONS} selects shader-pack control flow which requires
	 * a Rust-owned DH writer and depth/color semantics in the same submission;
	 * it is not merely a declaration that the binary contains DH support.
	 * Ordinary fixtures that exclude that writer must omit the define.</p>
	 */
	static Map<String, String> wholeFrameEnvironmentDefines(boolean distantHorizonsOwned) {
		TreeMap<String, String> defines = new TreeMap<>();
		defines.put("IS_IRIS", "1");
		defines.put("IRIS_VERSION", "12000");
		if (distantHorizonsOwned) {
			// The selected frame owns the corresponding DH source writer, so packs
			// such as Complementary may safely enable their DH material and
			// fullscreen branches without consulting Iris state.
			defines.put("DISTANT_HORIZONS", "1");
		}
		// Minecraft's shader macro is a stable semantic engine value. Keep it in
		// the Rust-owned source snapshot rather than reading Iris' macro table or
		// any live renderer state.
		defines.put("MC_VERSION", "12105");
		// Complementary's selected MATTMC profile uses this scalar default. It is
		// transported as source configuration, never read from Iris GPU state.
		defines.put("SHADOW_QUALITY", "2");
		defines.put("ENTITY_SHADOWS_DEFINE", "-1");
		defines.put("PLAYER_SHADOW", "1");
		defines.put("FXAA_DEFINE", "1");
		defines.put("DETAIL_QUALITY", "2");
		defines.put("CLOUD_QUALITY", "2");
		defines.put("WATER_REFLECT_QUALITY", "2");
		defines.put("BLOCK_REFLECT_QUALITY", "3");
		defines.put("LIGHTSHAFT_QUALI_DEFINE", "2");
		defines.put("SSAO_QUALI_DEFINE", "2");
		defines.put("COLORED_LIGHTING", "256");
		defines.put("RAIN_PUDDLES", "0");
		defines.put("ANISOTROPIC_FILTER", "0");
		defines.put("MC_RENDER_STAGE_SUN", "4");
		defines.put("MC_RENDER_STAGE_MOON", "5");
		defines.put("MC_RENDER_STAGE_TERRAIN_SOLID", "8");
		defines.put("MC_RENDER_STAGE_TERRAIN_CUTOUT_MIPPED", "9");
		defines.put("MC_RENDER_STAGE_TERRAIN_CUTOUT", "10");
		defines.put("MC_RENDER_STAGE_TERRAIN_TRANSLUCENT", "15");
		defines.put("MC_RENDER_STAGE_RAIN_SNOW", "19");
		defines.put("MC_RENDER_STAGE_CLOUDS", "20");
		defines.put("MC_RENDER_STAGE_ENTITIES", "23");
		defines.put("MC_RENDER_STAGE_HAND", "24");
		return defines;
	}

	static Map<String, String> readWholeFramePackOptions(Path shaderpacks, String packName) throws IOException {
		Path options = shaderpacks.resolve(packName + ".txt").normalize();
		if (!options.startsWith(shaderpacks) || !Files.isRegularFile(options)) {
			return Map.of();
		}
		if (Files.size(options) > MAX_FILE_BYTES) {
			throw new IOException("shader-pack option file exceeds " + MAX_FILE_BYTES + " bytes");
		}
		Properties values = new Properties();
		try (var input = Files.newInputStream(options)) {
			values.load(input);
		}
		TreeMap<String, String> result = new TreeMap<>();
		for (String name : values.stringPropertyNames()) {
			String value = values.getProperty(name);
			if (name.matches("[A-Za-z_][A-Za-z0-9_]*") && value != null && !value.isBlank()) {
				result.put(name, value.trim());
			}
		}
		return result;
	}

	static Optional<String> configuredPackNameFromProperties(Path config) throws IOException {
		if (!Files.isRegularFile(config)) {
			return Optional.empty();
		}
		Properties values = new Properties();
		try (var input = Files.newInputStream(config)) {
			values.load(input);
		}
		// Iris retains the last selected filename when shaders are switched off.
		// That preference is not an active source pack: a vanilla post effect
		// must collect only ResourceManager bytes, never merge the inactive pack.
		if ("false".equals(values.getProperty("enableShaders"))) {
			return Optional.empty();
		}
		String name = values.getProperty("shaderPack", "").trim();
		return name.isEmpty() || "(internal)".equals(name) ? Optional.empty() : Optional.of(name);
	}

	public static SourceGeneration disabled(long generation) {
		if (generation <= 0L) {
			throw new IllegalArgumentException("shader-pack source generation must be positive");
		}
		return new SourceGeneration("disabled", generation, List.of(), 0L, List.of(), 0L);
	}

	public static SourceGeneration collect(Path shaderRoot, String packName, long generation) throws IOException {
		Objects.requireNonNull(shaderRoot, "shaderRoot");
		Objects.requireNonNull(packName, "packName");
		if (generation <= 0L) {
			throw new IllegalArgumentException("shader-pack source generation must be positive");
		}
		if (packName.isBlank()) {
			throw new IllegalArgumentException("shader-pack source pack name is empty");
		}
		Path normalizedRoot = shaderRoot.toAbsolutePath().normalize();
		if (!Files.isDirectory(normalizedRoot)) {
			throw new IOException("shader-pack source root is not a directory: " + normalizedRoot);
		}

		List<Path> paths;
		try (Stream<Path> files = Files.walk(normalizedRoot)) {
			paths = files
				.filter(Files::isRegularFile)
				.filter(RustShaderPackSourceCollector::isSourceFile)
				.sorted(Comparator.comparing(path -> normalizedRoot.relativize(path).toString()))
				.toList();
		}
		if (paths.size() > MAX_FILES) {
			throw new IOException("shader-pack source file count exceeds " + MAX_FILES);
		}

		long totalBytes = 0L;
		List<VulkanicGalBridge.ShaderPackSourceFileRecord> files = new java.util.ArrayList<>(paths.size());
		for (Path path : paths) {
			Path relative = normalizedRoot.relativize(path).normalize();
			if (relative.isAbsolute() || relative.startsWith("..")) {
				throw new IOException("shader-pack source path escapes root: " + path);
			}
			long size = Files.size(path);
			if (size > MAX_FILE_BYTES) {
				throw new IOException("shader-pack source file exceeds " + MAX_FILE_BYTES + " bytes: " + relative);
			}
			totalBytes = Math.addExact(totalBytes, size);
			if (totalBytes > MAX_TOTAL_BYTES) {
				throw new IOException("shader-pack source payload exceeds " + MAX_TOTAL_BYTES + " bytes");
			}
			files.add(new VulkanicGalBridge.ShaderPackSourceFileRecord(
				relative.toString().replace('\\', '/'),
				readBoundedFile(path, MAX_FILE_BYTES, "shader-pack source file")
			));
		}
		return new SourceGeneration(packName, generation, List.copyOf(files), totalBytes, List.of(), 0L);
	}

	/**
	 * Collects source text and binary assets from the same shader-relative root.
	 * Asset paths retain that root-relative identity so source declarations can
	 * refer to them without leaking host paths, archives, or Iris resources.
	 */
	static SourceGeneration collectWithAssets(Path shaderRoot, String packName, long generation) throws IOException {
		SourceGeneration source = collect(shaderRoot, packName, generation);
		Path normalizedRoot = shaderRoot.toAbsolutePath().normalize();
		List<Path> paths;
		try (Stream<Path> files = Files.walk(normalizedRoot)) {
			paths = files
				.filter(Files::isRegularFile)
				.filter(RustShaderPackSourceCollector::isAssetFile)
				.sorted(Comparator.comparing(path -> normalizedRoot.relativize(path).toString()))
				.toList();
		}
		if (paths.size() > MAX_ASSET_FILES) {
			throw new IOException("shader-pack asset file count exceeds " + MAX_ASSET_FILES);
		}

		long totalBytes = 0L;
		List<VulkanicGalBridge.ShaderPackAssetFileRecord> assets = new java.util.ArrayList<>(paths.size());
		for (Path path : paths) {
			Path relative = normalizedRoot.relativize(path).normalize();
			if (relative.isAbsolute() || relative.startsWith("..")) {
				throw new IOException("shader-pack asset path escapes root: " + path);
			}
			long size = Files.size(path);
			if (size > MAX_ASSET_FILE_BYTES) {
				throw new IOException("shader-pack asset exceeds " + MAX_ASSET_FILE_BYTES + " bytes: " + relative);
			}
			totalBytes = Math.addExact(totalBytes, size);
			if (totalBytes > MAX_ASSET_TOTAL_BYTES) {
				throw new IOException("shader-pack asset payload exceeds " + MAX_ASSET_TOTAL_BYTES + " bytes");
			}
			assets.add(new VulkanicGalBridge.ShaderPackAssetFileRecord(
				relative.toString().replace('\\', '/'),
				readBoundedFile(path, MAX_ASSET_FILE_BYTES, "shader-pack asset file")
			));
		}
		return new SourceGeneration(
			source.packName(),
			source.generation(),
			source.files(),
			source.totalBytes(),
			List.copyOf(assets),
			totalBytes
		);
	}

	/** Reads one pack file with a second bound check to close size/read races. */
	private static byte[] readBoundedFile(Path path, int maximumBytes, String kind) throws IOException {
		try (var input = Files.newInputStream(path)) {
			byte[] contents = input.readNBytes(Math.addExact(maximumBytes, 1));
			if (contents.length > maximumBytes) {
				throw new IOException(kind + " exceeds " + maximumBytes + " bytes: " + path);
			}
			return contents;
		}
	}

	private static boolean isSourceFile(Path path) {
		String name = path.getFileName().toString().toLowerCase(java.util.Locale.ROOT);
		return name.endsWith(".vsh")
			|| name.endsWith(".fsh")
			|| name.endsWith(".gsh")
			|| name.endsWith(".csh")
			|| name.endsWith(".glsl")
			|| name.endsWith(".properties");
	}

	private static boolean isAssetFile(Path path) {
		String name = path.getFileName().toString().toLowerCase(java.util.Locale.ROOT);
		return name.endsWith(".png")
			|| name.endsWith(".jpg")
			|| name.endsWith(".jpeg")
			|| name.endsWith(".bmp")
			|| name.endsWith(".tga")
			|| name.endsWith(".dds")
			|| name.endsWith(".ktx")
			|| name.endsWith(".ktx2")
			|| name.endsWith(".exr")
			|| name.endsWith(".raw")
			|| name.endsWith(".bin")
			|| name.endsWith(".dat")
			|| name.endsWith(".mcmeta")
			|| name.endsWith(".json");
	}

	/**
	 * Copies only the resolved scalar option values that affect source
	 * preprocessing. The Iris option objects are never retained or passed over
	 * FFI; Rust receives one immutable, bounded properties payload alongside
	 * the selected pack sources.
	 */
	static boolean isShaderStageSelector(String name) {
		return switch (Objects.requireNonNull(name, "name")) {
			case "VERTEX_SHADER", "FRAGMENT_SHADER", "GEOMETRY_SHADER", "COMPUTE_SHADER" -> true;
			default -> false;
		};
	}

	static void putEnabledBooleanOption(Map<String, String> options, String name, boolean enabled) {
		Objects.requireNonNull(options, "options");
		Objects.requireNonNull(name, "name");
		if (enabled) {
			options.put(name, "1");
		}
	}

	/**
	 * Replaces raw shader files with the configured include graph's immutable
	 * source. This is a source/configuration snapshot only: it copies no Iris
	 * program, framebuffer, texture, callback, or native handle.
	 */
	static SourceGeneration withResolvedSourceSnapshot(SourceGeneration source, Map<String, byte[]> resolvedSources) throws IOException {
		Objects.requireNonNull(source, "source");
		Objects.requireNonNull(resolvedSources, "resolvedSources");
		long totalBytes = 0L;
		List<VulkanicGalBridge.ShaderPackSourceFileRecord> files = new java.util.ArrayList<>(source.files().size());
		for (VulkanicGalBridge.ShaderPackSourceFileRecord file : source.files()) {
			byte[] contents = resolvedSources.getOrDefault(file.path(), file.contentsUtf8());
			if (contents.length > MAX_FILE_BYTES) {
				throw new IOException("resolved shader-pack source file exceeds " + MAX_FILE_BYTES + " bytes: " + file.path());
			}
			totalBytes = Math.addExact(totalBytes, contents.length);
			if (totalBytes > MAX_TOTAL_BYTES) {
				throw new IOException("resolved shader-pack source payload exceeds " + MAX_TOTAL_BYTES + " bytes");
			}
			files.add(new VulkanicGalBridge.ShaderPackSourceFileRecord(file.path(), contents));
		}
		return new SourceGeneration(
			source.packName(), source.generation(), List.copyOf(files), totalBytes, source.assets(), source.assetTotalBytes()
		);
	}

	/**
	 * Copies immutable game semantics, not Iris's resolved material map. Rust
	 * owns the selected-pack parser and resolves these identities against its
	 * own source-derived block-property contract.
	 */
	static SourceGeneration withRuntimeBlockStateIdentitySnapshot(SourceGeneration source) throws IOException {
		Objects.requireNonNull(source, "source");
		StringBuilder properties = new StringBuilder();
		for (Block block : BuiltInRegistries.BLOCK) {
			String blockId = BuiltInRegistries.BLOCK.getKey(block).toString();
			for (BlockState state : block.getStateDefinition().getPossibleStates()) {
				int rawStateId = Block.getId(state);
				if (rawStateId < 0) {
					throw new IOException("Minecraft block state has no raw registry identity: " + state);
				}
				properties.append("state.").append(rawStateId).append('=').append(blockId);
				for (Property<?> property : state.getProperties().stream()
					.sorted(java.util.Comparator.comparing(Property::getName)).toList()) {
					properties.append('|').append(property.getName()).append('=')
						.append(propertyValueName(state, property));
				}
				properties.append('\n');
			}
		}
		byte[] contents = properties.toString().getBytes(java.nio.charset.StandardCharsets.UTF_8);
		if (contents.length > MAX_FILE_BYTES) {
			throw new IOException("shader-pack runtime block-state payload exceeds " + MAX_FILE_BYTES + " bytes");
		}
		return appendReservedProperties(source, RUNTIME_BLOCK_STATE_IDENTITIES_PATH, contents, "block-state identity");
	}

	private static <T extends Comparable<T>> String propertyValueName(BlockState state, Property<T> property) {
		return property.getName(state.getValue(property));
	}

	static SourceGeneration withRuntimeOptionSnapshot(SourceGeneration source, java.util.Map<String, String> options) throws IOException {
		return withRuntimeScalarSnapshot(source, options, RUNTIME_OPTIONS_PATH, "option");
	}

	static SourceGeneration withRuntimeConstantSnapshot(SourceGeneration source, java.util.Map<String, String> constants) throws IOException {
		return withRuntimeScalarSnapshot(source, constants, RUNTIME_CONSTANTS_PATH, "constant");
	}

	private static SourceGeneration withRuntimeScalarSnapshot(
		SourceGeneration source,
		java.util.Map<String, String> values,
		String reservedPath,
		String kind
	) throws IOException {
		Objects.requireNonNull(source, "source");
		Objects.requireNonNull(values, "values");
		if (values.size() > MAX_RUNTIME_PROPERTIES) {
			throw new IOException("shader-pack runtime " + kind + " entry count exceeds " + MAX_RUNTIME_PROPERTIES);
		}
		StringBuilder properties = new StringBuilder();
		for (var entry : new TreeMap<>(values).entrySet()) {
			validateOptionEntry(entry.getKey(), entry.getValue());
			properties.append(entry.getKey()).append('=').append(entry.getValue()).append('\n');
		}
		byte[] contents = properties.toString().getBytes(java.nio.charset.StandardCharsets.UTF_8);
		if (contents.length > MAX_FILE_BYTES) {
			throw new IOException("shader-pack runtime option payload exceeds " + MAX_FILE_BYTES + " bytes");
		}
		return appendReservedProperties(source, reservedPath, contents, kind);
	}

	static SourceGeneration withRuntimeEnvironmentSnapshot(SourceGeneration source, java.util.Map<String, String> defines) throws IOException {
		Objects.requireNonNull(source, "source");
		Objects.requireNonNull(defines, "defines");
		if (defines.size() > MAX_RUNTIME_PROPERTIES) {
			throw new IOException("shader-pack runtime environment entry count exceeds " + MAX_RUNTIME_PROPERTIES);
		}
		StringBuilder properties = new StringBuilder();
		for (var entry : new TreeMap<>(defines).entrySet()) {
			String value = entry.getValue() == null || entry.getValue().isEmpty() ? "1" : entry.getValue();
			validateOptionEntry(entry.getKey(), value);
			properties.append(entry.getKey()).append('=').append(value).append('\n');
		}
		byte[] contents = properties.toString().getBytes(java.nio.charset.StandardCharsets.UTF_8);
		if (contents.length > MAX_FILE_BYTES) {
			throw new IOException("shader-pack runtime environment payload exceeds " + MAX_FILE_BYTES + " bytes");
		}
		return appendReservedProperties(source, RUNTIME_ENVIRONMENT_PATH, contents, "environment");
	}

	private static SourceGeneration appendReservedProperties(
		SourceGeneration source,
		String reservedPath,
		byte[] contents,
		String kind
	) throws IOException {
		List<VulkanicGalBridge.ShaderPackSourceFileRecord> files = new java.util.ArrayList<>(source.files());
		if (files.stream().anyMatch(file -> reservedPath.equals(file.path()))) {
			throw new IOException("shader-pack source reserves " + reservedPath);
		}
		if (files.size() >= MAX_FILES) {
			throw new IOException("shader-pack source file count exceeds " + MAX_FILES + " after adding " + kind);
		}
		long totalBytes = Math.addExact(source.totalBytes(), contents.length);
		if (totalBytes > MAX_TOTAL_BYTES) {
			throw new IOException("shader-pack source and " + kind + " payload exceed " + MAX_TOTAL_BYTES + " bytes");
		}
		files.add(new VulkanicGalBridge.ShaderPackSourceFileRecord(reservedPath, contents));
		return new SourceGeneration(
			source.packName(),
			source.generation(),
			List.copyOf(files),
			totalBytes,
			source.assets(),
			source.assetTotalBytes()
		);
	}

	private static void validateOptionEntry(String name, String value) throws IOException {
		if (!name.matches("[A-Za-z_][A-Za-z0-9_]*")) {
			throw new IOException("shader-pack option name is not a preprocessor identifier: " + name);
		}
		if (value.isEmpty() || value.chars().anyMatch(Character::isWhitespace)
			|| value.indexOf('#') >= 0 || value.indexOf('\\') >= 0 || value.indexOf('=') >= 0) {
			throw new IOException("shader-pack option value is not one preprocessor token: " + name);
		}
	}

	public record SourceGeneration(
		String packName,
		long generation,
		List<VulkanicGalBridge.ShaderPackSourceFileRecord> files,
		long totalBytes,
		List<VulkanicGalBridge.ShaderPackAssetFileRecord> assets,
		long assetTotalBytes
	) {
		public SourceGeneration {
			Objects.requireNonNull(packName, "packName");
			files = List.copyOf(files);
			assets = List.copyOf(assets);
		}
	}
}
